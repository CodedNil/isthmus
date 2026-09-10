use super::{
    ART_SIZE, CondensedPlaylist, MusicResult, PlaybackCommand, PlaylistId, Track, TrackId, TrackRuntime,
    lyrics::{Channel, LyricSegment, Lyrics as TrackLyrics},
};
use crate::{
    app::{AppUpdater, send_update},
    config::{self, Config, MAX_PLAYLIST_TARGETS},
    platform,
};
use arrayvec::ArrayVec;
use flate2::{Compression, write::GzEncoder};
use futures_util::{
    StreamExt,
    future::{BoxFuture, try_join_all},
    stream::FuturesUnordered,
};
use librespot_core::{
    FileId, Session, SessionConfig, SpotifyId, SpotifyUri, authentication::Credentials, cache::Cache,
    dealer::protocol::Message as DealerMessage, error::ErrorKind,
};
use librespot_metadata::{Lyrics, Metadata as _, Playlist, Track as CatalogTrack, lyrics::SyncType};
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
    playlist4_external::{
        Add, Delta, Item, ListAttributes, ListChanges, Op, PictureSize, Rem, SelectedListContent, op,
    },
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use reqwest::{
    Method,
    header::{self, HeaderMap},
};
use serde_json::json;
use std::{
    collections::{HashMap, hash_map::Entry},
    io,
    path::PathBuf,
    str,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        watch,
    },
    task::spawn_blocking,
    time::sleep,
};
use tracing::{error, info, warn};
use web_time::Instant;

const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
const RATING_PLAYLISTS: [&str; 10] = ["0.5", "1.0", "1.5", "2.0", "2.5", "3.0", "3.5", "4.0", "4.5", "5.0"];

#[derive(Clone)]
pub struct Spotify {
    commands: UnboundedSender<PlaybackCommand>,
    session: watch::Receiver<Option<Session>>,
}

type Metadata = HashMap<String, CatalogTrack>;

