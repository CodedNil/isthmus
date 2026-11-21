use crate::music::{PlaybackCommand, Track};
use futures_util::{StreamExt as _, future::join_all, stream::select};
use std::{
    borrow::Cow,
    collections::HashMap,
    mem,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, watch},
    time::{sleep, timeout},
};
use tracing::warn;
use zbus::{
    Connection, MatchRule, MessageStream, ObjectPath, OwnedObjectPath, OwnedVariant, Proxy, Str, Variant,
    fdo::{self, DBusProxy, Properties, RequestNameFlags},
    message::Type,
    names::InterfaceName,
    object_server::SignalEmitter,
    proxy::{Builder, CacheProperties},
};

const NAME: &str = "org.mpris.MediaPlayer2.cantus";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

pub struct MediaCommand {
    pub device: String,
    track_id: OwnedObjectPath,
    pub command: PlaybackCommand,
}

impl MediaCommand {
    pub fn matches_track(&self, uri: &str, uid: &str) -> bool {
        self.track_id == track_path(uri, uid)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Metadata {
    id: OwnedObjectPath,
    uri: String,
    title: String,
    artists: Vec<String>,
    album: String,
    art: Option<String>,
    isrc: Option<String>,
    length: i64,
}

impl From<&Track> for Metadata {
    fn from(track: &Track) -> Self {
        Self {
            id: track_path(&track.uri, &track.queue_uid),
            uri: track.uri.clone(),
            title: track.name.clone(),
            artists: track.artists.clone(),
            album: track.album.clone(),
            art: track.image.clone(),
            isrc: track.isrc.clone(),
            length: i64::from(track.duration_ms) * 1_000,
        }
    }
}

fn track_path(uri: &str, uid: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(format!("/io/github/CodedNil/Cantus/Track/t{}_{}", hex::encode(uri), hex::encode(uid)))
        .expect("hexadecimal object path")
}

#[derive(Clone)]
pub struct Playback {
    pub device: String,
    pub track: Metadata,
    paused: bool,
    position: i64,
    rate: f32,
    observed: Instant,
}

impl Playback {
    pub fn new(device: String, track: Metadata, paused: bool, observation: (f32, f32, Instant)) -> Self {
        let (position_ms, rate, observed) = observation;
        Self { device, track, paused, position: (position_ms * 1_000.0) as i64, rate, observed }
    }

    pub fn matches_track(&self, uri: &str, uid: &str) -> bool {
        self.track.id == track_path(uri, uid)
    }

    pub fn pause(&mut self) {
        self.position = self.position(Instant::now());
        self.paused = true;
        self.rate = 0.0;
    }

    fn position(&self, now: Instant) -> i64 {
        let elapsed = now.saturating_duration_since(self.observed).as_micros() as f64;
        let position = self.position.saturating_add((elapsed * f64::from(self.rate)) as i64);
        position.clamp(0, if self.track.length == 0 { i64::MAX } else { self.track.length })
    }
}

pub fn start(
    local_playback: watch::Receiver<bool>,
) -> (watch::Sender<Option<Playback>>, mpsc::UnboundedReceiver<MediaCommand>) {
    let (state, mut playback) = watch::channel(None);
    let (commands, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while playback.has_changed().is_ok() && playback.wait_for(Option::is_some).await.is_ok() {
            let Err(error) = serve(&mut playback, local_playback.clone(), commands.clone()).await else { break };
            warn!(%error, "Cantus MPRIS service stopped");
            sleep(Duration::from_secs(5)).await;
        }
    });
    (state, receiver)
}

async fn serve(
    playback: &mut watch::Receiver<Option<Playback>>,
    mut local_playback: watch::Receiver<bool>,
    commands: mpsc::UnboundedSender<MediaCommand>,
) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let dbus = DBusProxy::new(&connection).await?;
    let names = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg0ns("org.mpris.MediaPlayer2")
        .build()?;
    let properties = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")
        .member("PropertiesChanged")
        .path(PATH)
        .arg(0, PLAYER)
        .build()?;
    let names = MessageStream::for_match_rule(names, &connection, None).await?;
    let properties = MessageStream::for_match_rule(properties, &connection, None).await?;
    let mut signals = select(names, properties);
    connection.object_server().at(PATH, Root).await?;
    connection
        .object_server()
        .at(PATH, Player { playback: None, local_playback: local_playback.clone(), commands })
        .await?;
    let interface = connection.object_server().interface::<_, Player>(PATH).await?;
    let mut others_playing = other_player_playing(&connection, &dbus).await?;
    loop {
        let available = !*local_playback.borrow_and_update() && !others_playing;
        let snapshot = playback.borrow_and_update().as_ref().filter(|_| available).cloned();
        let visibility_changed = interface.get_mut().await.update(snapshot, interface.signal_emitter()).await?;
        match visibility_changed {
            Some(true) => {
                connection.request_name_with_flags(NAME, RequestNameFlags::DO_NOT_QUEUE).await?;
            }
            Some(false) => {
                connection.release_name(NAME).await?;
            }
            None => {}
        }
        tokio::select! {
            update = playback.changed() => {
                if update.is_err() {
                    return Ok(());
                }
            }
            update = local_playback.changed() => {
                if update.is_err() {
                    return Ok(());
                }
            }
            message = signals.next() => {
                let message = message.ok_or_else(|| zbus::Error::Failure("MPRIS event streams closed".into()))??;
                let header = message.header();
                let changed = if header.member().is_some_and(|member| member.as_str() == "NameOwnerChanged") {
                    let body = message.body();
                    let (name, _, _) = body.deserialize::<(&str, &str, &str)>()?;
                    other_player(name)
                } else {
                    header.sender() != connection.unique_name().map(|name| &**name)
                };
                if changed {
                    others_playing = other_player_playing(&connection, &dbus).await?;
                }
            }
        }
    }
}

