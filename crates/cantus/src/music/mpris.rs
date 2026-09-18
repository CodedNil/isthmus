use super::{LocalTrack, MusicResult, PlaybackCommand, Timeline, Track, TrackRuntime};
use crate::{
    app::{AppUpdater, send_update},
    platform::spawn_task,
};
use std::{collections::HashMap, error::Error, time::Duration};
use tokio::time::{interval, timeout};
use tracing::warn;
use web_time::Instant;
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PATH: &str = "/org/mpris/MediaPlayer2";
type Properties = HashMap<String, OwnedValue>;

fn take<T: TryFrom<OwnedValue>>(values: &mut Properties, key: &str) -> Option<T> {
    values.remove(key).and_then(|value| T::try_from(value).ok())
}

pub fn start(updater: AppUpdater) {
    spawn_task(async move {
        if let Err(error) = monitor(updater).await {
            warn!(%error, "MPRIS monitor stopped");
        }
    });
}

pub fn command(source: String, track: String, command: PlaybackCommand) {
    spawn_task(async move {
        let result = timeout(Duration::from_secs(2), async {
            let connection = Connection::session().await?;
            let player = Proxy::new(&connection, source.as_str(), PATH, PLAYER).await?;
            match command {
                PlaybackCommand::Seek(ms) => {
                    let track = OwnedObjectPath::try_from(track)?;
                    player.call::<_, _, ()>("SetPosition", &(track, i64::from(ms) * 1000)).await?;
                }
                PlaybackCommand::SetPlaying(playing) => {
                    player.call::<_, _, ()>(if playing { "Play" } else { "Pause" }, &()).await?;
                }
                _ => {}
            }
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            warn!(?result, "MPRIS command failed");
        }
    });
}

async fn monitor(updater: AppUpdater) -> MusicResult<()> {
    let connection = Connection::session().await?;
    let bus = zbus::fdo::DBusProxy::new(&connection).await?;
    let mut tick = interval(Duration::from_millis(500));
    let mut selected = String::new();
    loop {
        tick.tick().await;
        let mut active = None;
        let mut names = bus.list_names().await?;
        names.sort();
        for name in names.iter().filter(|name| name.as_str().starts_with("org.mpris.MediaPlayer2.")) {
            let snapshot = read_player(&connection, name.as_str());
            let Ok(Ok(Some(local))) = timeout(Duration::from_millis(250), snapshot).await else { continue };
            let priority = (local.playing, local.source == selected);
            if active.as_ref().is_none_or(|old: &LocalTrack| priority > (old.playing, old.source == selected)) {
                active = Some(local);
            }
        }
        selected = active.as_ref().map_or_else(String::new, |local| local.source.clone());
        if !send_update(&updater, move |app| {
            if let (Some(local), Some(previous)) = (&mut active, &app.music.local)
                && local.source == previous.source
                && local.track.uri == previous.track.uri
                && local.mpris_id == previous.mpris_id
            {
                local.track.runtime = previous.track.runtime.clone();
                local.track.interaction_id = previous.track.interaction_id;
                local.timeline.queue_start_ms = previous.timeline.queue_start_ms;
                local.timeline.movement = previous.timeline.movement;
                reconcile_position(local, previous);
            }
            app.music.local = active;
        }) {
            return Ok(());
        }
    }
}

fn reconcile_position(local: &mut LocalTrack, previous: &LocalTrack) {
    // Browsers may report a static anchor, so only changed values move the clock.
    if local.reported_position.is_none()
        || local.playing == previous.playing && local.reported_position == previous.reported_position
    {
        local.timeline.position_ms = previous.timeline.position_now();
        local.timeline.observed_at = Instant::now();
    }
    local.reported_position = local.reported_position.or(previous.reported_position);
    if local.track.duration_ms == 0 {
        local.track.duration_ms = previous.track.duration_ms;
    }
}

async fn read_player(connection: &Connection, source: &str) -> MusicResult<Option<LocalTrack>> {
    let proxy = Proxy::new(connection, source, PATH, "org.freedesktop.DBus.Properties").await?;
    Ok(parse_player(source, proxy.call("GetAll", &(PLAYER,)).await?))
}

fn parse_player(source: &str, mut properties: Properties) -> Option<LocalTrack> {
    let status: String = take(&mut properties, "PlaybackStatus").unwrap_or_default();
    if !matches!(status.as_str(), "Playing" | "Paused") {
        return None;
    }
    let mut metadata: Properties = take(&mut properties, "Metadata").unwrap_or_default();
    let url: String = take(&mut metadata, "xesam:url").unwrap_or_default();
    if source.to_ascii_lowercase().contains("spotify")
        || url.starts_with("spotify:")
        || reqwest::Url::parse(&url).is_ok_and(|url| url.host_str() == Some("open.spotify.com"))
    {
        return None;
    }
    let track_id: OwnedObjectPath = take(&mut metadata, "mpris:trackid")?;
    let uri = if url.is_empty() { track_id.to_string() } else { url };
    let playing = status == "Playing";
    let control = take::<bool>(&mut properties, "CanControl").unwrap_or(false);
    let reported_position = take::<i64>(&mut properties, "Position").map(|position| position.max(0) as f32 / 1000.0);
    let position = reported_position.unwrap_or(0.0);
    let rate =
        take::<f64>(&mut properties, "Rate").filter(|rate| rate.is_finite() && *rate > 0.0).unwrap_or(1.0) as f32;
    Some(LocalTrack {
        track: Track {
            id: None,
            uri,
            name: take(&mut metadata, "xesam:title").unwrap_or_else(|| "Unknown track".into()),
            original_title: None,
            artists: take(&mut metadata, "xesam:artist").unwrap_or_default(),
            album: take(&mut metadata, "xesam:album").unwrap_or_default(),
            image: take(&mut metadata, "mpris:artUrl"),
            duration_ms: (take::<i64>(&mut metadata, "mpris:length").unwrap_or(0).max(0) / 1000)
                .min(i64::from(u32::MAX)) as u32,
            interaction_id: Track::next_interaction_id(),
            runtime: TrackRuntime::default(),
        },
        timeline: Timeline {
            position_ms: position,
            rate: if playing { rate } else { 0.0 },
            observed_at: Instant::now(),
            queue_start_ms: -position,
            ..
        },
        playing,
        source: source.into(),
        mpris_id: track_id.to_string(),
        reported_position,
        can_seek: control && take::<bool>(&mut properties, "CanSeek").unwrap_or(false),
        can_toggle: control
            && take::<bool>(&mut properties, if playing { "CanPause" } else { "CanPlay" }).unwrap_or(false),
    })
}
