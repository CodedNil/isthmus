use super::{
    ArtState, AudioFeatures, CondensedPlaylist, LyricSegment, MAX_PLAYLIST_TARGETS, MusicResult, PlaybackCommand,
    PlaylistId, PlaylistTracks, Track, TrackId, TrackRuntime,
};
use crate::app::{AppUpdater, Background, send_update};
use crate::config::{self, Config};
use arrayvec::ArrayVec;
use flate2::{Compression, write::GzEncoder};
use futures_util::{StreamExt, future::try_join_all};
use isthmus::FloatExt;
use librespot_core::{
    FileId, Session, SessionConfig, SpotifyId, authentication::Credentials, cache::Cache,
    dealer::protocol::Message as DealerMessage, error::ErrorKind,
};
use librespot_metadata::{Lyrics, lyrics::SyncType};
use librespot_oauth::OAuthClientBuilder;
use librespot_protocol::{
    connect::{
        Capabilities, Cluster, ClusterUpdate, Device as ConnectDevice, DeviceInfo, MemberType, PutStateReason,
        PutStateRequest,
    },
    devices::DeviceType,
    extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery},
    extension_kind::ExtensionKind,
    metadata,
    player::{ContextPlayerOptions, PlayerState, ProvidedTrack, Suppressions},
    playlist4_external::{Add, Delta, Item, ListAttributes, ListChanges, Op, Rem, SelectedListContent, op},
};
use parking_lot::Mutex;
use protobuf::{EnumOrUnknown, Message as _, MessageField};
use reqwest::{
    Method,
    header::{self, HeaderMap},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    error::Error,
    io::{self, Write},
    path::PathBuf,
    str,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::spawn_blocking;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::sleep,
};
use tracing::{error, info, warn};

const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
const RATING_PLAYLISTS: [&str; 10] = ["0.5", "1.0", "1.5", "2.0", "2.5", "3.0", "3.5", "4.0", "4.5", "5.0"];

type ClientResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
pub struct Spotify {
    events: UnboundedSender<WorkerEvent>,
    session: Arc<Mutex<Option<Session>>>,
}

enum WorkerEvent {
    Command(PlaybackCommand),
    Metadata {
        generation: u64,
        values: HashMap<String, TrackDetails>,
    },
}

impl Spotify {
    pub(super) fn new(config: &Config, updater: &AppUpdater, background: &Background) -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(None));
        let connected_session = Arc::clone(&session);
        let worker_events = events.clone();
        let updater = updater.clone();
        let playlist_targets = config.playlists.clone();
        let ratings_enabled = config.ratings_enabled;
        background.spawn(async move {
            let mut receiver = receiver;
            let mut generation = 0;
            loop {
                generation += 1;
                match connect().await {
                    Ok(spotify) => {
                        *connected_session.lock() = Some(spotify.clone());
                        if let Err(error) = run_spotify(
                            spotify,
                            &mut receiver,
                            worker_events.clone(),
                            updater.clone(),
                            playlist_targets.clone(),
                            ratings_enabled,
                            generation,
                        )
                        .await
                        {
                            error!(%error, "Spotify worker stopped");
                        }
                        *connected_session.lock() = None;
                    }
                    Err(error) => {
                        warn!(%error, "Spotify unavailable; retrying");
                    }
                }
                sleep(Duration::from_secs(5)).await;
            }
        });
        Self { events, session }
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        if self.events.send(WorkerEvent::Command(command)).is_err() {
            warn!("Discarded music command after Spotify worker stopped");
        }
    }

    pub(super) async fn lyrics(&self, track_id: TrackId) -> MusicResult<Vec<LyricSegment>> {
        let session = self
            .session
            .lock()
            .clone()
            .ok_or_else(|| io::Error::other("Spotify is not connected"))?;
        let id = SpotifyId::from_base62(&track_id)?;
        let lines = match Lyrics::get(&session, &id).await {
            Ok(lyrics) if lyrics.lyrics.sync_type == SyncType::LineSynced => lyrics.lyrics.lines,
            Ok(_) => return Ok(Vec::new()),
            Err(error) if error.kind == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        Ok(lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let start_ms = line.start_time_ms.parse().ok()?;
                let next_start_ms = lines.get(index + 1).and_then(|next| next.start_time_ms.parse().ok());
                Some(LyricSegment::line(start_ms, next_start_ms, line.words.clone()))
            })
            .collect())
    }

    pub(super) async fn audio_features(&self, track_id: TrackId) -> MusicResult<AudioFeatures> {
        #[derive(Deserialize)]
        struct Features {
            energy: f32,
            danceability: f32,
            acousticness: f32,
            tempo: f32,
            valence: f32,
            instrumentalness: f32,
        }

        let session = self
            .session
            .lock()
            .clone()
            .ok_or_else(|| io::Error::other("Spotify is not connected"))?;
        let path = format!("/audio-attributes/v1/audio-features/{track_id}?format=json");
        let features: Features = serde_json::from_slice(
            &session
                .spclient()
                .request_as_json(&Method::GET, &path, None, None)
                .await?,
        )?;
        Ok(AudioFeatures {
            energy: features.energy.saturate(),
            danceability: features.danceability.saturate(),
            acousticness: features.acousticness.saturate(),
            tempo: (features.tempo / 300.0).saturate(),
            valence: features.valence.saturate(),
            instrumentalness: features.instrumentalness.saturate(),
        })
    }
}

