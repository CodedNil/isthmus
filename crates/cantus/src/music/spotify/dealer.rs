use super::{
    CLUSTER_TOPIC, MusicResult, PlaylistId, REQUEST_TIMEOUT,
    auth::AuthorizationExpired,
    parse_id,
    proto::{
        connect::{Cluster, ClusterUpdate},
        playlist_edit::{OperationKind, PlaylistNotification},
    },
    session::{Endpoint, Session},
};
use base64::{Engine as _, prelude::BASE64_STANDARD};
use flate2::read::MultiGzDecoder;
use futures_util::{SinkExt as _, StreamExt as _};
use prost::Message as _;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, Read as _},
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
const RECONNECT_INTERVAL: Duration = Duration::from_secs(10);
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
    pub(super) async fn start(session: Session) -> MusicResult<Self> {
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
        let (host, port) = session.resolve(Endpoint::Dealer).await;
        let token = session.auth_token().await?;
        let mut url = Url::parse(&format!("wss://{host}:{port}/"))?;
        url.query_pairs_mut().append_pair("access_token", &token);
        Ok(connect_async(url.as_str()).await?.0)
    })
    .await?
}

async fn run(session: Session, mut socket: Socket, events: UnboundedSender<Event>) {
    loop {
        if let Err(error) = receive(&mut socket, &events, ANNOUNCEMENT_TIMEOUT).await {
            warn!(%error, "Spotify dealer disconnected");
        }
        if events.send(Event::Disconnected).is_err() {
            return;
        }
        loop {
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
            sleep(RECONNECT_INTERVAL).await;
            if events.is_closed() {
                return;
            }
        }
    }
}

async fn receive<Stream: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<Stream>,
    events: &UnboundedSender<Event>,
    announcement_timeout: Duration,
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
                                    announced |= matches!(event, Event::Connected(_));
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
#[serde(tag = "type", rename_all = "snake_case")]
enum Envelope {
    Message {
        uri: String,
        #[serde(default)]
        headers: HashMap<String, String>,
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
    Json(Value),
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
        let update: ClusterUpdate = match payload {
            Payload::Json(value) => serde_json::from_value(value)?,
            Payload::Base64(_) | Payload::Bytes(_) => ClusterUpdate::decode(decode_binary(payload, gzip)?)?,
        };
        return Ok(update.cluster.map(|cluster| Event::Cluster(Box::new(cluster))));
    }
    Ok(None)
}

fn decode_binary(payload: Payload, gzip: bool) -> MusicResult<bytes::Bytes> {
    let bytes = match payload {
        Payload::Base64(value) => BASE64_STANDARD.decode(value)?,
        Payload::Bytes(value) => value,
        Payload::Json(_) => return Err(io::Error::other("Spotify Dealer payload was JSON").into()),
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
        let error = receive(&mut socket, &events, Duration::from_millis(30)).await.unwrap_err();
        assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), io::ErrorKind::TimedOut);
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
    fn dealer_decodes_binary_compressed_and_json_cluster_events() {
        let json = json!({
            "cluster": {
                "activeDeviceId": "phone",
                "player_state": {
                    "timestamp": "1700000000000",
                    "positionAsOfTimestamp": 42000,
                    "context_uri": "spotify:playlist:example",
                    "playbackSpeed": "1.25",
                    "nextTracks": null,
                    "options": { "shufflingContext": true }
                },
                "needFullPlayerState": false
            },
            "ackId": "id"
        });
        let raw = serde_json::from_value::<ClusterUpdate>(json.clone()).unwrap().encode_to_vec();
        let mut gzip = super::super::client::gzip_encoder();
        gzip.write_all(&raw).unwrap();
        let gzip = BASE64_STANDARD.encode(gzip.finish().unwrap());
        for (headers, payload) in [
            (HashMap::new(), Payload::Bytes(raw)),
            (HashMap::from([("transfer-encoding".into(), "gzip".into())]), Payload::Base64(gzip)),
            (HashMap::new(), Payload::Json(json)),
        ] {
            let event = decode_event(CLUSTER_TOPIC, &headers, vec![payload]).unwrap().unwrap();
            assert!(matches!(event, Event::Cluster(update)
                    if update.active_device_id == "phone"
                    && update.player_state.as_ref().is_some_and(|player|
                        player.timestamp == 1_700_000_000_000
                            && player.position_as_of_timestamp == 42_000
                            && player.context_uri == "spotify:playlist:example"
                            && (player.playback_speed - 1.25).abs() < f64::EPSILON
                            && player.next_tracks.is_empty()
                            && player.options.is_none())));
        }
    }
}