fn other_player(name: &str) -> bool {
    name.starts_with("org.mpris.MediaPlayer2.") && name != NAME && name != "org.mpris.MediaPlayer2.playerctld"
}

async fn other_player_playing(connection: &Connection, bus: &DBusProxy<'_>) -> zbus::Result<bool> {
    let names = bus.list_names().await?;
    let statuses = join_all(names.iter().filter(|name| other_player(name.as_str())).map(|name| async move {
        timeout(Duration::from_millis(500), async {
            let player: Proxy<'_> = Builder::new(connection)
                .destination(name.as_str())
                .path(PATH)
                .interface(PLAYER)
                .cache_properties(CacheProperties::No)
                .build()
                .await?;
            player.get_property::<String>("PlaybackStatus").await
        })
        .await
    }))
    .await;
    Ok(statuses.into_iter().any(|status| matches!(status, Ok(Ok(status)) if status == "Playing")))
}

// Static capabilities still use zbus's typed properties and introspection.
macro_rules! interface {
    ($name:literal, $ty:ident { $($property:ident: $type:ty = $value:expr;)* } $($method:item)*) => {
        #[zbus::interface(name = $name)]
        #[expect(clippy::unused_self, reason = "D-Bus interface methods require a receiver")]
        impl $ty {
            $(
                #[zbus(property(emits_changed_signal = "const"))]
                const fn $property(&self) -> $type { $value }
            )*
            $($method)*
        }
    };
}

struct Root;

interface!("org.mpris.MediaPlayer2", Root {
    can_quit: bool = false;
    can_raise: bool = false;
    has_track_list: bool = false;
    identity: &'static str = "Cantus";
    supported_uri_schemes: Vec<&'static str> = Vec::new();
    supported_mime_types: Vec<&'static str> = Vec::new();
}
    const fn raise(&self) {}
    const fn quit(&self) {}
);

struct Player {
    playback: Option<Playback>,
    local_playback: watch::Receiver<bool>,
    commands: mpsc::UnboundedSender<MediaCommand>,
}

impl Player {
    async fn update(&mut self, playback: Option<Playback>, signal: &SignalEmitter<'_>) -> zbus::Result<Option<bool>> {
        let previous = mem::replace(&mut self.playback, playback);
        let current = self.playback.as_ref();
        if previous.is_some() != current.is_some() {
            return Ok(Some(current.is_some()));
        }
        let Some((previous, current)) = previous.as_ref().zip(current) else { return Ok(None) };

        let mut changed = HashMap::new();
        if previous.paused != current.paused {
            changed.insert("PlaybackStatus", Variant::from(self.playback_status()));
        }
        if previous.track != current.track {
            changed.insert("Metadata", Variant::from(self.metadata().map_err(zbus::Error::from)?));
        }
        if !changed.is_empty() {
            Properties::properties_changed(signal, InterfaceName::from_static_str(PLAYER)?, changed, Cow::default())
                .await?;
        }
        let now = Instant::now();
        let position = current.position(now);
        if previous.track.id == current.track.id && previous.position(now).abs_diff(position) > 1_000_000 {
            Self::seeked(signal, position).await?;
        }
        Ok(None)
    }