async fn connect() -> ClientResult<Session> {
    let (cache, cached_credentials) = spawn_blocking(|| {
        let cache = Cache::new(Some(config::directory()), None::<PathBuf>, None::<PathBuf>, None)?;
        let credentials = cache.credentials();
        Ok::<_, librespot_core::Error>((cache, credentials))
    })
    .await??;
    let credentials = if let Some(credentials) = cached_credentials {
        credentials
    } else {
        let token = OAuthClientBuilder::new(CLIENT_ID, REDIRECT_URI, vec!["streaming", "app-remote-control"])
            .open_in_browser()
            .with_custom_message("Cantus connected successfully; this tab can be closed.")
            .build()?
            .get_access_token_async()
            .await?;
        Credentials::with_access_token(token.access_token)
    };
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(credentials, true).await?;
    info!(username = %session.username(), device_id = %session.device_id(), "Authenticated Spotify session");
    Ok(session)
}

async fn run_spotify(
    session: Session,
    events: &mut UnboundedReceiver<WorkerEvent>,
    event_tx: UnboundedSender<WorkerEvent>,
    updater: AppUpdater,
    playlist_targets: ArrayVec<String, MAX_PLAYLIST_TARGETS>,
    ratings_enabled: bool,
    generation: u64,
) -> ClientResult<()> {
    let dealer = session.dealer();
    let mut connections = dealer.listen_for("hm://pusher/v1/connections", Ok)?;
    let mut clusters = dealer.listen_for(
        "hm://connect-state/v1/cluster",
        DealerMessage::from_raw::<ClusterUpdate>,
    )?;
    let mut playlist_changes = dealer.listen_for("hm://playlist/v2", |_| Ok(()))?;
    dealer.start().await?;

    let mut worker = SpotifyWorker {
        session,
        events: event_tx,
        updater,
        active_device: None,
        playlist_targets,
        playlist_revisions: HashMap::new(),
        track_metadata: HashMap::new(),
        queue: None,
        ratings_enabled,
        generation,
    };

    loop {
        tokio::select! {
            Some(event) = events.recv() => worker.event(event).await,
            Some(message) = connections.next() => match message {
                Ok(message) => {
                    if let Some(connection_id) = message.headers.iter().find_map(|(key, value)| {
                        key.eq_ignore_ascii_case("Spotify-Connection-Id").then_some(value)
                    }) {
                        worker.session.set_connection_id(connection_id);
                        match worker.register().await {
                            Ok(cluster) => worker.update_cluster(cluster),
                            Err(error) => error!(%error, "Failed to register Spotify observer"),
                        }
                        worker.refresh_playlists().await;
                    }
                }
                Err(error) => warn!(%error, "Invalid Spotify connection update"),
            },
            Some(update) = clusters.next() => match update {
                Ok(update) => worker.update_cluster(update.cluster.into_option().unwrap_or_default()),
                Err(error) => warn!(%error, "Invalid Spotify cluster update"),
            },
            Some(change) = playlist_changes.next() => match change {
                Ok(()) => worker.refresh_playlists().await,
                Err(error) => warn!(%error, "Invalid Spotify playlist update"),
            },
            else => return Ok(()),
        }
    }
}