impl Spotify {
    pub(super) fn new(config: &Config, updater: &AppUpdater) -> Self {
        let (commands, mut receiver) = mpsc::unbounded_channel();
        let (connected_session, session) = watch::channel(None);
        let updater = updater.clone();
        let playlist_targets = config.playlists.clone();
        let ratings_enabled = config.ratings_enabled;
        platform::spawn_task(async move {
            let reconnect = async {
                loop {
                    let result = async {
                        let session = connect().await?;
                        connected_session.send_replace(Some(session.clone()));
                        run_spotify(session, &mut receiver, updater.clone(), playlist_targets.clone(), ratings_enabled)
                            .await
                    }
                    .await;
                    connected_session.send_replace(None);
                    if let Err(error) = result {
                        warn!(%error, "Spotify unavailable; retrying");
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

    pub(super) async fn lyrics(&self, track_id: TrackId) -> MusicResult<TrackLyrics> {
        let session = self.session.borrow().clone().ok_or_else(|| io::Error::other("Spotify is not connected"))?;
        let id = SpotifyId::from_base62(&track_id)?;
        let lines = match Lyrics::get(&session, &id).await {
            Ok(lyrics) if lyrics.lyrics.sync_type == SyncType::LineSynced => lyrics.lyrics.lines,
            Ok(_) => return Ok(TrackLyrics::default()),
            Err(error) if error.kind == ErrorKind::NotFound => return Ok(TrackLyrics::default()),
            Err(error) => return Err(error.into()),
        };
        let segments = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let start_ms: f32 = line.start_time_ms.parse().ok()?;
                let next_start_ms = lines.get(index + 1).and_then(|next| next.start_time_ms.parse().ok());
                let estimated_end = start_ms + line.words.chars().count().max(10) as f32 * 100.0;
                Some(LyricSegment {
                    time: start_ms..next_start_ms.map_or(estimated_end, |next| estimated_end.min(next)),
                    text: format!("{} ", line.words),
                })
            })
            .collect();
        let mut lyrics = TrackLyrics::default();
        lyrics.channels.push(Channel { segments, ..Default::default() });
        Ok(lyrics)
    }

    pub(super) async fn get_json(&self, path: &str) -> MusicResult<impl AsRef<[u8]>> {
        let session = self.session.borrow().clone().ok_or_else(|| io::Error::other("Spotify is not connected"))?;
        Ok(session.spclient().request_as_json(&Method::GET, path, None, None).await?)
    }
}

async fn connect() -> MusicResult<Session> {
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
    commands: &mut UnboundedReceiver<PlaybackCommand>,
    updater: AppUpdater,
    playlist_targets: ArrayVec<String, MAX_PLAYLIST_TARGETS>,
    ratings_enabled: bool,
) -> MusicResult<()> {
    let dealer = session.dealer();
    let mut connections = dealer.listen_for("hm://pusher/v1/connections", Ok)?;
    let mut clusters = dealer.listen_for("hm://connect-state/v1/cluster", DealerMessage::from_raw::<ClusterUpdate>)?;
    let mut playlist_changes = dealer.listen_for("hm://playlist/v2", |_| Ok(()))?;
    dealer.start().await?;

    let mut worker = SpotifyWorker {
        session,
        updater,
        active_device: None,
        playlist_targets,
        playlist_revisions: HashMap::new(),
        ratings_enabled,
        track_metadata: HashMap::new(),
        player: None,
        pending: FuturesUnordered::new(),
    };

    loop {
        tokio::select! {
            Some(command) = commands.recv() => worker.command(command).await,
            Some((requested, mut values)) = worker.pending.next() => {
                    let mut changed = false;
                    for uri in requested {
                        if let Entry::Occupied(mut slot) = worker.track_metadata.entry(uri.clone()) {
                            if let Some(value) = values.remove(&uri) {
                                slot.insert(Some(value));
                                changed = true;
                            } else if slot.get().is_none() {
                                slot.remove();
                            }
                        }
                    }
                    if changed {
                        worker.publish_snapshot(true);
                    }
            },
            Some(message) = connections.next() => match message {
                Ok(message) => {
                    if let Some(connection_id) = message.headers.iter().find_map(|(key, value)| {
                        key.eq_ignore_ascii_case("Spotify-Connection-Id").then_some(value)
                    }) {
                        worker.session.set_connection_id(connection_id);
                        match SpotifyWorker::register(&worker.session).await {
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
    pending: FuturesUnordered<BoxFuture<'static, (Vec<String>, Metadata)>>,
    updater: AppUpdater,
    active_device: Option<String>,
    playlist_targets: ArrayVec<String, MAX_PLAYLIST_TARGETS>,
    playlist_revisions: HashMap<PlaylistId, Vec<u8>>,
    ratings_enabled: bool,
    /// `None` marks metadata currently being fetched.
    track_metadata: HashMap<String, Option<CatalogTrack>>,
    player: Option<PlayerState>,
}

impl SpotifyWorker {
    async fn command(&mut self, command: PlaybackCommand) {
        let mut count = 1;
        let mut command = match command {
            PlaybackCommand::PlayPlaylist(id) => {
                let uri = id.map_or_else(
                    || format!("spotify:user:{}:collection", self.session.username()),
                    |id| format!("spotify:playlist:{id}"),
                );
                json!({
                    "endpoint": "play",
                    "context": { "uri": uri, "url": format!("context://{uri}") },
                    "play_origin": { "feature_identifier": "cantus" },
                    "options": {
                        "initially_paused": false,
                        "player_options_override": { "shuffling_context": true },
                    },
                })
            }
            PlaybackCommand::SetPlaying(playing) => {
                json!({ "endpoint": if playing { "resume" } else { "pause" } })
            }
            PlaybackCommand::Seek(position_ms) => json!({ "endpoint": "seek_to", "value": position_ms }),
            PlaybackCommand::Skip(offset) => {
                count = offset.unsigned_abs();
                json!({ "endpoint": if offset > 0 { "skip_next" } else { "skip_prev" } })
            }
            PlaybackCommand::UpdateLibrary { track_id, playlists, liked } => {
                if let Err(error) = self.update_library(track_id, &playlists, liked).await {
                    warn!(%error, "Spotify library update failed");
                }
                return;
            }
        };
        for option in ["override_restrictions", "only_for_local_device", "system_initiated"] {
            command["options"][option] = false.into();
        }
        command["logging_params"] = json!({});
        let Some(target) = &self.active_device else { return };
        let path = format!("/connect-state/v1/player/command/from/{}/to/{target}", self.session.device_id());
        for _ in 0..count {
            let result: MusicResult<_> = async {
                let mut compressed = GzEncoder::new(Vec::new(), Compression::fast());
                serde_json::to_writer(
                    &mut compressed,
                    &json!({
                        "command": command,
                        "connection_type": "wlan",
                        "intent_id": format!("{:032x}", fastrand::u128(..)),
                    }),
                )?;
                connected_request(&self.session, &path, Some("gzip"), &compressed.finish()?).await
            }
            .await;
            if let Err(error) = result {
                error!(%error, endpoint = %command["endpoint"], "Spotify player command failed");
            }
        }
    }

    async fn register(session: &Session) -> MusicResult<Cluster> {
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
                    device_id: session.device_id().into(),
                    client_id: CLIENT_ID.into(),
                    ..Default::default()
                }),
                player_state: MessageField::some(PlayerState {
                    session_id: session.session_id(),
                    playback_speed: 1.0,
                    options: MessageField::some(ContextPlayerOptions::default()),
                    suppressions: MessageField::some(Suppressions::default()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            member_type: EnumOrUnknown::new(MemberType::CONNECT_STATE),
            put_state_reason: EnumOrUnknown::new(PutStateReason::NEW_DEVICE),
            client_side_timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            ..Default::default()
        };
        let bytes = session.spclient().put_connect_state_request(&request).await?;
        Ok(Cluster::parse_from_bytes(&bytes)?)
    }

    fn update_cluster(&mut self, cluster: Cluster) {
        let Some(player) = cluster.player_state.into_option() else {
            warn!("Spotify cluster update contained no player state");
            return;
        };
        self.active_device = (!cluster.active_device_id.is_empty()).then_some(cluster.active_device_id);
        let tracks = player.prev_tracks.iter().chain(player.track.as_ref()).chain(&player.next_tracks);
        self.track_metadata.retain(|uri, _| tracks.clone().any(|track| track.uri == *uri));
        self.schedule_metadata(tracks);
        let rebuild_queue = self.player.as_ref().is_none_or(|previous| {
            previous.prev_tracks != player.prev_tracks
                || previous.track != player.track
                || previous.next_tracks != player.next_tracks
                || previous.duration != player.duration
        });
        self.player = Some(player);
        self.publish_snapshot(rebuild_queue);
    }

    fn publish_snapshot(&self, rebuild_queue: bool) {
        let Some(player) = &self.player else { return };
        let index = player.prev_tracks.iter().filter(|track| !track.uri.ends_with(":delimiter")).count();
        let queue = rebuild_queue.then(|| {
            player
                .prev_tracks
                .iter()
                .chain(player.track.as_ref())
                .chain(&player.next_tracks)
                .enumerate()
                .filter(|(_, track)| !track.uri.ends_with(":delimiter"))
                .map(|(provided_index, track)| {
                    track_from_provided(
                        track,
                        self.track_metadata.get(&track.uri).and_then(Option::as_ref),
                        u32::try_from(player.duration).ok().filter(|_| provided_index == player.prev_tracks.len()),
                    )
                })
                .collect()
        });
        let playing = player.is_playing && !player.is_paused;
        let rate = if playing { player.playback_speed.max(0.0) as f32 } else { 0.0 };
        let observed_at = Instant::now();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
        let age_ms = now.saturating_sub(player.timestamp).max(0);
        let position_ms = (player.position_as_of_timestamp.max(0) as f64 + age_ms as f64 * f64::from(rate)) as f32;
        send_update(&self.updater, move |app| {
            if let Some(queue) = queue {
                app.music.replace_queue(queue, index, position_ms, rate, observed_at);
            } else {
                app.music.observe(index, position_ms, rate, observed_at);
            }
            if playing && !app.music.playing {
                app.music.last_toggle = Instant::now();
            }
            app.music.playing = playing;
            if rebuild_queue {
                app.refresh_enrichment();
            }
        });
    }

    fn schedule_metadata<'a>(&mut self, tracks: impl Iterator<Item = &'a ProvidedTrack>) {
        let requested = tracks
            .filter(|track| {
                matches!(SpotifyUri::from_uri(&track.uri), Ok(SpotifyUri::Track { .. }))
                    && !self.track_metadata.contains_key(&track.uri)
                    && {
                        self.track_metadata.insert(track.uri.clone(), None);
                        true
                    }
            })
            .map(|track| track.uri.clone())
            .collect::<Vec<_>>();
        if requested.is_empty() {
            return;
        }
        let session = self.session.clone();
        self.pending.push(Box::pin(async move {
            let metadata = fetch_track_metadata(&session, &requested).await;
            (requested, metadata)
        }));
    }
}

async fn connected_request(
    session: &Session,
    path: &str,
    encoding: Option<&'static str>,
    body: &[u8],
) -> MusicResult<()> {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/json"));
    headers.insert("x-spotify-connection-id", session.connection_id().parse()?);
    if let Some(encoding) = encoding {
        headers.insert(header::CONTENT_ENCODING, encoding.parse()?);
    }
    session.spclient().request(&Method::POST, path, Some(headers), Some(body)).await?;
    Ok(())
}

impl SpotifyWorker {
    async fn refresh_playlists(&mut self) {
        if let Err(error) = self.load_playlists().await {
            warn!(%error, "Spotify library refresh failed");
        }
    }

    async fn update_library(
        &mut self,
        track_id: TrackId,
        playlists: &[(PlaylistId, bool)],
        liked: Option<bool>,
    ) -> MusicResult<()> {
        let uri = SpotifyUri::Track { id: SpotifyId::from_base62(&track_id)? }.to_uri()?;
        for &(playlist_id, add) in playlists {
            let Some(revision) = self.playlist_revisions.get(&playlist_id).cloned() else {
                warn!(%playlist_id, "Spotify playlist is not loaded");
                continue;
            };
            let item = Item { uri: Some(uri.clone()), ..Default::default() };
            let operation = if add {
                Op {
                    kind: Some(op::Kind::ADD.into()),
                    add: MessageField::some(Add { items: vec![item], add_last: Some(true), ..Default::default() }),
                    ..Default::default()
                }
            } else {
                Op {
                    kind: Some(op::Kind::REM.into()),
                    rem: MessageField::some(Rem { items: vec![item], items_as_key: Some(true), ..Default::default() }),
                    ..Default::default()
                }
            };
            let request = ListChanges {
                base_revision: Some(revision.clone()),
                deltas: vec![Delta { base_version: Some(revision), ops: vec![operation], ..Default::default() }],
                want_resulting_revisions: Some(true),
                ..Default::default()
            };
            let mut headers = HeaderMap::new();
            headers.insert("x-spotify-connection-id", self.session.connection_id().parse()?);
            if let Err(error) = self
                .session
                .spclient()
                .request_with_protobuf(
                    &Method::POST,
                    &format!("/playlist/v2/playlist/{playlist_id}/changes"),
                    Some(headers),
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
            if let Err(error) = connected_request(
                &self.session,
                "/collection/v2/write?market=from_token",
                None,
                &serde_json::to_vec(&body)?,
            )
            .await
            {
                error!(%error, %track_id, "Failed to update Spotify library");
            }
        }

        self.load_playlists().await
    }

    async fn load_playlists(&mut self) -> MusicResult<()> {
        let root =
            SelectedListContent::parse_from_bytes(&self.session.spclient().get_rootlist(0, Some(10_000)).await?)?;
        let requests =
            root.contents.get_or_default().items.iter().zip(&root.contents.get_or_default().meta_items).filter_map(
                |(item, metadata)| {
                    let SpotifyUri::Playlist { id, .. } = SpotifyUri::from_uri(item.uri()).ok()? else {
                        return None;
                    };
                    let id = id.to_base62().ok()?.parse::<PlaylistId>().ok()?;
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
                    let session = &self.session;
                    Some(async move {
                        let playlist = Playlist::get(session, &SpotifyUri::Playlist {
                            id: SpotifyId::from_base62(&id)?,
                            user: None,
                        })
                        .await?;
                        MusicResult::Ok((
                            CondensedPlaylist {
                                id,
                                image_url: playlist_image(attributes),
                                tracks: playlist
                                    .tracks()
                                    .filter_map(|uri| match uri {
                                        SpotifyUri::Track { id } => id.to_base62().ok()?.parse().ok(),
                                        _ => None,
                                    })
                                    .collect(),
                                rating_index,
                            },
                            metadata.revision().to_vec(),
                            name,
                        ))
                    })
                },
            );
        let mut updates = Vec::new();
        let mut playlists = try_join_all(requests).await?;
        playlists.sort_unstable_by_key(|(_, _, name)| *name);
        self.playlist_revisions.clear();
        for (playlist, revision, _) in playlists {
            self.playlist_revisions.insert(playlist.id, revision);
            updates.push(playlist);
        }

        send_update(&self.updater, move |app| {
            app.music.playlists = updates;
            app.refresh_enrichment();
        });
        Ok(())
    }
}

fn playlist_image(attributes: &ListAttributes) -> Option<String> {
    attributes.picture_size.iter().rev().map(PictureSize::url).find(|url| !url.is_empty()).map(image_url).or_else(
        || {
            let picture = attributes.picture();
            let id = str::from_utf8(picture)
                .ok()
                .and_then(|picture| picture.strip_prefix("spotify:image:"))
                .map(str::to_owned)
                .or_else(|| (picture.len() == 20).then(|| FileId::from_raw(picture).to_string()))?;
            Some(format!("https://i.scdn.co/image/{id}"))
        },
    )
}

async fn fetch_track_metadata(session: &Session, tracks: &[String]) -> Metadata {
    let entity_request = tracks
        .iter()
        .map(|uri| EntityRequest {
            entity_uri: uri.clone(),
            query: vec![ExtensionQuery {
                extension_kind: EnumOrUnknown::new(ExtensionKind::TRACK_V4),
                ..Default::default()
            }],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let request = BatchedEntityRequest { entity_request, ..Default::default() };
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
            let message = metadata::Track::parse_from_bytes(&bytes).ok()?;
            match CatalogTrack::try_from(&message) {
                Ok(mut track) => {
                    if track.album.covers.is_empty() {
                        track.album.covers = message.album.get_or_default().cover.as_slice().into();
                    }
                    Some((data.entity_uri, track))
                }
                Err(error) => {
                    warn!(%error, uri = %data.entity_uri, "Invalid Spotify track metadata");
                    None
                }
            }
        })
        .collect()
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
        .flat_map(|track| track.artists.iter())
        .map(|artist| &artist.name)
        .filter(|name| !name.is_empty())
        .cloned()
        .collect();
    if artists.is_empty() {
        artists.extend(track.metadata.get("artist_name").filter(|s| !s.is_empty()).cloned());
    }
    Track {
        id: match SpotifyUri::from_uri(&track.uri) {
            Ok(SpotifyUri::Track { id }) => id.to_base62().ok().and_then(|id| id.parse().ok()),
            _ => None,
        },
        uri: track.uri.clone(),
        original_title: catalog.map(|track| track.original_title.clone()),
        name: catalog
            .map(|track| &track.name)
            .filter(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| text("title", "")),
        artists,
        album: text("album_title", catalog.map_or("", |track| track.album.name.as_str())),
        image: ["image_url", "image_large_url", "image_xlarge_url"]
            .into_iter()
            .find_map(|key| track.metadata.get(key).filter(|url| !url.is_empty()))
            .map(|url| image_url(url))
            .or_else(|| {
                let album = &catalog?.album;
                let image = album
                    .cover_group
                    .iter()
                    .chain(album.covers.iter())
                    .min_by_key(|image| image.width.abs_diff(ART_SIZE as i32))?;
                Some(format!("https://i.scdn.co/image/{}", image.id))
            }),
        duration_ms: track
            .metadata
            .get("duration")
            .and_then(|duration| duration.parse().ok())
            .or(fallback_duration_ms)
            .or_else(|| catalog.and_then(|track| u32::try_from(track.duration).ok()))
            .unwrap_or_default(),
        interaction_id: Track::next_interaction_id(),
        runtime: TrackRuntime::default(),
    }
}

fn image_url(url: &str) -> String {
    url.strip_prefix("spotify:image:").map_or_else(|| url.into(), |id| format!("https://i.scdn.co/image/{id}"))
}