    fn command(&self, command: impl FnOnce(&Playback) -> Option<PlaybackCommand>) {
        if !*self.local_playback.borrow()
            && let Some(playback) = &self.playback
            && let Some(command) = command(playback)
        {
            let _sent = self.commands.send(MediaCommand {
                device: playback.device.clone(),
                track_id: playback.track.id.clone(),
                command,
            });
        }
    }
}

interface!("org.mpris.MediaPlayer2.Player", Player {
    minimum_rate: f64 = 1.0;
    maximum_rate: f64 = 1.0;
    can_go_next: bool = true;
    can_go_previous: bool = true;
    can_play: bool = true;
    can_pause: bool = true;
    can_seek: bool = true;
    can_control: bool = true;
}
    fn next(&self) {
        self.command(|_| Some(PlaybackCommand::Skip { forward: true }));
    }

    fn previous(&self) {
        self.command(|_| Some(PlaybackCommand::Skip { forward: false }));
    }

    fn play(&self) {
        self.command(|state| state.paused.then_some(PlaybackCommand::SetPlaying(true)));
    }

    fn pause(&self) {
        self.command(|state| (!state.paused).then_some(PlaybackCommand::SetPlaying(false)));
    }

    fn play_pause(&self) {
        self.command(|state| Some(PlaybackCommand::SetPlaying(state.paused)));
    }

    fn stop(&self) {
        self.pause();
    }

    fn seek(&self, offset: i64) {
        self.command(|state| {
            let position = state.position(Instant::now()).saturating_add(offset).max(0);
            Some(if state.track.length > 0 && position > state.track.length {
                PlaybackCommand::Skip { forward: true }
            } else {
                PlaybackCommand::Seek((position / 1_000).min(i64::from(u32::MAX)) as u32)
            })
        });
    }

    fn set_position(&self, track_id: &ObjectPath<'_>, position: i64) {
        self.command(|state| {
            (track_id == &**state.track.id && (0..state.track.length).contains(&position))
                .then_some(PlaybackCommand::Seek((position / 1_000) as u32))
        });
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    const fn playback_status(&self) -> &'static str {
        match &self.playback {
            Some(playback) if playback.paused => "Paused",
            Some(_) => "Playing",
            None => "Stopped",
        }
    }

    #[zbus(property)]
    fn metadata(&self) -> fdo::Result<HashMap<&str, OwnedVariant>> {
        let Some(state) = self.playback.as_ref() else { return Ok(HashMap::new()) };
        let track = &state.track;
        let mut metadata = HashMap::from([
            ("mpris:trackid", OwnedVariant::from((*track.id).clone())),
            ("xesam:url", OwnedVariant::from(Str::from(track.uri.as_str()))),
            ("xesam:title", OwnedVariant::from(Str::from(track.title.as_str()))),
            ("xesam:artist", OwnedVariant::try_from(Variant::from(track.artists.as_slice()))?),
            ("xesam:album", OwnedVariant::from(Str::from(track.album.as_str()))),
        ]);
        if track.length > 0 {
            metadata.insert("mpris:length", OwnedVariant::from(track.length));
        }
        for (key, text) in [("mpris:artUrl", &track.art), ("xesam:isrc", &track.isrc)] {
            if let Some(text) = text {
                metadata.insert(key, OwnedVariant::from(Str::from(text.as_str())));
            }
        }
        Ok(metadata)
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.playback.as_ref().map_or(0, |state| state.position(Instant::now()))
    }

    #[zbus(property)]
    const fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    #[expect(clippy::float_cmp, reason = "MPRIS specifies exact values for pause and normal speed")]
    fn set_rate(&self, rate: f64) -> fdo::Result<()> {
        if rate == 0.0 {
            self.pause();
        } else if rate != 1.0 {
            return Err(fdo::Error::NotSupported("Playback rate is fixed by Spotify".into()));
        }
        Ok(())
    }

    #[zbus(property)]
    const fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn set_volume(&self, volume: f64) -> fdo::Result<()> {
        let _ = volume;
        Err(fdo::Error::NotSupported("Remote volume control is unavailable".into()))
    }

 );

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zbus::fdo::PropertiesProxy;

    struct LocalPlayer;

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl LocalPlayer {
        #[zbus(property(emits_changed_signal = "const"))]
        #[expect(clippy::unused_self, reason = "D-Bus properties require a receiver")]
        const fn playback_status(&self) -> &'static str {
            "Playing"
        }
    }

    async fn wait_for_name(bus: &DBusProxy<'_>, present: bool) {
        timeout(Duration::from_secs(3), async {
            while bus.name_has_owner(NAME.try_into().unwrap()).await.unwrap() != present {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Cantus should update whether it owns the MPRIS name");
    }

    #[tokio::test]
    #[ignore = "Run with dbus-run-session to use an isolated session bus"]
    async fn publishes_skip_metadata_and_yields_to_local_playback() {
        let connection = Connection::session().await.unwrap();
        let bus = DBusProxy::new(&connection).await.unwrap();
        let (local_audio, audio) = watch::channel(false);
        let (remote, mut commands) = start(audio);
        wait_for_name(&bus, false).await;

        let uri = "spotify:track:6vWfZZyIlODYPMEbsrOK3G";
        let isrc = "USRC17607839";
        let track: Track = serde_json::from_value(json!({
            "name": "First", "uri": uri, "isrc": isrc, "artists": ["Artist"], "album": "Album", "duration_ms": 180_000
        }))
        .unwrap();
        let first = Metadata::from(&track);
        let update_track =
            |track| remote.send_replace(Some(Playback::new("phone".into(), track, false, (0.0, 1.0, Instant::now()))));
        update_track(first.clone());
        wait_for_name(&bus, true).await;

        let player = Builder::<Proxy<'_>>::new(&connection)
            .destination(NAME)
            .path(PATH)
            .interface(PLAYER)
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .unwrap();
        let props = PropertiesProxy::builder(&connection).destination(NAME).path(PATH).build().await.unwrap();
        let mut updates = props.receive_properties_changed().await.unwrap();
        let next_uri = "spotify:track:3OIgsG6duyvwodGPoFYVzJ";
        let next = Metadata {
            id: track_path(next_uri, ""),
            uri: next_uri.into(),
            title: "Next".into(),
            isrc: None,
            ..first.clone()
        };
        for (method, forward, track) in [("Next", true, next), ("Previous", false, first)] {
            player.call::<_, _, ()>(method, &()).await.unwrap();
            let request = timeout(Duration::from_secs(1), commands.recv()).await.unwrap().unwrap();
            assert_eq!(request.device, "phone");
            assert!(matches!(request.command, PlaybackCommand::Skip { forward: direction } if direction == forward));

            update_track(track.clone());
            let update = timeout(Duration::from_secs(1), updates.next()).await.unwrap().unwrap();
            let args = update.args().unwrap();
            let signal = OwnedVariant::try_from(&args.changed_properties()["Metadata"]).unwrap();
            let metadata = HashMap::<String, OwnedVariant>::try_from(signal).unwrap();
            assert_eq!(metadata["xesam:title"], OwnedVariant::from(Str::from(track.title)));
            assert_eq!(metadata["xesam:url"], OwnedVariant::from(Str::from(track.uri)));
            assert_eq!(
                metadata.get("xesam:isrc"),
                track.isrc.map(|value| OwnedVariant::from(Str::from(value))).as_ref()
            );
            assert_eq!(player.get_property::<HashMap<String, OwnedVariant>>("Metadata").await.unwrap(), metadata);
        }

        let local_name = "org.mpris.MediaPlayer2.cantus_test";
        connection.object_server().at(PATH, LocalPlayer).await.unwrap();
        connection.request_name(local_name).await.unwrap();
        wait_for_name(&bus, false).await;
        connection.release_name(local_name).await.unwrap();
        wait_for_name(&bus, true).await;

        local_audio.send_replace(true);
        wait_for_name(&bus, false).await;
        local_audio.send_replace(false);
        wait_for_name(&bus, true).await;
        remote.send_replace(None);
        wait_for_name(&bus, false).await;
    }
}
