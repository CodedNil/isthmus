use crate::{
    app::{AppUpdater, send_update},
    music::{PlaybackCommand, TRACK_RESTART_GRACE, Track, should_restart_track},
};
use futures_util::{Stream, StreamExt as _, future::join_all, stream::select};
use parking_lot::Mutex;
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
    Connection, MatchRule, MessageStream, ObjectPath, OwnedObjectPath, OwnedVariant, Str, Variant,
    fdo::{self, DBusProxy, Properties, PropertiesProxy, RequestNameFlags},
    message::Type,
    names::InterfaceName,
    object_server::SignalEmitter,
};

const NAME: &str = "org.mpris.MediaPlayer2.cantus";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const TRACK_LIST: &str = "org.mpris.MediaPlayer2.TrackList";
const SKIP_FORWARD: &str = "cantus:skipForward";
// MPRIS reports the new state without identifying who requested it.
static EXPECTED_NOTICE: Mutex<Option<(Instant, String, Option<PlaybackStatus>)>> = Mutex::new(None);

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
    skip_forward: Option<bool>,
    paused: bool,
    position: i64,
    rate: f32,
    observed: Instant,
}

impl Playback {
    pub fn new(
        device: String,
        track: Metadata,
        paused: bool,
        observation: (f32, f32, Instant),
        skip_forward: Option<bool>,
    ) -> Self {
        let (position_ms, rate, observed) = observation;
        Self { device, track, skip_forward, paused, position: (position_ms * 1_000.0) as i64, rate, observed }
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
    let mut signals = player_signals(&connection).await?;
    connection.object_server().at(PATH, Root).await?;
    connection
        .object_server()
        .at(PATH, Player { playback: None, local_playback: local_playback.clone(), commands, restarted_at: None })
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

async fn player_signals(
    connection: &Connection,
) -> zbus::Result<impl Stream<Item = zbus::Result<zbus::Message>> + use<>> {
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
    let names = MessageStream::for_match_rule(names, connection, None).await?;
    let properties = MessageStream::for_match_rule(properties, connection, None).await?;
    Ok(select(names, properties))
}

fn other_player(name: &str) -> bool {
    media_player(name) && name != NAME
}

fn media_player(name: &str) -> bool {
    name.starts_with("org.mpris.MediaPlayer2.") && name != "org.mpris.MediaPlayer2.playerctld"
}

#[derive(Clone, PartialEq, Eq)]
pub struct MediaNotice {
    pub title: String,
    pub artist: String,
    pub status: PlaybackStatus,
    pub skip_forward: Option<bool>,
    track_id: Option<OwnedObjectPath>,
    uri: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

pub fn expect_notice(change: Option<(&str, Option<PlaybackStatus>)>) {
    *EXPECTED_NOTICE.lock() = change.map(|(uri, status)| (Instant::now(), uri.into(), status));
}

fn suppress_notice(name: &str, old: Option<&MediaNotice>, notice: &MediaNotice) -> bool {
    let spotify = name == NAME || name.split('.').nth(3) == Some("spotify");
    if !spotify || notice.title.is_empty() || notice.status == PlaybackStatus::Stopped {
        return false;
    }
    let track_changed = old.is_some_and(|old| old.track_id != notice.track_id || old.uri != notice.uri);
    let new_track = track_changed || old.is_none_or(|old| old.title.is_empty());
    EXPECTED_NOTICE
        .lock()
        .take_if(|(sent, uri, status)| {
            sent.elapsed() < Duration::from_secs(5)
                && notice.uri.as_ref() == Some(uri)
                && status.map_or(new_track, |status| {
                    !track_changed && notice.status == status && old.is_none_or(|old| old.status != status)
                })
        })
        .is_some()
}

pub fn watch_osd(updater: AppUpdater) {
    tokio::spawn(async move {
        let report = |notice| {
            send_update(&updater, move |app| {
                if let Some(status) = &mut app.bar.status {
                    status.show_media(notice);
                }
            })
        };
        while let Err(error) = watch_players(report).await {
            warn!(%error, "Could not watch MPRIS playback");
            sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn watch_players(mut report: impl FnMut(MediaNotice) -> bool) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let bus = DBusProxy::new(&connection).await?;
    let mut signals = player_signals(&connection).await?;
    let names = bus.list_names().await?;
    let connection = &connection;
    let bus = &bus;
    let mut players: HashMap<_, _> =
        join_all(names.into_iter().filter(|name| media_player(name.as_str())).map(|name| async move {
            let owner = bus.get_name_owner((*name).clone()).await.ok()?;
            let state = read_notice(connection, owner.as_str()).await;
            Some((name.to_string(), (owner.to_string(), state)))
        }))
        .await
        .into_iter()
        .flatten()
        .collect();

    while let Some(message) = signals.next().await {
        let message = message?;
        let header = message.header();
        let body = message.body();
        let notice = if header.member().is_some_and(|name| name.as_str() == "NameOwnerChanged") {
            let (name, _, owner) = body.deserialize::<(&str, &str, &str)>()?;
            if !media_player(name) {
                continue;
            }
            if owner.is_empty() {
                players.remove(name);
                continue;
            }
            if players.get(name).is_some_and(|(previous, _)| previous == owner) {
                continue;
            }
            let current = read_notice(connection, owner).await;
            players.insert(name.into(), (owner.into(), current.clone()));
            current
                .filter(|notice| notice.status == PlaybackStatus::Playing && !suppress_notice(name, None, notice))
                .map(|notice| MediaNotice { skip_forward: None, ..notice })
        } else {
            let Some(sender) = header.sender() else { continue };
            let Some((name, (owner, previous))) = players.iter_mut().find(|(_, (owner, _))| owner == sender.as_str())
            else {
                continue;
            };
            let (_, changed, invalidated) = body.deserialize::<(&str, HashMap<&str, Variant<'_>>, Vec<&str>)>()?;
            if !["PlaybackStatus", "Metadata"]
                .iter()
                .any(|name| changed.contains_key(name) || invalidated.contains(name))
            {
                continue;
            }
            let current = read_notice(connection, owner).await;
            if current == *previous {
                continue;
            }
            let old = mem::replace(previous, current.clone());
            let mut current = current.filter(|notice| !suppress_notice(name, old.as_ref(), notice));
            if let Some(notice) = &mut current {
                if notice.status == PlaybackStatus::Stopped {
                    let Some(old) = old.filter(|old| old.status != PlaybackStatus::Stopped) else { continue };
                    if notice.title.is_empty() {
                        notice.title = old.title;
                        notice.artist = old.artist;
                    }
                    notice.skip_forward = None;
                } else if let Some((before, after)) = old
                    .as_ref()
                    .and_then(|old| old.track_id.as_ref())
                    .zip(notice.track_id.as_ref())
                    .filter(|(before, after)| before != after)
                {
                    if notice.skip_forward.is_none() {
                        notice.skip_forward = track_list_direction(connection, owner, before, after).await;
                    }
                } else {
                    notice.skip_forward = None;
                }
            }
            current
        };
        if let Some(notice) = notice
            && (notice.status == PlaybackStatus::Stopped || !notice.title.is_empty())
            && !report(notice)
        {
            return Ok(());
        }
    }
    Err(zbus::Error::Failure("MPRIS event streams closed".into()))
}

async fn player_properties<'owner>(
    connection: &Connection,
    owner: &'owner str,
) -> zbus::Result<PropertiesProxy<'owner>> {
    PropertiesProxy::builder(connection).destination(owner).path(PATH).build().await
}

async fn read_notice(connection: &Connection, owner: &str) -> Option<MediaNotice> {
    timeout(Duration::from_millis(500), async {
        let player = player_properties(connection, owner).await.ok()?;
        let mut properties = player.get_all(InterfaceName::try_from(PLAYER).ok()?).await.ok()?;
        let status = properties.remove("PlaybackStatus")?;
        let status = match <&str>::try_from(&status).ok()? {
            "Playing" => PlaybackStatus::Playing,
            "Paused" => PlaybackStatus::Paused,
            "Stopped" => PlaybackStatus::Stopped,
            _ => return None,
        };
        let mut metadata = properties
            .remove("Metadata")
            .and_then(|value| HashMap::<String, OwnedVariant>::try_from(value).ok())
            .unwrap_or_default();
        let title = metadata
            .remove("xesam:title")
            .and_then(|value| String::try_from(value).ok())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_default();
        let artist = metadata
            .remove("xesam:artist")
            .and_then(|artists| Vec::<String>::try_from(artists).ok())
            .map_or_else(String::new, |artists| artists.join(", "));
        let track_id = metadata.remove("mpris:trackid").and_then(|value| OwnedObjectPath::try_from(value).ok());
        let skip_forward = metadata.remove(SKIP_FORWARD).and_then(|value| bool::try_from(value).ok());
        let mut uri = metadata.remove("xesam:url").and_then(|value| String::try_from(value).ok());
        if let Some(path) = uri.as_deref().and_then(|url| url.strip_prefix("https://open.spotify.com/")) {
            uri = Some(format!("spotify:{}", path.split('?').next().unwrap_or(path).replace('/', ":")));
        }
        Some(MediaNotice { title, artist, status, skip_forward, track_id, uri })
    })
    .await
    .ok()?
}

async fn track_list_direction(
    connection: &Connection,
    owner: &str,
    before: &OwnedObjectPath,
    after: &OwnedObjectPath,
) -> Option<bool> {
    timeout(Duration::from_millis(150), async {
        let player = player_properties(connection, owner).await.ok()?;
        let tracks = player.get(InterfaceName::from_static_str(TRACK_LIST).ok()?, "Tracks").await.ok()?;
        let tracks = Vec::<OwnedObjectPath>::try_from(tracks).ok()?;
        let before = tracks.iter().position(|track| track == before)?;
        let after = tracks.iter().position(|track| track == after)?;
        Some(after > before)
    })
    .await
    .ok()?
}

async fn other_player_playing(connection: &Connection, bus: &DBusProxy<'_>) -> zbus::Result<bool> {
    let names = bus.list_names().await?;
    let statuses = join_all(names.iter().filter(|name| other_player(name.as_str())).map(|name| {
        timeout(Duration::from_millis(500), async move {
            let player = player_properties(connection, name.as_str()).await.ok()?;
            let status = player.get(InterfaceName::from_static_str(PLAYER).ok()?, "PlaybackStatus").await.ok()?;
            <&str>::try_from(&status).ok().map(|status| status == "Playing")
        })
    }))
    .await;
    Ok(statuses.into_iter().any(|status| matches!(status, Ok(Some(true)))))
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
    restarted_at: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_restarts_then_skips_while_the_remote_position_is_stale() {
        let now = Instant::now();
        let track = Metadata {
            id: track_path("track", "uid"),
            uri: "track".into(),
            title: String::new(),
            artists: Vec::new(),
            album: String::new(),
            art: None,
            isrc: None,
            length: 30_000_000,
        };
        let (_local, local_playback) = watch::channel(false);
        let (commands, mut received) = mpsc::unbounded_channel();
        let mut player = Player {
            playback: Some(Playback::new("remote".into(), track, true, (15_000.0, 0.0, now), None)),
            local_playback,
            commands,
            restarted_at: None,
        };

        player.previous();
        assert!(matches!(received.try_recv().unwrap().command, PlaybackCommand::Seek(0)));
        player.previous();
        assert!(matches!(received.try_recv().unwrap().command, PlaybackCommand::Skip { forward: false }));
    }
}

impl Player {
    async fn update(&mut self, playback: Option<Playback>, signal: &SignalEmitter<'_>) -> zbus::Result<Option<bool>> {
        let previous = mem::replace(&mut self.playback, playback);
        let current = self.playback.as_ref();
        if !previous.as_ref().zip(current).is_some_and(|(previous, current)| {
            previous.device == current.device && previous.track.id == current.track.id
        }) {
            self.restarted_at = None;
        }
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
    fn next(&mut self) {
        self.restarted_at = None;
        self.command(|_| Some(PlaybackCommand::Skip { forward: true }));
    }

    fn previous(&mut self) {
        let now = Instant::now();
        let recently_restarted = self.restarted_at.is_some_and(|at| now - at < TRACK_RESTART_GRACE);
        let mut restart = false;
        self.command(|state| {
            restart = should_restart_track(state.position(now) as f32 / 1_000.0, recently_restarted);
            Some(if restart { PlaybackCommand::Seek(0) } else { PlaybackCommand::Skip { forward: false } })
        });
        if restart {
            self.restarted_at = Some(now);
        }
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
        if let Some(forward) = state.skip_forward {
            metadata.insert(SKIP_FORWARD, OwnedVariant::from(forward));
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
