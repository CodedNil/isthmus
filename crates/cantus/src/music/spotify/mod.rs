use self::{
    library::CachedPlaylist,
    lyrics::SpotifyLyrics,
    proto::{
        connect::{
            Capabilities, Cluster, Device as ConnectDevice, DeviceInfo, DeviceType, MemberType, PlayerState,
            ProvidedTrack, PutStateReason, PutStateRequest,
        },
        metadata,
    },
    session::Session,
};
use super::{ART_SIZE, AudioFeatures, MusicResult, PlaybackCommand, PlaylistId, Track, TrackId, TrackRuntime};
use crate::{
    app::{AppUpdater, send_update},
    config::{Config, MAX_PLAYLIST_TARGETS},
    platform::mpris,
};
use arrayvec::{ArrayString, ArrayVec};
use bytes::Bytes;
use reqwest::{Client, Version};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    io, mem, str,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        watch,
    },
    time::{Instant as TokioInstant, sleep, timeout},
};
use tracing::{error, info, warn};

mod auth;
mod client;
mod dealer;
mod library;
pub(super) mod lyrics;
mod proto;
mod session;

const METADATA_CACHE_LIMIT: usize = 512;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const CLUSTER_TOPIC: &str = "hm://connect-state/v1/cluster";

#[derive(Clone)]
pub struct Spotify {
    commands: UnboundedSender<PlaybackCommand>,
    session: watch::Receiver<Option<Arc<Session>>>,
}

type CatalogTrack = metadata::Track;
type Metadata = HashMap<String, Option<CatalogTrack>>;

impl Spotify {
    pub(super) fn new(config: &Config, updater: &AppUpdater, local_playback: watch::Receiver<bool>) -> Self {
        let (commands, mut receiver) = mpsc::unbounded_channel();
        let (connected_session, session) = watch::channel(None);
        let updater = updater.clone();
        let playlist_targets = config.playlists.clone();
        let ratings_enabled = config.ratings_enabled;
        tokio::spawn(async move {
            let reconnect = async {
                loop {
                    match auth::connect().await {
                        Ok(session) => {
                            connected_session.send_replace(Some(Arc::clone(&session)));
                            let result = run_spotify(
                                session,
                                &mut receiver,
                                updater.clone(),
                                playlist_targets.clone(),
                                ratings_enabled,
                                local_playback.clone(),
                            )
                            .await;
                            connected_session.send_replace(None);
                            freeze_playback(&updater);
                            if let Err(error) = result {
                                warn!(%error, "Spotify disconnected");
                            }
                        }
                        Err(error) => warn!(%error, "Spotify connection failed"),
                    }
                    sleep(Duration::from_secs(5)).await;
                }
            };
            tokio::select! {
                () = connected_session.closed() => {},
                () = reconnect => {},
            }
        });
        Self { commands, session }
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        if self.commands.send(command).is_err() {
            warn!("Discarded music command after Spotify worker stopped");
        }
    }

    pub(super) async fn audio_features(&self, ids: &[TrackId]) -> MusicResult<HashMap<TrackId, AudioFeatures>> {
        let session = self.session.borrow().clone().ok_or_else(|| io::Error::other("Spotify is not connected"))?;
        timeout(REQUEST_TIMEOUT, session.audio_features(ids)).await?
    }

    pub(super) async fn get_lyrics(&self, id: TrackId) -> MusicResult<Option<SpotifyLyrics>> {
        let Some(session) = self.session.borrow().clone() else { return Ok(None) };
        timeout(REQUEST_TIMEOUT, session.lyrics(id)).await?
    }

