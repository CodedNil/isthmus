//! One local Shelly RPC stream, shared by all Lares browser connections.
use super::{Snapshot, Track};
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as BrowserMessage, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::watch,
    time::{interval, sleep, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const SENSOR: &str = "ws://192.168.0.133/rpc";
const STALE: Duration = Duration::from_secs(5);

struct Hub {
    latest: watch::Sender<Snapshot>,
}
impl Hub {
    fn new() -> Arc<Self> {
        let (latest, _) = watch::channel(Snapshot::empty(false));
        let hub = Arc::new(Self { latest });
        tokio::spawn(Arc::clone(&hub).run());
        hub
    }

    async fn run(self: Arc<Self>) {
        loop {
            let _ = self.stream().await;
            self.latest.send_replace(Snapshot::empty(false));
            sleep(Duration::from_secs(3)).await;
        }
    }

    async fn stream(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (mut sensor, _) = timeout(Duration::from_secs(4), connect_async(SENSOR)).await??;
        self.latest.send_replace(Snapshot::empty(true));
        let mut renew = interval(Duration::from_secs(40));
        let mut watchdog = interval(Duration::from_secs(1));
        let mut received = Instant::now();
        loop {
            tokio::select! {
                _ = renew.tick() => {
                    // LiveTrack is a temporary subscription, not sensor reconfiguration.
                    sensor.send(Message::Text(json!({"id":1,"src":"lares","method":"Presence.LiveTrack"}).to_string().into())).await?;
                }
                _ = watchdog.tick() => {
                    if received.elapsed() >= Duration::from_secs(15) {
                        return Err("Shelly tracking stream timed out".into());
                    }
                    if received.elapsed() >= STALE && !self.latest.borrow().stale {
                        self.latest.send_replace(Snapshot::empty(true));
                    }
                }
                message = sensor.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(tracks) = parse_tracks(text.as_ref()) {
                            received = Instant::now();
                            self.latest.send_replace(Snapshot { connected: true, stale: false, tracks });
                        }
                    }
                    Some(Ok(Message::Ping(bytes))) => sensor.send(Message::Pong(bytes)).await?,
                    Some(Ok(Message::Close(_))) | None => return Err("Shelly connection closed".into()),
                    Some(Err(error)) => return Err(error.into()),
                    _ => {}
                }
            }
        }
    }
}

fn parse_tracks(text: &str) -> Option<Vec<Track>> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("method")?.as_str()? != "NotifyEvent" {
        return None;
    }
    let event = value.get("params")?.get("events")?.as_array()?.iter().rev().find(|event| {
        event.get("component").and_then(Value::as_str) == Some("presence")
            && event.get("event").and_then(Value::as_str) == Some("track")
    })?;
    let objects = event.get("object")?.as_array()?;
    Some(
        objects
            .iter()
            .take(10)
            .filter_map(|value| serde_json::from_value::<Track>(value.clone()).ok())
            .filter(|track| {
                track.x.is_finite()
                    && track.y.is_finite()
                    && track.z.is_finite()
                    && track.x.abs() <= 20.0
                    && (0.0..=20.0).contains(&track.y)
            })
            .collect(),
    )
}

pub fn router() -> Router {
    Router::new().route("/api/presence", get(upgrade)).with_state(Hub::new())
}

async fn upgrade(
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    socket: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    if let Some(origin) = headers.get("origin") {
        let origin = origin.to_str().map_err(|_error| StatusCode::FORBIDDEN)?;
        let host = headers.get("host").and_then(|value| value.to_str().ok()).ok_or(StatusCode::FORBIDDEN)?;
        if origin != format!("http://{host}") && origin != format!("https://{host}") {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(socket.on_upgrade(move |socket| forward(hub, socket)))
}

async fn forward(hub: Arc<Hub>, socket: WebSocket) {
    let mut updates = hub.latest.subscribe();
    let (mut output, mut input) = socket.split();
    loop {
        let message = serde_json::to_string(&*updates.borrow_and_update()).expect("finite presence values");
        if output.send(BrowserMessage::Text(message.into())).await.is_err() {
            break;
        }
        tokio::select! {
            result = updates.changed() => { if result.is_err() { break; } }
            message = input.next() => match message {
                Some(Ok(BrowserMessage::Ping(bytes))) => {
                    if output.send(BrowserMessage::Pong(bytes)).await.is_err() { break; }
                }
                Some(Ok(BrowserMessage::Close(_)) | Err(_)) | None => break,
                _ => {}
            }
        }
    }
    let _ = output.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_events_clear_absent_people_and_ignore_other_notifications() {
        let message = r#"{"method":"NotifyEvent","params":{"events":[{"component":"presence","event":"track","object":[{"id":3,"x":0.27,"y":5.04,"z":-0.09,"minz":0.70,"maxz":0.82}]}]}}"#;
        let tracks = parse_tracks(message).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, 3);
        assert_eq!(tracks[0].y, 5.04);
        assert!(
            parse_tracks(
                r#"{"method":"NotifyEvent","params":{"events":[{"component":"presence","event":"track","object":[]}]}}"#
            )
            .unwrap()
            .is_empty()
        );
        assert!(parse_tracks(r#"{"method":"NotifyStatus","params":{}}"#).is_none());
        assert!(parse_tracks("truncated").is_none());
    }
}
