use super::{
    CLUSTER_TOPIC, MusicResult, PlaylistId, REQUEST_TIMEOUT,
    auth::AuthorizationExpired,
    parse_id,
    proto::{
        connect::{Cluster, ClusterUpdate},
        playlist_edit::{OperationKind, PlaylistNotification},
    },
    session::Session,
};
use base64::{Engine as _, prelude::BASE64_STANDARD};
use flate2::read::MultiGzDecoder;
use futures_util::{SinkExt as _, StreamExt as _};
use prost::Message as _;
use reqwest::Url;
use serde::{Deserialize, de::IgnoredAny};
use serde_json::json;
use std::{
    collections::HashMap,
    io::{self, Read as _},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval_at, sleep, timeout},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use tracing::warn;

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(3);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(10);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(120);
const ANNOUNCEMENT_TIMEOUT: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) struct Dealer {
    messages: UnboundedReceiver<Event>,
    task: JoinHandle<()>,
}

pub(super) enum Event {
    Connected(String),
    Disconnected,
    Cluster(Box<Cluster>),
    RootlistChanged(Vec<u8>),
    PlaylistChanged { id: PlaylistId, revision: Vec<u8>, membership_only: bool },
}

impl Dealer {
    pub(super) async fn start(session: Arc<Session>) -> MusicResult<Self> {
        let socket = connect(&session).await?;
        let (send, messages) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(session, socket, send));
        Ok(Self { messages, task })
    }

    pub(super) async fn next(&mut self) -> MusicResult<Event> {
        self.messages.recv().await.ok_or_else(|| io::Error::other("Spotify dealer stream closed").into())
    }
}

impl Drop for Dealer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn connect(session: &Session) -> MusicResult<Socket> {
    timeout(REQUEST_TIMEOUT, async {
        let endpoint = session.dealer_url().await?;
        let token = session.auth_token().await?;
        let mut url = Url::parse(&endpoint)?;
        url.query_pairs_mut().append_pair("access_token", &token);
        Ok(connect_async(url.as_str()).await?.0)
    })
    .await?
}

async fn run(session: Arc<Session>, mut socket: Socket, events: UnboundedSender<Event>) {
    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    loop {
        if let Err(error) = receive(&mut socket, &events, ANNOUNCEMENT_TIMEOUT, &mut reconnect_delay).await {
            warn!(%error, "Spotify dealer disconnected");
        }
        if events.send(Event::Disconnected).is_err() {
            return;
        }
        loop {
            let jitter = f64::from(getrandom::u32().unwrap_or(u32::MAX)) / f64::from(u32::MAX);
            let delay = reconnect_delay.mul_f64(0.5 + 0.5 * jitter);
            reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
            tokio::select! {
                () = events.closed() => return,
                () = sleep(delay) => {}
            }
            match connect(&session).await {
                Ok(connected) => {
                    socket = connected;
                    break;
                }
                Err(error) if error.is::<AuthorizationExpired>() => {
                    warn!("Spotify authorization expired while reconnecting");
                    return;
                }
                Err(error) => warn!(%error, "Could not reconnect Spotify dealer"),
            }
        }
    }
}