    pub(super) async fn artwork(http: &Client, url: &str) -> MusicResult<Bytes> {
        const CDN: &str = "https://image-cdn-fa.spotifycdn.com/image/";
        let id = [CDN, "https://i.scdn.co/image/", "https://image-cdn-ak.spotifycdn.com/image/"]
            .into_iter()
            .find_map(|prefix| url.strip_prefix(prefix))
            .filter(|id| id.len() == 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let request = id.map_or_else(|| http.get(url), |id| http.get(format!("{CDN}{id}")));
        Ok(request.version(Version::HTTP_3).send().await?.error_for_status()?.bytes().await?)
    }
}

async fn run_spotify(
    session: Arc<Session>,
    commands: &mut UnboundedReceiver<PlaybackCommand>,
    updater: AppUpdater,
    playlist_targets: ArrayVec<String, MAX_PLAYLIST_TARGETS>,
    ratings_enabled: bool,
    local_playback: watch::Receiver<bool>,
) -> MusicResult<()> {
    let mut dealer = dealer::Dealer::start(Arc::clone(&session)).await?;

    let (mpris, mut media_keys) = mpris::start(local_playback.clone());
    let mut worker =
        SpotifyWorker { playlist_targets, ratings_enabled, mpris: Some(mpris), ..SpotifyWorker::new(session, updater) };
    let mut registered = false;
    let mut playlists_dirty = false;
    let mut pending_playlists = HashMap::<PlaylistId, Vec<u8>>::new();
    let mut playlist_read = None;
    let mut metadata = None;

    loop {
        if playlist_read.is_none() {
            pending_playlists.retain(|&id, revision| worker.needs_playlist_refresh(id, revision));
            if playlists_dirty || !pending_playlists.is_empty() {
                let requested = if mem::take(&mut playlists_dirty) {
                    Vec::new()
                } else {
                    pending_playlists.drain().collect::<Vec<_>>()
                };
                let read = worker.playlist_refresh(requested.iter().map(|&(id, _)| id).collect());
                playlist_read = Some((requested, Box::pin(read)));
            }
        }
        if metadata.is_none() && worker.metadata_dirty {
            worker.metadata_dirty = false;
            let uris = worker.missing_metadata();
            if !uris.is_empty() {
                let session = Arc::clone(&worker.session);
                metadata = Some(Box::pin(async move { session.track_metadata(&uris).await }));
            }
        }
        tokio::select! {
            Some(request) = media_keys.recv() => {
                if !*local_playback.borrow() && worker.accepts_media_command(&request) {
                    timeout(Duration::from_secs(20), worker.command(request.command)).await
                        .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, format!("Spotify command: {error}")))?;
                }
            }
            Some(command) = commands.recv() => {
                let edited = matches!(
                    &command,
                    PlaybackCommand::UpdateLibrary { playlists, .. } if !playlists.is_empty()
                );
                if edited && let Some((requested, _)) = playlist_read.take() {
                    playlists_dirty |= requested.is_empty();
                    for (id, revision) in requested {
                        pending_playlists.entry(id).or_insert(revision);
                    }
                }
                playlists_dirty |= timeout(Duration::from_secs(20), worker.command(command)).await
                    .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, format!("Spotify command: {error}")))?;
            }
            result = async { metadata.as_mut().expect("pending metadata").await }, if metadata.is_some() => {
                metadata = None;
                match result {
                    Ok(mut values) if !values.is_empty() => {
                        values.retain(|uri, track| track.is_some() || worker.player.as_ref().is_some_and(|player|
                            player_tracks(player).any(|track| requested_track_uri(track) == uri)));
                        worker.track_metadata.extend(values);
                        worker.publish(false);
                    }
                    Ok(_) => {}
                    Err(error) => warn!(%error, "Failed to fetch Spotify track metadata"),
                }
            }
            event = dealer.next() => match event? {
                dealer::Event::Connected(connection_id) => {
                    worker.connection_id = connection_id;
                    let reason = if registered { PutStateReason::NewConnection } else { PutStateReason::NewDevice };
                    let cluster = timeout(REQUEST_TIMEOUT, worker.register(reason))
                        .await.map_err(|error| {
                            io::Error::new(io::ErrorKind::TimedOut, format!("Spotify observer registration: {error}"))
                        })??;
                    info!(reconnected = registered, "Spotify playback subscription registered");
                    let reported_track = cluster.player_state.as_ref().and_then(|player| player.track.as_ref());
                    if !registered && reported_track.is_none_or(|track| track.uri.is_empty()) {
                        info!("Spotify reported no current track. Start playback in Spotify to populate the queue.");
                    }
                    registered = true;
                    worker.update_cluster(cluster);
                    playlists_dirty = true;
                }
                dealer::Event::Cluster(cluster) => worker.update_cluster(*cluster),
                dealer::Event::Disconnected => {
                    worker.connection_id.clear();
                    worker.metadata_dirty = false;
                    worker.clear_active_device();
                }
                dealer::Event::RootlistChanged(revision) => {
                    playlists_dirty |= worker.needs_root_refresh(&revision);
                }
                dealer::Event::PlaylistChanged { id, revision, membership_only } => {
                    // A rename can bring a previously untracked playlist into the configured targets.
                    playlists_dirty |= !membership_only && !worker.cached_playlists.contains_key(&id);
                    pending_playlists.insert(id, revision);
                }
            },
            result = async { playlist_read.as_mut().expect("pending playlist read").1.as_mut().await },
                if playlist_read.is_some() => {
                let (requested, _) = playlist_read.take().expect("pending playlist read");
                playlists_dirty |= worker.apply_playlist_read(&requested, result);
            }
        }
    }
}

