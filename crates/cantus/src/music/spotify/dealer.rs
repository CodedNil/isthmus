use super::{
    CLUSTER_TOPIC, MusicResult, REQUEST_TIMEOUT,
    auth::AuthorizationExpired,
    client::decode_gzip,
    protocol::{
        connect::{Cluster, ClusterUpdate},
        playlist4_external::PlaylistModificationInfo,
    },
    session::{Endpoint, Session},
};
use base64::{Engine as _, prelude::BASE64_STANDARD};
use futures_util::{SinkExt as _, StreamExt as _};
use protobuf::{Message as _, MessageField};
use reqwest::Url;
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::{Value, json};
use std::{collections::HashMap, fmt::Display, io, str::FromStr, time::Duration};
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
    PlaylistChanged(Option<PlaylistModificationInfo>),
}

impl Dealer {
    pub(super) async fn start(session: Session) -> MusicResult<Self> {
        let socket = timeout(REQUEST_TIMEOUT, connect(&session)).await??;
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
    let (host, port) = session.resolve(Endpoint::Dealer).await;
    let token = session.auth_token().await?;
    let mut url = Url::parse(&format!("wss://{host}:{port}/"))?;
    url.query_pairs_mut().append_pair("access_token", &token);
    Ok(connect_async(url.as_str()).await?.0)
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
            match timeout(REQUEST_TIMEOUT, connect(&session)).await {
                Ok(Ok(connected)) => {
                    socket = connected;
                    break;
                }
                Ok(Err(error)) => {
                    if error.is::<AuthorizationExpired>() {
                        warn!("Spotify authorization expired while reconnecting");
                        return;
                    }
                    warn!(%error, "Could not reconnect Spotify dealer");
                }
                Err(error) => warn!(%error, "Spotify dealer reconnect timed out"),
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
                    let envelope = serde_json::from_slice(&message.into_data());
                    match envelope {
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
        let change = payloads.pop().and_then(|payload| {
            let bytes = decode_binary(payload, gzip).ok()?;
            PlaylistModificationInfo::parse_from_bytes(&bytes).ok()
        });
        return Ok(Some(Event::PlaylistChanged(change)));
    }
    if matches_topic(CLUSTER_TOPIC) {
        let payload = payloads
            .pop()
            .filter(|_| payloads.is_empty())
            .ok_or_else(|| io::Error::other("Spotify cluster event must have one payload"))?;
        let update: ClusterUpdate = match payload {
            Payload::Json(value) => serde_json::from_value(value)?,
            Payload::Base64(_) | Payload::Bytes(_) => ClusterUpdate::parse_from_bytes(&decode_binary(payload, gzip)?)?,
        };
        return Ok(update.cluster.into_option().map(|cluster| Event::Cluster(Box::new(cluster))));
    }
    Ok(None)
}

fn decode_binary(payload: Payload, gzip: bool) -> MusicResult<bytes::Bytes> {
    let bytes = match payload {
        Payload::Base64(value) => BASE64_STANDARD.decode(value)?,
        Payload::Bytes(value) => value,
        Payload::Json(_) => return Err(io::Error::other("Spotify Dealer payload was JSON").into()),
    };
    Ok(decode_gzip(bytes.into(), gzip)?)
}

pub(super) fn message<'de, D: Deserializer<'de>, T: Deserialize<'de>>(input: D) -> Result<MessageField<T>, D::Error> {
    Ok(MessageField::from_option(Option::deserialize(input)?))
}

pub(super) fn default_on_null<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    input: D,
) -> Result<T, D::Error> {
    Ok(Option::deserialize(input)?.unwrap_or_default())
}

pub(super) fn number<'de, D, T>(input: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default + FromStr<Err: Display>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number<T> {
        Number(T),
        String(String),
    }
    match Option::<Number<T>>::deserialize(input)? {
        Some(Number::Number(value)) => Ok(value),
        Some(Number::String(value)) => value.parse().map_err(D::Error::custom),
        None => Ok(T::default()),
    }
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
    fn dealer_routes_root_and_playlist_edits() {
        let payload = PlaylistModificationInfo {
            uri: Some(b"spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".to_vec()),
            new_revision: Some(vec![2; 24]),
            ..Default::default()
        };
        let encoded = BASE64_STANDARD.encode(payload.write_to_bytes().unwrap());
        for topic in ["hm://playlist/user/account/rootlist", "hm://playlist/v2/playlist/id"] {
            let event = decode_event(topic, &HashMap::new(), vec![Payload::Base64(encoded.clone())]).unwrap();
            assert!(matches!(event, Some(Event::PlaylistChanged(Some(change))) if change == payload));
        }
        assert!(decode_event("hm://playlist-permission/v1/playlist/id", &HashMap::new(), vec![]).unwrap().is_none());
    }

    #[test]
    fn dealer_decodes_binary_compressed_and_json_cluster_events() {
        let json = json!({
            "cluster": {
                "activeDeviceId": "phone",
                "player_state": { "timestamp": "1700000000000", "positionAsOfTimestamp": 42000 },
                "needFullPlayerState": false
            },
            "ackId": "id"
        });
        let raw = serde_json::from_value::<ClusterUpdate>(json.clone()).unwrap().write_to_bytes().unwrap();
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
                    && update.player_state.get_or_default().timestamp == 1_700_000_000_000
                    && update.player_state.get_or_default().position_as_of_timestamp == 42_000));
        }
    }
}