struct SpotifyWorker {
    session: Session,
    events: UnboundedSender<WorkerEvent>,
    updater: AppUpdater,
    active_device: Option<String>,
    playlist_targets: ArrayVec<String, MAX_PLAYLIST_TARGETS>,
    playlist_revisions: HashMap<PlaylistId, Vec<u8>>,
    /// `None` marks metadata currently being fetched.
    track_metadata: HashMap<String, Option<TrackDetails>>,
    queue: Option<QueueSnapshot>,
    ratings_enabled: bool,
    generation: u64,
}

#[derive(Clone, Copy)]
struct PlaybackUpdate {
    playing: bool,
    position_ms: f32,
    rate: f32,
    observed_at: Instant,
}

struct QueueSnapshot {
    tracks: Vec<ProvidedTrack>,
    current: usize,
    current_duration_ms: Option<u32>,
    playback: PlaybackUpdate,
}

#[derive(Clone, Copy)]
enum PlayerCommand {
    Playing(bool),
    Seek(u32),
    Skip(bool),
}

impl SpotifyWorker {
    async fn event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Command(command) => self.command(command).await,
            WorkerEvent::Metadata { generation, values } if generation == self.generation => {
                self.track_metadata.retain(|_, metadata| metadata.is_some());
                if !values.is_empty() {
                    self.track_metadata
                        .extend(values.into_iter().map(|(uri, metadata)| (uri, Some(metadata))));
                    self.publish_snapshot(true);
                }
            }
            WorkerEvent::Metadata { .. } => {}
        }
    }

    async fn command(&mut self, command: PlaybackCommand) {
        match command {
            PlaybackCommand::SetPlaying(playing) => self.player_command(PlayerCommand::Playing(playing)).await,
            PlaybackCommand::Seek(position_ms) => self.player_command(PlayerCommand::Seek(position_ms)).await,
            PlaybackCommand::Skip(count) => {
                for _ in 0..count.unsigned_abs() {
                    self.player_command(PlayerCommand::Skip(count > 0)).await;
                }
            }
            PlaybackCommand::UpdateLibrary {
                track_id,
                playlists,
                liked,
            } => self.update_library(track_id, &playlists, liked).await,
        }
    }

    async fn register(&self) -> ClientResult<Cluster> {
        let request = PutStateRequest {
            device: MessageField::some(ConnectDevice {
                device_info: MessageField::some(DeviceInfo {
                    can_play: false,
                    name: "Cantus".into(),
                    capabilities: MessageField::some(Capabilities {
                        can_be_player: false,
                        is_observable: true,
                        needs_full_player_state: true,
                        hidden: true,
                        supports_gzip_pushes: true,
                        supports_playlist_v2: true,
                        supported_types: vec!["audio/track".into(), "audio/episode".into()],
                        ..Default::default()
                    }),
                    device_type: EnumOrUnknown::new(DeviceType::OBSERVER),
                    device_id: self.session.device_id().into(),
                    client_id: CLIENT_ID.into(),
                    ..Default::default()
                }),
                player_state: MessageField::some(PlayerState {
                    session_id: self.session.session_id(),
                    playback_speed: 1.0,
                    options: MessageField::some(ContextPlayerOptions::default()),
                    suppressions: MessageField::some(Suppressions::default()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            member_type: EnumOrUnknown::new(MemberType::CONNECT_STATE),
            put_state_reason: EnumOrUnknown::new(PutStateReason::NEW_DEVICE),
            client_side_timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            ..Default::default()
        };
        let bytes = self.session.spclient().put_connect_state_request(&request).await?;
        Ok(Cluster::parse_from_bytes(&bytes)?)
    }

    fn update_cluster(&mut self, cluster: Cluster) {
        let Some(player) = cluster.player_state.into_option() else {
            warn!("Spotify cluster update contained no player state");
            return;
        };
        self.active_device = (!cluster.active_device_id.is_empty()).then(|| cluster.active_device_id.clone());
        let playing = player.is_playing && !player.is_paused;
        let observed_at = Instant::now();
        let rate = player.playback_speed.max(0.0) as f32;
        let current_position = player.prev_tracks.len();
        let position = player_position(&player, rate);
        let mut provided = player.prev_tracks;
        if let Some(current) = player.track.into_option() {
            provided.push(current);
        }
        provided.extend(player.next_tracks);
        self.track_metadata
            .retain(|uri, _| provided.iter().any(|track| track.uri == *uri));
        self.schedule_metadata(&provided);
        let current_duration_ms = u32::try_from(player.duration).ok();
        let rebuild_queue = self.queue.as_ref().is_none_or(|previous| {
            previous.current != current_position
                || previous.current_duration_ms != current_duration_ms
                || previous.tracks != provided
        });
        self.queue = Some(QueueSnapshot {
            tracks: provided,
            current: current_position,
            current_duration_ms,
            playback: PlaybackUpdate {
                playing,
                position_ms: position,
                rate,
                observed_at,
            },
        });
        self.publish_snapshot(rebuild_queue);
    }

    fn publish_snapshot(&self, rebuild_queue: bool) {
        let Some(snapshot) = &self.queue else { return };
        let index = snapshot.tracks[..snapshot.current.min(snapshot.tracks.len())]
            .iter()
            .filter(|track| !track.uri.ends_with(":delimiter"))
            .count();
        let queue = rebuild_queue.then(|| {
            snapshot
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, track)| !track.uri.ends_with(":delimiter"))
                .map(|(provided_index, track)| {
                    track_from_provided(
                        track,
                        self.track_metadata.get(&track.uri).and_then(Option::as_ref),
                        snapshot
                            .current_duration_ms
                            .filter(|_| provided_index == snapshot.current),
                    )
                })
                .collect()
        });
        let playback = snapshot.playback;
        send_update(&self.updater, move |app| {
            let queue_changed = if let Some(queue) = queue {
                app.music
                    .replace_queue(queue, index, playback.position_ms, playback.rate, playback.observed_at);
                true
            } else {
                app.music
                    .observe(index, playback.position_ms, playback.rate, playback.observed_at);
                false
            };
            if playback.playing && !app.music.playing {
                app.music.last_toggle = Instant::now();
            }
            app.music.playing = playback.playing;
            if queue_changed {
                app.refresh_enrichment(true);
            }
        });
    }

    fn schedule_metadata(&mut self, tracks: &[ProvidedTrack]) {
        let requested = tracks
            .iter()
            .filter(|track| {
                track.uri.starts_with("spotify:track:")
                    && !track.metadata.contains_key("duration")
                    && !self.track_metadata.contains_key(&track.uri)
                    && {
                        self.track_metadata.insert(track.uri.clone(), None);
                        true
                    }
            })
            .cloned()
            .collect::<Vec<_>>();
        if requested.is_empty() {
            return;
        }
        let session = self.session.clone();
        let sender = self.events.clone();
        let generation = self.generation;
        tokio::spawn(async move {
            let metadata = fetch_track_metadata(&session, &requested).await;
            let _ = sender.send(WorkerEvent::Metadata {
                generation,
                values: metadata,
            });
        });
    }

    async fn player_command(&self, command: PlayerCommand) {
        let Some(target) = &self.active_device else { return };
        let (endpoint, value) = match command {
            PlayerCommand::Playing(true) => ("resume", None),
            PlayerCommand::Playing(false) => ("pause", None),
            PlayerCommand::Seek(position) => ("seek_to", Some(position)),
            PlayerCommand::Skip(true) => ("skip_next", None),
            PlayerCommand::Skip(false) => ("skip_prev", None),
        };
        let mut command = json!({
            "endpoint": endpoint,
            "options": {
                "override_restrictions": false,
                "only_for_local_device": false,
                "system_initiated": false,
            },
        });
        if let Some(value) = value {
            command["value"] = value.into();
        }
        let body = serde_json::to_vec(&json!({
            "command": command,
            "connection_type": "wlan",
            "intent_id": format!("{:032x}", fastrand::u128(..)),
        }))
        .unwrap_or_default();
        let mut compressed = GzEncoder::new(Vec::new(), Compression::fast());
        let result: ClientResult<_> = async {
            compressed.write_all(&body)?;
            let body = compressed.finish()?;
            let mut headers = HeaderMap::new();
            headers.insert("x-spotify-connection-id", self.session.connection_id().parse()?);
            headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
            headers.insert(header::CONTENT_ENCODING, "gzip".parse()?);
            let path = format!(
                "/connect-state/v1/player/command/from/{}/to/{target}",
                self.session.device_id()
            );
            Ok(self
                .session
                .spclient()
                .request(&Method::POST, &path, Some(headers), Some(&body))
                .await?)
        }
        .await;
        if let Err(error) = result {
            error!(%error, %endpoint, "Spotify player command failed");
        }
    }

    async fn request_connected_json(&self, path: &str, body: serde_json::Value) -> ClientResult<Vec<u8>> {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert("x-spotify-connection-id", self.session.connection_id().parse()?);
        let body = serde_json::to_string(&body)?;
        Ok(self
            .session
            .spclient()
            .request_as_json(&Method::POST, path, Some(headers), Some(&body))
            .await?
            .to_vec())
    }

    async fn request_connected_proto<T: protobuf::Message>(
        &self,
        method: &Method,
        path: &str,
        message: &T,
    ) -> ClientResult<Vec<u8>> {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/x-protobuf".parse()?);
        headers.insert("x-spotify-connection-id", self.session.connection_id().parse()?);
        Ok(self
            .session
            .spclient()
            .request(method, path, Some(headers), Some(&message.write_to_bytes()?))
            .await?
            .to_vec())
    }
}

impl SpotifyWorker {
    async fn update_library(&mut self, track_id: TrackId, changes: &[(PlaylistId, bool)], liked: Option<bool>) {
        let uri = format!("spotify:track:{track_id}");
        for &(playlist_id, add) in changes {
            let Some(revision) = self.playlist_revisions.get(&playlist_id).cloned() else {
                warn!(%playlist_id, "Spotify playlist is not loaded");
                continue;
            };
            let item = Item {
                uri: Some(uri.clone()),
                ..Default::default()
            };
            let operation = if add {
                Op {
                    kind: Some(op::Kind::ADD.into()),
                    add: MessageField::some(Add {
                        items: vec![item],
                        add_last: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            } else {
                Op {
                    kind: Some(op::Kind::REM.into()),
                    rem: MessageField::some(Rem {
                        items: vec![item],
                        items_as_key: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            };
            let request = ListChanges {
                base_revision: Some(revision.clone()),
                deltas: vec![Delta {
                    base_version: Some(revision),
                    ops: vec![operation],
                    ..Default::default()
                }],
                want_resulting_revisions: Some(true),
                ..Default::default()
            };
            if let Err(error) = self
                .request_connected_proto(
                    &Method::POST,
                    &format!("/playlist/v2/playlist/{playlist_id}/changes"),
                    &request,
                )
                .await
            {
                error!(%error, %playlist_id, "Failed to update Spotify playlist");
            }
        }

        if let Some(should_like) = liked {
            let body = json!({
                "username": self.session.username(),
                "set": "collection",
                "items": [{
                    "uri": uri,
                    "is_removed": !should_like,
                }],
            });
            if let Err(error) = self
                .request_connected_json("/collection/v2/write?market=from_token", body)
                .await
            {
                error!(%error, %track_id, "Failed to update Spotify library");
            }
        }

        if !changes.is_empty() || liked.is_some() {
            self.refresh_playlists().await;
        }
    }

    async fn refresh_playlists(&mut self) {
        if let Err(error) = self.load_playlists().await {
            warn!(%error, "Failed to refresh Spotify playlists");
        }
    }

    async fn load_playlists(&mut self) -> ClientResult<()> {
        let root =
            SelectedListContent::parse_from_bytes(&self.session.spclient().get_rootlist(0, Some(10_000)).await?)?;
        let requests = root
            .contents
            .get_or_default()
            .items
            .iter()
            .zip(&root.contents.get_or_default().meta_items)
            .filter_map(|(item, metadata)| {
                let id = item
                    .uri()
                    .strip_prefix("spotify:playlist:")
                    .and_then(|id| id.parse::<PlaylistId>().ok())?;
                let attributes = metadata.attributes.get_or_default();
                let name = attributes.name();
                let rating_index = RATING_PLAYLISTS
                    .iter()
                    .position(|rating| *rating == name)
                    .filter(|_| self.ratings_enabled)
                    .map(|index| index as u8);
                if !self.playlist_targets.iter().any(|target| target == name) && rating_index.is_none() {
                    return None;
                }
                Some(PlaylistRequest {
                    id,
                    revision: metadata.revision().to_vec(),
                    name: name.to_owned(),
                    image_url: playlist_image(attributes),
                    rating_index,
                })
            })
            .collect::<Vec<_>>();

        let tracks = try_join_all(
            requests
                .iter()
                .map(|request| fetch_playlist_tracks(&self.session, request.id)),
        )
        .await?;
        let mut updates = Vec::with_capacity(requests.len());
        for (request, tracks) in requests.into_iter().zip(tracks) {
            self.playlist_revisions.insert(request.id, request.revision);
            updates.push(CondensedPlaylist {
                id: request.id,
                name: request.name,
                image_url: request.image_url,
                art: ArtState::default(),
                tracks,
                rating_index: request.rating_index,
            });
        }

        send_update(&self.updater, move |app| {
            for playlist in &mut updates {
                if let Some(old) = app
                    .music
                    .playlists
                    .iter()
                    .find(|old| old.id == playlist.id && old.image_url == playlist.image_url)
                {
                    playlist.art = old.art.clone();
                }
            }
            updates.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            app.music.playlists = updates;
            app.refresh_enrichment(false);
        });
        Ok(())
    }
}

struct PlaylistRequest {
    id: PlaylistId,
    revision: Vec<u8>,
    name: String,
    image_url: Option<String>,
    rating_index: Option<u8>,
}

async fn fetch_playlist_tracks(session: &Session, id: PlaylistId) -> ClientResult<PlaylistTracks> {
    let spotify_id = SpotifyId::from_base62(&id)?;
    let playlist = SelectedListContent::parse_from_bytes(&session.spclient().get_playlist(&spotify_id).await?)?;
    Ok(Arc::new(
        playlist
            .contents
            .get_or_default()
            .items
            .iter()
            .filter_map(|item| item.uri().strip_prefix("spotify:track:")?.parse().ok())
            .collect(),
    ))
}

fn playlist_image(attributes: &ListAttributes) -> Option<String> {
    attributes
        .picture_size
        .iter()
        .rev()
        .find_map(|picture| {
            let value = picture.url();
            (!value.is_empty()).then(|| {
                value
                    .strip_prefix("spotify:image:")
                    .map_or_else(|| value.to_owned(), |id| format!("https://i.scdn.co/image/{id}"))
            })
        })
        .or_else(|| {
            let picture = attributes.picture();
            let id = str::from_utf8(picture)
                .ok()
                .and_then(|picture| picture.strip_prefix("spotify:image:"))
                .map(str::to_owned)
                .or_else(|| (picture.len() == 20).then(|| FileId::from_raw(picture).to_string()))?;
            Some(format!("https://i.scdn.co/image/{id}"))
        })
}

fn player_position(player: &PlayerState, rate: f32) -> f32 {
    let position = player.position_as_of_timestamp.max(0) as f64;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let age_ms = now.saturating_sub(player.timestamp);
    (position + age_ms as f64 * f64::from(rate)) as f32
}

async fn fetch_track_metadata(session: &Session, tracks: &[ProvidedTrack]) -> HashMap<String, TrackDetails> {
    let entity_request = tracks
        .iter()
        .filter(|track| track.uri.starts_with("spotify:track:") && !track.metadata.contains_key("duration"))
        .map(|track| EntityRequest {
            entity_uri: track.uri.clone(),
            query: vec![ExtensionQuery {
                extension_kind: EnumOrUnknown::new(ExtensionKind::TRACK_V4),
                ..Default::default()
            }],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    if entity_request.is_empty() {
        return HashMap::new();
    }
    let request = BatchedEntityRequest {
        entity_request,
        ..Default::default()
    };
    let Ok(response) = session.spclient().get_extended_metadata(request).await else {
        warn!("Failed to fetch Spotify track metadata");
        return HashMap::new();
    };
    response
        .extended_metadata
        .into_iter()
        .filter(|array| array.extension_kind == EnumOrUnknown::new(ExtensionKind::TRACK_V4))
        .flat_map(|array| array.extension_data)
        .filter_map(|data| {
            let bytes = data.extension_data.into_option()?.value;
            Some((
                data.entity_uri,
                TrackDetails::from_spotify(&metadata::Track::parse_from_bytes(&bytes).ok()?),
            ))
        })
        .collect()
}

struct TrackDetails {
    name: String,
    artist: String,
    album: String,
    image: Option<String>,
    duration_ms: u32,
}

impl TrackDetails {
    fn from_spotify(track: &metadata::Track) -> Self {
        Self {
            name: track.name().to_owned(),
            artist: track
                .artist
                .first()
                .map_or_else(String::new, |artist| artist.name().to_owned()),
            album: track.album.get_or_default().name().to_owned(),
            image: track_image_url(track),
            duration_ms: u32::try_from(track.duration()).unwrap_or_default(),
        }
    }
}

fn track_from_provided(
    track: &ProvidedTrack,
    track_metadata: Option<&TrackDetails>,
    fallback_duration_ms: Option<u32>,
) -> Track {
    let metadata = &track.metadata;
    let text = |key, fallback: fn(&TrackDetails) -> &String| {
        metadata
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
            .or_else(|| track_metadata.map(fallback).cloned())
            .unwrap_or_default()
    };
    Track {
        id: track.uri.strip_prefix("spotify:track:").and_then(|id| id.parse().ok()),
        uri: track.uri.clone(),
        name: text("title", |details| &details.name),
        artist: text("artist_name", |details| &details.artist),
        album: text("album_title", |details| &details.album),
        image: ["image_xlarge_url", "image_large_url", "image_url"]
            .into_iter()
            .find_map(|key| metadata.get(key))
            .map(|url| {
                url.strip_prefix("spotify:image:")
                    .map_or_else(|| url.clone(), |id| format!("https://i.scdn.co/image/{id}"))
            })
            .or_else(|| track_metadata.and_then(|details| details.image.clone())),
        duration_ms: metadata
            .get("duration")
            .and_then(|duration| duration.parse().ok())
            .or(fallback_duration_ms)
            .or_else(|| track_metadata.map(|details| details.duration_ms))
            .unwrap_or_default(),
        interaction_id: Track::next_interaction_id(),
        runtime: TrackRuntime::default(),
    }
}

fn track_image_url(track: &metadata::Track) -> Option<String> {
    let album = track.album.as_ref()?;
    let image = album
        .cover_group
        .as_ref()
        .into_iter()
        .flat_map(|group| &group.image)
        .chain(&album.cover)
        .max_by_key(|image| image.width());
    let image = image?;
    (!image.file_id().is_empty()).then(|| format!("https://i.scdn.co/image/{}", FileId::from(image)))
}