fn freeze_playback(updater: &AppUpdater) {
    send_update(updater, |app| {
        let queue = &mut app.music.queue;
        let position = queue.timeline.position_now();
        let position = queue
            .tracks
            .get(queue.index)
            .filter(|track| track.duration_ms > 0)
            .map_or(position, |track| position.min(track.duration_ms as f32));
        queue.observe(queue.index, position, 0.0, Instant::now());
        app.music.playing = false;
    });
}

fn playback_position(player: &PlayerState, server_now: i64) -> (f32, f32) {
    let rate = if player.is_playing
        && !player.is_paused
        && !player.is_buffering
        && player.playback_speed.is_finite()
        && player.playback_speed > 0.0
    {
        player.playback_speed as f32
    } else {
        0.0
    };
    // Spotify timestamps use the server clock. A missing timestamp supplies no elapsed time.
    let age_ms = if player.timestamp > 0 { server_now.saturating_sub(player.timestamp).max(0) } else { 0 };
    let position = (player.position_as_of_timestamp.max(0) as f64 + age_ms as f64 * f64::from(rate)) as f32;
    (position, rate)
}

struct SpotifyWorker {
    session: Arc<Session>,
    updater: AppUpdater,
    connection_id: String = String::new(),
    active_device: Option<String> = None,
    playlist_targets: ArrayVec<String, MAX_PLAYLIST_TARGETS>,
    cached_playlists: Arc<HashMap<PlaylistId, CachedPlaylist>>,
    root_revision: Vec<u8> = Vec::new(),
    ratings_enabled: bool = false,
    track_metadata: Metadata,
    metadata_dirty: bool = false,
    player: Option<PlayerState> = None,
    queue_context: String = String::new(),
    skip_forward: Option<bool> = None,
    server_clock: Option<(i64, TokioInstant)> = None,
    mpris: Option<watch::Sender<Option<mpris::Playback>>> = None,
}

impl SpotifyWorker {
    fn new(session: Arc<Session>, updater: AppUpdater) -> Self {
        Self {
            session,
            updater,
            playlist_targets: ArrayVec::new(),
            cached_playlists: Arc::default(),
            track_metadata: HashMap::new(),
            ..
        }
    }

