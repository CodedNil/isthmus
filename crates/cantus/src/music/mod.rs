use crate::{app::AppUpdater, config::Config, render::music::AudioFeatures};
use arrayvec::ArrayString;
use enrichment::TrackCache;
use queue::{Queue, Timeline};
use serde::Deserialize;
use std::{
    collections::HashSet,
    error::Error,
    sync::atomic::{AtomicU64, Ordering},
};
use tracing::{info, warn};
use web_time::Instant;

pub mod enrichment;
pub mod lyrics;
pub mod queue;
#[cfg_attr(target_arch = "wasm32", path = "web.rs")]
mod spotify;

pub type TrackId = ArrayString<22>;
pub type PlaylistId = ArrayString<22>;
pub type MusicResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
pub const ART_SIZE: u32 = 128;
pub const TRACK_SPACING_MS: f32 = 4000.0;
static NEXT_QUEUE_ID: AtomicU64 = AtomicU64::new(1);

fn audio_features_path(id: TrackId) -> String {
    format!("/audio-attributes/v1/audio-features/{id}?format=json")
}

pub struct Music {
    pub resources: TrackCache,
    pub playing: bool = false,
    pub queue: Queue,
    pub playlists: Vec<CondensedPlaylist> = Vec::new(),
    pub last_toggle: Instant,
    pub(crate) spotify: spotify::Spotify,
}

impl Music {
    pub(crate) fn new(config: &Config, updater: &AppUpdater) -> Self {
        Self {
            resources: TrackCache::default(),
            queue: Queue { timeline: Timeline { observed_at: Instant::now(), .. }, .. },
            last_toggle: Instant::now(),
            spotify: spotify::Spotify::new(config, updater),
            ..
        }
    }

    pub fn update_playback(&mut self, drag: Option<(f32, bool)>, delta_time: f32) {
        self.queue.update(drag.map(|(offset, _)| offset), delta_time);
        if drag.is_some_and(|(_, released)| released)
            && let Some((index, position)) = self.queue.span_at_playhead()
        {
            self.seek(index, position);
        }
    }

    pub fn seek(&self, index: usize, position: f32) {
        let Some(track) = self.queue.tracks.get(index) else { return };
        let skip = (index as i64 - self.queue.index as i64).clamp(-10, 10) as i8;
        self.spotify.command(if skip == 0 {
            PlaybackCommand::Seek(position.clamp(0.0, track.duration_ms as f32).round() as u32)
        } else {
            PlaybackCommand::Skip(skip)
        });
    }

    pub fn toggle_playing(&self) {
        let playing = !self.playing;
        info!("{} current track", if playing { "Playing" } else { "Pausing" });
        self.spotify.command(PlaybackCommand::SetPlaying(playing));
    }

    pub(crate) fn play_playlist(&self, id: Option<PlaylistId>) {
        self.spotify.command(PlaybackCommand::PlayPlaylist(id));
    }

    pub(crate) fn rate_track(&self, track_id: TrackId, rating: u8) {
        self.spotify.command(PlaybackCommand::UpdateLibrary {
            track_id,
            playlists: self
                .playlists
                .iter()
                .filter_map(|playlist| {
                    let add = playlist.rating_index? == rating;
                    (playlist.tracks.contains(&track_id) != add).then_some((playlist.id, add))
                })
                .collect(),
            liked: Some(rating >= 5),
        });
    }

    pub(crate) fn toggle_playlist(&self, track_id: TrackId, playlist_id: PlaylistId) {
        let Some(playlist) = self.playlists.iter().find(|playlist| playlist.id == playlist_id) else {
            warn!(%playlist_id, %track_id, "Playlist not found for track");
            return;
        };
        let add = !playlist.tracks.contains(&track_id);
        self.spotify.command(PlaybackCommand::UpdateLibrary {
            track_id,
            playlists: vec![(playlist_id, add)],
            liked: None,
        });
    }
}

#[derive(Clone, Deserialize)]
pub struct Track {
    pub id: Option<TrackId>,
    /// The recording identifier used to disambiguate lyric-provider matches.
    #[serde(default)]
    pub isrc: Option<String>,
    /// Requested recording identity, preserved when Spotify substitutes a playback alternative.
    #[serde(default)]
    pub uri: String,
    pub name: String,
    pub original_title: Option<String>,
    pub artists: Vec<String>,
    pub album: String,
    pub image: Option<String>,
    /// Zero means the duration is unknown, including live media.
    pub duration_ms: u32,
    /// Spotify identifies individual queue occurrences, including repeated recordings.
    #[serde(skip)]
    pub(crate) queue_uid: String,
    #[serde(skip, default = "Track::next_interaction_id")]
    pub(crate) interaction_id: u64,
    #[serde(skip)]
    pub runtime: TrackRuntime,
}

#[derive(Clone, Default)]
pub struct TrackRuntime {
    pub(crate) track_expansion: f32,
    /// Animated visibility of rating stars and primary playlist icons, respectively.
    pub(crate) icon_visibility: [f32; 2],
}

impl Track {
    pub fn compact_title(&self) -> &str {
        self.original_title.as_deref().filter(|title| !title.trim().is_empty()).unwrap_or(&self.name)
    }

    pub fn primary_artist(&self) -> &str {
        self.artists.first().map_or("", String::as_str)
    }

    fn next_interaction_id() -> u64 {
        NEXT_QUEUE_ID.fetch_add(1, Ordering::Relaxed)
    }

    pub fn queue_span_ms(&self) -> f32 {
        self.duration_ms as f32 + TRACK_SPACING_MS
    }
}

pub struct CondensedPlaylist {
    pub id: PlaylistId,
    pub image_url: Option<String>,
    pub tracks: HashSet<TrackId>,
    pub rating_index: Option<u8>,
}

pub enum PlaybackCommand {
    PlayPlaylist(Option<PlaylistId>),
    SetPlaying(bool),
    Seek(u32),
    Skip(i8),
    UpdateLibrary { track_id: TrackId, playlists: Vec<(PlaylistId, bool)>, liked: Option<bool> },
}