async fn receive<Stream: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<Stream>,
    events: &UnboundedSender<Event>,
    announcement_timeout: Duration,
    reconnect_delay: &mut Duration,
) -> MusicResult<()> {
    let mut interval = interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let ping_timeout = sleep(PING_TIMEOUT);
    tokio::pin!(ping_timeout);
    let announcement_deadline = sleep(announcement_timeout);
    tokio::pin!(announcement_deadline);
    let mut announced = false;
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            () = events.closed() => return Ok(()),
            () = &mut announcement_deadline, if !announced => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Spotify dealer did not announce a connection").into());
            }
            _ = interval.tick() => {
                timeout(PING_TIMEOUT, socket.send(Message::Text(r#"{"type":"ping"}"#.into()))).await??;
                ping_timeout.as_mut().reset(Instant::now() + PING_TIMEOUT);
                awaiting_pong = true;
            }
            () = &mut ping_timeout, if awaiting_pong => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Spotify dealer did not answer ping").into());
            }
            message = socket.next() => match message.transpose()? {
                Some(message @ (Message::Text(_) | Message::Binary(_))) => {
                    match serde_json::from_slice(&message.into_data()) {
                        Ok(Envelope::Message { uri, headers, payloads }) => {
                            match decode_event(&uri, &headers, payloads) {
                                Ok(Some(event)) => {
                                    if matches!(event, Event::Connected(_)) {
                                        announced = true;
                                        *reconnect_delay = INITIAL_RECONNECT_DELAY;
                                    }
                                    if announced {
                                        events.send(event)?;
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => warn!(%error, "Invalid Spotify dealer event"),
                            }
                        }
                        Ok(Envelope::Request { key }) => {
                            let reply = json!({"type": "reply", "key": key, "payload": {"success": false}});
                            timeout(PING_TIMEOUT, socket.send(Message::Text(reply.to_string().into()))).await??;
                        }
                        Ok(Envelope::Pong) => awaiting_pong = false,
                        Ok(Envelope::Unknown) => {}
                        Err(error) => warn!(%error, "Invalid Spotify dealer message"),
                    }
                }
                Some(Message::Ping(_)) => timeout(PING_TIMEOUT, socket.flush()).await??,
                Some(Message::Close(_)) | None => return Ok(()),
                Some(_) => {}
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Envelope {
    Message {
        uri: String,
        // Absent headers carry no transfer encoding.
        #[serde(default)]
        headers: HashMap<String, String>,
        // Connection announcements carry headers and omit payloads.
        #[serde(default)]
        payloads: Vec<Payload>,
    },
    Request {
        key: String,
    },
    Pong,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Payload {
    Base64(String),
    Bytes(Vec<u8>),
    Other(IgnoredAny),
}

fn decode_event(
    uri: &str,
    headers: &HashMap<String, String>,
    mut payloads: Vec<Payload>,
) -> MusicResult<Option<Event>> {
    let topic = uri.split_once('?').map_or(uri, |(topic, _)| topic);
    let matches_topic =
        |prefix| topic == prefix || topic.strip_prefix(prefix).is_some_and(|tail| tail.starts_with('/'));
    if matches_topic("hm://pusher/v1/connections") {
        let id = headers
            .iter()
            .find(|(key, value)| key.eq_ignore_ascii_case("Spotify-Connection-Id") && !value.is_empty())
            .map(|(_, value)| value.clone())
            .ok_or_else(|| io::Error::other("Spotify dealer connection event had no connection ID"))?;
        return Ok(Some(Event::Connected(id)));
    }
    let gzip = headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("Transfer-Encoding") && value.eq_ignore_ascii_case("gzip"));
    if matches_topic("hm://playlist/v2") || matches_topic("hm://playlist/user") {
        let notice = payloads
            .pop()
            .and_then(|payload| decode_binary(payload, gzip).ok())
            .and_then(|bytes| PlaylistNotification::decode(bytes).ok());
        let Some(notice) = notice else { return Ok(Some(Event::RootlistChanged(Vec::new()))) };
        let uri = str::from_utf8(&notice.uri_or_revision).unwrap_or_default();
        let event = if uri.starts_with("spotify:user:") && uri.ends_with(":rootlist") {
            Event::RootlistChanged(notice.revision_or_parent)
        } else if let Some(id) = uri.strip_prefix("spotify:playlist:").and_then(parse_id) {
            let membership_only = !notice.ops.is_empty()
                && notice.ops.iter().all(|op| matches!(op.kind(), OperationKind::Add | OperationKind::Remove));
            Event::PlaylistChanged { id, revision: notice.revision_or_parent, membership_only }
        } else if matches_topic("hm://playlist/user") || topic.ends_with("/rootlist") {
            Event::RootlistChanged(notice.uri_or_revision)
        } else {
            Event::RootlistChanged(Vec::new())
        };
        return Ok(Some(event));
    }
    if matches_topic(CLUSTER_TOPIC) {
        let payload = payloads
            .pop()
            .filter(|_| payloads.is_empty())
            .ok_or_else(|| io::Error::other("Spotify cluster event must have one payload"))?;
        let update = ClusterUpdate::decode(decode_binary(payload, gzip)?)?;
        return Ok(update.cluster.map(|cluster| Event::Cluster(Box::new(cluster))));
    }
    Ok(None)
}

fn decode_binary(payload: Payload, gzip: bool) -> MusicResult<bytes::Bytes> {
    let bytes = match payload {
        Payload::Base64(value) => BASE64_STANDARD.decode(value)?,
        Payload::Bytes(value) => value,
        Payload::Other(IgnoredAny) => return Err(io::Error::other("Spotify Dealer payload was not binary").into()),
    };
    if !gzip || bytes.is_empty() {
        return Ok(bytes.into());
    }
    let mut decoded = Vec::new();
    MultiGzDecoder::new(bytes.as_slice()).read_to_end(&mut decoded)?;
    Ok(decoded.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tokio::io::duplex;
    use tokio_tungstenite::tungstenite::protocol::Role;

    #[tokio::test]
    async fn dealer_requires_an_announcement_even_when_socket_answers() {
        let (client, server) = duplex(1024);
        let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        let mut socket = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        server.send(Message::Text(r#"{"type":"pong"}"#.into())).await.unwrap();
        let (events, _receiver) = mpsc::unbounded_channel();
        let mut reconnect_delay = MAX_RECONNECT_DELAY;
        let error = receive(&mut socket, &events, Duration::from_millis(30), &mut reconnect_delay).await.unwrap_err();
        assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), io::ErrorKind::TimedOut);
        assert_eq!(reconnect_delay, MAX_RECONNECT_DELAY);
    }

    #[tokio::test]
    async fn dealer_resets_reconnect_delay_after_announcement() {
        let (client, server) = duplex(1024);
        let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        let mut socket = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        let announcement = json!({
            "type": "message",
            "uri": "hm://pusher/v1/connections",
            "headers": {"Spotify-Connection-Id": "test"}
        });
        server.send(Message::Text(announcement.to_string().into())).await.unwrap();
        server.close(None).await.unwrap();
        let (events, mut receiver) = mpsc::unbounded_channel();
        let mut reconnect_delay = MAX_RECONNECT_DELAY;
        receive(&mut socket, &events, ANNOUNCEMENT_TIMEOUT, &mut reconnect_delay).await.unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Event::Connected(id)) if id == "test"));
        assert_eq!(reconnect_delay, INITIAL_RECONNECT_DELAY);
    }

    #[test]
    fn dealer_routes_playlist_and_root_notices() {
        let id = "37i9dQZF1DXcBWIGoYBM5M";
        let revision = vec![0xff; 24];
        let encode = |uri_or_revision, revision_or_parent| {
            PlaylistNotification { uri_or_revision, revision_or_parent, ..Default::default() }.encode_to_vec()
        };

        let mut playlist = encode(format!("spotify:playlist:{id}").into_bytes(), revision.clone());
        playlist.extend([34, 4, 8, 2, 32, 1]); // An unsupported operation must still trigger a playlist read.
        let event = decode_event("hm://playlist/v2", &HashMap::new(), vec![Payload::Bytes(playlist)]).unwrap();
        assert!(matches!(event, Some(Event::PlaylistChanged { id: target, revision: rev, membership_only: true })
            if target.as_str() == id && rev == revision));

        let root = encode(revision.clone(), vec![1; 24]);
        for topic in
            ["hm://playlist/user", "hm://playlist/user/account/rootlist", "hm://playlist/v2/user/account/rootlist"]
        {
            let event = decode_event(topic, &HashMap::new(), vec![Payload::Bytes(root.clone())]).unwrap();
            assert!(matches!(event, Some(Event::RootlistChanged(rev)) if rev == revision));
        }
        let legacy = encode(b"spotify:user:account:rootlist".to_vec(), revision.clone());
        let event = decode_event("hm://playlist/v2", &HashMap::new(), vec![Payload::Bytes(legacy)]).unwrap();
        assert!(matches!(event, Some(Event::RootlistChanged(rev)) if rev == revision));
    }

    #[test]
    fn dealer_decodes_uncompressed_and_compressed_protobuf_cluster_events() {
        use super::super::proto::connect::{PlayerState, ProvidedTrack};

        let raw = ClusterUpdate {
            cluster: Some(Cluster {
                active_device_id: "phone".into(),
                server_timestamp_ms: 1_700_000_000_123,
                player_state: Some(PlayerState {
                    timestamp: 1_700_000_000_000,
                    position_as_of_timestamp: 42_000,
                    duration: 180_000,
                    context_uri: "spotify:playlist:example".into(),
                    playback_speed: 1.25,
                    is_playing: true,
                    next_tracks: vec![ProvidedTrack {
                        uri: "spotify:track:example".into(),
                        uid: "queue-occurrence".into(),
                        metadata: HashMap::from([("title".into(), "Example track".into())]),
                    }],
                    ..Default::default()
                }),
            }),
        }
        .encode_to_vec();
        let mut gzip = super::super::client::gzip_encoder();
        gzip.write_all(&raw).unwrap();
        let gzip = BASE64_STANDARD.encode(gzip.finish().unwrap());
        for envelope in [
            json!({"type": "message", "uri": CLUSTER_TOPIC, "payloads": [raw]}),
            json!({"type": "message", "uri": CLUSTER_TOPIC, "headers": {"transfer-encoding": "gzip"}, "payloads": [gzip]}),
        ] {
            let Envelope::Message { uri, headers, payloads } = serde_json::from_value(envelope).unwrap() else {
                panic!("expected a Dealer message");
            };
            let event = decode_event(&uri, &headers, payloads).unwrap().unwrap();
            assert!(matches!(event, Event::Cluster(update)
                    if update.active_device_id == "phone"
                    && update.server_timestamp_ms == 1_700_000_000_123
                    && update.player_state.as_ref().is_some_and(|player|
                        player.timestamp == 1_700_000_000_000
                            && player.position_as_of_timestamp == 42_000
                            && player.duration == 180_000
                            && player.context_uri == "spotify:playlist:example"
                            && (player.playback_speed - 1.25).abs() < f64::EPSILON
                            && player.is_playing
                            && !player.is_paused
                            && !player.is_buffering
                            && player.prev_tracks.is_empty()
                            && player.next_tracks.first().is_some_and(|track|
                                track.uri == "spotify:track:example"
                                    && track.uid == "queue-occurrence"
                                    && track.metadata.get("title").is_some_and(|title| title == "Example track")))));
        }
    }
}