    async fn command(&mut self, command: PlaybackCommand) -> bool {
        let mut command = match command {
            PlaybackCommand::PlayPlaylist(id) => {
                let uri = id.map_or_else(
                    || format!("spotify:user:{}:collection", self.session.username),
                    |id| format!("spotify:playlist:{id}"),
                );
                json!({
                    "endpoint": "play",
                    "context": { "uri": uri, "url": format!("context://{uri}") },
                    "play_origin": { "feature_identifier": "cantus" },
                    "options": {
                        "initially_paused": false,
                        "skip_to": {},
                        "player_options_override": { "shuffling_context": true },
                    },
                    "play_options": {
                        "reason": "interactive",
                        "operation": "replace",
                        "trigger": "immediately",
                    },
                })
            }
            PlaybackCommand::SetPlaying(playing) => {
                json!({ "endpoint": if playing { "resume" } else { "pause" } })
            }
            PlaybackCommand::Seek(position_ms) => json!({ "endpoint": "seek_to", "value": position_ms }),
            PlaybackCommand::Skip { forward } => {
                json!({ "endpoint": if forward { "skip_next" } else { "skip_prev" } })
            }
            PlaybackCommand::JumpTo { uri, uid } => {
                let Some((track, forward)) =
                    self.player.as_ref().and_then(|player| queued_occurrence(player, &uri, &uid))
                else {
                    warn!("Spotify queue target is no longer available or is ambiguous");
                    return false;
                };
                json!({
                    "endpoint": if forward { "skip_next" } else { "skip_prev" },
                    "track": { "uri": track.uri, "uid": track.uid, "metadata": track.metadata },
                })
            }
            PlaybackCommand::UpdateLibrary { track_id, playlists, liked } => {
                return self.update_library(track_id, &playlists, liked).await;
            }
        };
        let option_group = if command["endpoint"] == "play" { "play_options" } else { "options" };
        for option in ["override_restrictions", "only_for_local_device", "system_initiated"] {
            command[option_group][option] = false.into();
        }
        command["logging_params"] = json!({});
        let Some(target) = &self.active_device else { return false };
        if let Err(error) = self.session.post_player_command(&self.connection_id, target, &command).await {
            error!(%error, endpoint = %command["endpoint"], "Spotify player command failed");
        }
        false
    }

    async fn register(&self, reason: PutStateReason) -> MusicResult<Cluster> {
        let request = PutStateRequest {
            device: Some(ConnectDevice {
                device_info: Some(DeviceInfo {
                    name: "Cantus".into(),
                    capabilities: Some(Capabilities {
                        is_observable: true,
                        needs_full_player_state: true,
                        hidden: true,
                        supports_gzip_pushes: true,
                        supports_playlist_v2: true,
                        supported_types: vec!["audio/track".into(), "audio/episode".into()],
                    }),
                    device_type: DeviceType::Observer as i32,
                    device_id: self.session.device_id.clone(),
                    client_id: auth::CLIENT_ID.into(),
                }),
                player_state: Some(PlayerState {
                    session_id: self.session.id.clone(),
                    playback_speed: 1.0,
                    options: Some(()),
                    suppressions: Some(()),
                    ..Default::default()
                }),
            }),
            member_type: MemberType::ConnectState as i32,
            put_state_reason: reason as i32,
            client_side_timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        };
        self.session.put_connect_state(&self.connection_id, &request).await
    }

    fn server_now(&self) -> i64 {
        self.server_clock.map_or_else(
            || SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
            |(timestamp, observed)| timestamp.saturating_add(observed.elapsed().as_millis() as i64),
        )
    }

    fn update_cluster(&mut self, cluster: Cluster) {
        if cluster.server_timestamp_ms > 0 {
            self.server_clock = Some((cluster.server_timestamp_ms, TokioInstant::now()));
        }
        let same_device = self.active_device.as_deref() == Some(cluster.active_device_id.as_str());
        if !same_device && let Some(mpris) = &self.mpris {
            mpris.send_replace(None);
        }
        let Some(player) = cluster.player_state else { return };
        let context_changed = !player.context_uri.is_empty() && player.context_uri != self.queue_context;
        if context_changed {
            info!(context = %player.context_uri, "Spotify queue context changed");
            self.queue_context.clone_from(&player.context_uri);
        }
        let Some(track) = player.track.as_ref().filter(|track| !track.uri.is_empty()) else {
            if cluster.active_device_id.is_empty() && !player.is_playing {
                self.clear_active_device();
            } else if same_device && player.is_paused {
                if let Some(previous) = &mut self.player {
                    previous.is_paused = true;
                }
                freeze_playback(&self.updater);
                if let Some(mpris) = &self.mpris {
                    mpris.send_if_modified(|state| {
                        let Some(state) = state else { return false };
                        state.pause();
                        true
                    });
                }
            }
            return;
        };
        let previous = self.player.as_ref().filter(|_| !context_changed && same_device);
        if !previous.and_then(|old| old.track.as_ref()).is_some_and(|old| old.uri == track.uri && old.uid == track.uid)
        {
            self.skip_forward = previous
                .and_then(|old| queued_occurrence(old, requested_track_uri(track), &track.uid))
                .map(|(_, forward)| forward);
        }
        self.active_device = (!cluster.active_device_id.is_empty()).then_some(cluster.active_device_id);
        // Keep recent metadata across temporary queue gaps.
        let prune = self.track_metadata.len() > METADATA_CACHE_LIMIT;
        if prune || self.track_metadata.values().any(Option::is_none) {
            let uris: HashSet<_> = player_tracks(&player).map(requested_track_uri).collect();
            self.track_metadata.retain(|uri, track| (!prune && track.is_some()) || uris.contains(uri.as_str()));
        }
        self.player = Some(player);
        self.metadata_dirty = true;
        self.publish(true);
    }

    fn missing_metadata(&self) -> Vec<String> {
        let Some(player) = &self.player else { return Vec::new() };
        let mut seen = HashSet::new();
        player_tracks(player)
            .map(requested_track_uri)
            .filter(|uri| {
                !self.track_metadata.contains_key(*uri) && track_id_from_uri(uri).is_some() && seen.insert(*uri)
            })
            .map(str::to_owned)
            .collect()
    }

    fn clear_active_device(&mut self) {
        if self.active_device.take().is_some() {
            info!("Spotify has no active playback device");
        }
        if let Some(player) = &mut self.player {
            player.is_playing = false;
        }
        if let Some(mpris) = &self.mpris {
            mpris.send_replace(None);
        }
        freeze_playback(&self.updater);
    }

    fn accepts_media_command(&self, request: &mpris::MediaCommand) -> bool {
        self.active_device.as_ref() == Some(&request.device)
            && self
                .mpris
                .as_ref()
                .is_some_and(|state| state.borrow().as_ref().is_some_and(|playback| playback.device == request.device))
            && (!matches!(request.command, PlaybackCommand::Seek(_))
                || self
                    .player
                    .as_ref()
                    .and_then(|player| player.track.as_ref())
                    .is_some_and(|track| request.matches_track(requested_track_uri(track), &track.uid)))
    }

    fn publish_mpris(&self, player: &PlayerState, track: Option<&Track>, observation: Option<(f32, f32, Instant)>) {
        let Some(mpris) = &self.mpris else { return };
        let (Some(device), Some(track)) = (&self.active_device, track) else {
            mpris.send_replace(None);
            return;
        };
        if device == &self.session.device_id || !(player.is_playing || player.is_paused || player.is_buffering) {
            mpris.send_replace(None);
            return;
        }
        // A skip can report the new URI before its display metadata arrives.
        if track.name.trim().is_empty() && track.id.is_some() && !self.track_metadata.contains_key(&track.uri) {
            return;
        }
        mpris.send_if_modified(|state| {
            let metadata = mpris::Metadata::from(track);
            if observation.is_none()
                && let Some(previous) = state
                && previous.device == *device
                && previous.matches_track(&track.uri, &track.queue_uid)
            {
                if previous.track == metadata {
                    return false;
                }
                previous.track = metadata;
            } else {
                let observation = observation.unwrap_or_else(|| self.observe_position(player));
                *state = Some(mpris::Playback::new(
                    device.clone(),
                    metadata,
                    player.is_paused,
                    observation,
                    self.skip_forward,
                ));
            }
            true
        });
    }

    fn observe_position(&self, player: &PlayerState) -> (f32, f32, Instant) {
        let (position, rate) = playback_position(player, self.server_now());
        (position, rate, Instant::now())
    }

    /// Publish queue metadata; only a new player report may reset the playback clock.
    fn publish(&self, observe_playback: bool) {
        let Some(player) = &self.player else { return };
        let index = player.prev_tracks.iter().filter(|track| !track.uri.ends_with(":delimiter")).count();
        let convert = |track: &ProvidedTrack, duration| {
            let catalog = self.track_metadata.get(requested_track_uri(track)).and_then(Option::as_ref);
            track_from_provided(track, catalog, duration)
        };
        let is_track = |track: &&ProvidedTrack| !track.uri.ends_with(":delimiter");
        let current =
            player.track.as_ref().filter(is_track).map(|track| convert(track, u32::try_from(player.duration).ok()));
        let playing = player.is_playing && !player.is_paused;
        let observation = observe_playback.then(|| self.observe_position(player));
        self.publish_mpris(player, current.as_ref(), observation);
        let tracks = player
            .prev_tracks
            .iter()
            .filter(is_track)
            .map(|track| convert(track, None))
            .chain(current)
            .chain(player.next_tracks.iter().filter(is_track).map(|track| convert(track, None)))
            .collect::<Vec<_>>();
        send_update(&self.updater, move |app| {
            let music = &mut app.music;
            let position = observation.map_or_else(|| music.queue.timeline.position_now(), |(position, ..)| position);
            music.queue.replace(tracks, index, position);
            if let Some((position, rate, observed_at)) = observation {
                music.queue.observe(index, position, rate, observed_at);
                if playing && !music.playing {
                    music.last_toggle = observed_at;
                }
                music.playing = playing;
            }
            app.refresh_enrichment();
        });
    }
}

fn player_tracks(player: &PlayerState) -> impl Iterator<Item = &ProvidedTrack> {
    player.prev_tracks.iter().chain(player.track.as_ref()).chain(&player.next_tracks)
}

fn queued_occurrence<'player>(
    player: &'player PlayerState,
    uri: &str,
    uid: &str,
) -> Option<(&'player ProvidedTrack, bool)> {
    let matches = |track: &ProvidedTrack| requested_track_uri(track) == uri && (uid.is_empty() || track.uid == uid);
    if player.track.as_ref().is_some_and(matches) {
        return None;
    }
    let mut candidates = player
        .prev_tracks
        .iter()
        .filter(|track| matches(track))
        .map(|track| (track, false))
        .chain(player.next_tracks.iter().filter(|track| matches(track)).map(|track| (track, true)));
    let target = candidates.next()?;
    candidates.next().is_none().then_some(target)
}

fn parse_id(id: &str) -> Option<ArrayString<22>> {
    (id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| ArrayString::from(id).ok())
        .flatten()
}

fn track_id_from_uri(uri: &str) -> Option<TrackId> {
    parse_id(uri.strip_prefix("spotify:track:")?)
}

/// Keeps the requested recording's identity when Spotify plays an alternative.
fn requested_track_uri(track: &ProvidedTrack) -> &str {
    track
        .metadata
        .get("requested_uri")
        .filter(|uri| track_id_from_uri(&track.uri).is_some() && track_id_from_uri(uri).is_some())
        .map_or(track.uri.as_str(), String::as_str)
}

fn track_from_provided(
    track: &ProvidedTrack,
    catalog: Option<&CatalogTrack>,
    fallback_duration_ms: Option<u32>,
) -> Track {
    let text = |key, fallback: &str| {
        track.metadata.get(key).map(String::as_str).filter(|s| !s.is_empty()).unwrap_or(fallback).to_owned()
    };
    let mut artists: Vec<_> = catalog
        .into_iter()
        .flat_map(|track| track.artist.iter())
        .filter(|artist| !artist.name().is_empty())
        .map(|artist| artist.name().to_owned())
        .collect();
    if artists.is_empty() {
        artists.extend(track.metadata.get("artist_name").filter(|s| !s.is_empty()).cloned());
    }
    let uri = requested_track_uri(track);
    Track {
        id: track_id_from_uri(uri),
        isrc: catalog
            .and_then(|track| track.external_id.iter().find(|id| id.r#type().eq_ignore_ascii_case("isrc")))
            .map(|id| id.id().to_owned()),
        uri: uri.to_owned(),
        queue_uid: track.uid.clone(),
        original_title: catalog.map(|track| track.original_title().to_owned()),
        name: catalog
            .filter(|track| !track.name().is_empty())
            .map_or_else(|| text("title", ""), |track| track.name().to_owned()),
        artists,
        album: text("album_title", catalog.and_then(|track| track.album.as_ref()).map_or("", metadata::Album::name)),
        image: ["image_url", "image_large_url", "image_xlarge_url"]
            .into_iter()
            .find_map(|key| track.metadata.get(key).filter(|url| !url.is_empty()))
            .map(|url| image_url(url))
            .or_else(|| {
                let album = catalog?.album.as_ref()?;
                let covers = album.cover_group.as_ref().map_or(&[][..], |group| group.image.as_slice());
                let images = if covers.is_empty() { &album.cover } else { covers };
                let image = images
                    .iter()
                    .filter(|image| image.file_id().len() == 20)
                    .min_by_key(|image| image.width().abs_diff(ART_SIZE as i32))?;
                Some(format!("https://i.scdn.co/image/{}", hex::encode(image.file_id())))
            }),
        duration_ms: [
            track.metadata.get("duration").and_then(|duration| duration.parse().ok()),
            fallback_duration_ms,
            catalog.and_then(|track| u32::try_from(track.duration()).ok()),
        ]
        .into_iter()
        .flatten()
        .find(|duration| *duration > 0)
        .unwrap_or_default(),
        interaction_id: Track::next_interaction_id(),
        runtime: TrackRuntime::default(),
    }
}

fn image_url(url: &str) -> String {
    url.strip_prefix("spotify:image:").map_or_else(|| url.into(), |id| format!("https://i.scdn.co/image/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::queue::{Queue, Timeline};
    use calloop::channel;

    const REQUESTED: &str = "spotify:track:6vWfZZyIlODYPMEbsrOK3G";
    const PLAYBACK: &str = "spotify:track:3OIgsG6duyvwodGPoFYVzJ";

    #[test]
    fn queue_jump_resolves_duplicate_tracks_by_occurrence() {
        let track = |uid: &str| ProvidedTrack {
            uri: PLAYBACK.into(),
            uid: uid.into(),
            metadata: HashMap::from([("requested_uri".into(), REQUESTED.into())]),
        };
        let player = PlayerState {
            track: Some(track("current")),
            prev_tracks: vec![track("previous")],
            next_tracks: vec![track("next")],
            ..Default::default()
        };
        assert!(matches!(queued_occurrence(&player, REQUESTED, "previous"), Some((_, false))));
        assert!(matches!(queued_occurrence(&player, REQUESTED, "next"), Some((_, true))));
        for uid in ["", "current", "stale"] {
            assert!(queued_occurrence(&player, REQUESTED, uid).is_none());
        }
    }

    #[test]
    fn playback_clock_only_advances_when_playing_and_ready() {
        let mut player = PlayerState {
            position_as_of_timestamp: 4_000,
            timestamp: 1_000,
            playback_speed: 1.0,
            is_playing: true,
            ..Default::default()
        };
        assert_eq!(playback_position(&player, 3_000), (6_000.0, 1.0));
        player.is_buffering = true;
        assert_eq!(playback_position(&player, 3_000), (4_000.0, 0.0));
        player.is_buffering = false;
        player.is_paused = true;
        assert_eq!(playback_position(&player, 3_000), (4_000.0, 0.0));
    }

    #[tokio::test]
    async fn trackless_cluster_does_not_clear_known_playback() {
        let (updater, _updates) = channel::channel();
        let mut worker = SpotifyWorker {
            active_device: Some("playing-device".into()),
            player: Some(PlayerState {
                track: Some(ProvidedTrack { uri: REQUESTED.into(), ..Default::default() }),
                is_playing: true,
                ..Default::default()
            }),
            ..SpotifyWorker::new(Arc::new(Session::new().unwrap()), updater)
        };
        for device in [Some("playing-device"), None] {
            worker.update_cluster(Cluster {
                active_device_id: device.unwrap_or_default().into(),
                player_state: Some(PlayerState { is_buffering: device.is_some(), ..Default::default() }),
                ..Default::default()
            });
            assert_eq!(worker.player.as_ref().unwrap().track.as_ref().unwrap().uri, REQUESTED);
            assert_eq!(worker.active_device.as_deref(), device);
        }
    }

    #[test]
    fn mpris_replaces_a_track_when_its_title_arrives() {
        let (updater, _updates) = channel::channel();
        let (mpris, playback) = watch::channel(None);
        let mut worker = SpotifyWorker {
            active_device: Some("phone".into()),
            mpris: Some(mpris),
            ..SpotifyWorker::new(Arc::new(Session::new().unwrap()), updater)
        };
        let report = |uri: &str, title: Option<&str>| PlayerState {
            track: Some(ProvidedTrack {
                uri: uri.into(),
                metadata: title.map_or_else(HashMap::new, |title| HashMap::from([("title".into(), title.into())])),
                ..Default::default()
            }),
            is_playing: true,
            ..Default::default()
        };
        worker.player = Some(report(REQUESTED, Some("First track")));
        worker.publish(true);
        assert!(playback.borrow().as_ref().unwrap().matches_track(REQUESTED, ""));

        worker.player = Some(report(PLAYBACK, None));
        worker.publish(true);
        assert!(playback.borrow().as_ref().unwrap().matches_track(REQUESTED, ""));

        worker
            .track_metadata
            .insert(PLAYBACK.into(), Some(CatalogTrack { name: Some("Second track".into()), ..Default::default() }));
        worker.publish(false);
        assert!(playback.borrow().as_ref().unwrap().matches_track(PLAYBACK, ""));
    }

    #[test]
    fn relinked_playback_keeps_rating_artwork_and_queue_occurrence() {
        let mut provided = ProvidedTrack {
            uri: REQUESTED.into(),
            uid: "921086112294dced".into(),
            metadata: HashMap::from([("requested_uri".into(), REQUESTED.into())]),
        };
        let mut queued = track_from_provided(&provided, None, Some(160_342));
        queued.image = Some("https://i.scdn.co/image/already-loaded".into());
        let interaction_id = queued.interaction_id;
        let image = queued.image.clone();
        let mut queue = Queue { tracks: vec![queued], timeline: Timeline { observed_at: Instant::now(), .. }, .. };

        provided.uri = PLAYBACK.into();
        let playing = track_from_provided(&provided, None, Some(160_342));
        assert_eq!(playing.id, queue.tracks[0].id);
        queue.replace(vec![playing], 0, 0.0);
        let playing = &queue.tracks[0];
        assert_eq!(playing.uri, REQUESTED);
        assert_eq!(playing.image, image);
        assert_eq!(playing.interaction_id, interaction_id);
    }

    #[test]
    fn missing_or_invalid_requested_uri_uses_reported_identity() {
        let mut provided = ProvidedTrack { uri: PLAYBACK.into(), ..Default::default() };
        for requested in ["", "invalid", "spotify:episode:6vWfZZyIlODYPMEbsrOK3G"] {
            provided.metadata.insert("requested_uri".into(), requested.into());
            assert_eq!(requested_track_uri(&provided), PLAYBACK);
        }
        provided.uri = "spotify:episode:6vWfZZyIlODYPMEbsrOK3G".into();
        provided.metadata.insert("requested_uri".into(), REQUESTED.into());
        assert_eq!(requested_track_uri(&provided), provided.uri);
    }
}
