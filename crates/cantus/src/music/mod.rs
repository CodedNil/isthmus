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
#[cfg(target_os = "linux")]
use zbus::OwnedObjectPath;

pub mod enrichment;
pub mod lyrics;
#[cfg(target_os = "linux")]
pub mod mpris;
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
    pub local: Option<LocalTrack> = None,
    pub(crate) spotify: spotify::Spotify,
}

pub struct LocalTrack {
    pub track: Track,
    pub timeline: Timeline,
    pub playing: bool,
    #[cfg(target_os = "linux")]
    pub source: String,
    #[cfg(target_os = "linux")]
    pub(crate) mpris_id: OwnedObjectPath,
    #[cfg(target_os = "linux")]
    pub(crate) reported_position: Option<f32>,
    pub can_seek: bool,
    pub can_toggle: bool,
}

impl Music {
    pub(crate) fn new(config: &Config, updater: &AppUpdater) -> Self {
        #[cfg(target_os = "linux")]
        mpris::start(updater.clone());
        Self {
            resources: TrackCache::default(),
            queue: Queue { timeline: Timeline { observed_at: Instant::now(), .. }, .. },
            last_toggle: Instant::now(),
            spotify: spotify::Spotify::new(config, updater),
            ..
        }
    }

    pub const fn local_foreground(&self) -> bool {
        self.local.is_some() && !self.playing
    }

    pub fn foreground_playing(&self) -> bool {
        self.playing || self.local.as_ref().is_some_and(|local| local.playing)
    }

    pub fn update_playback(&mut self, drag: Option<(u64, f32, bool)>, delta_time: f32) {
        let local_drag =
            self.local.as_ref().is_some_and(|local| drag.is_some_and(|(id, ..)| id == local.track.interaction_id));
        self.queue.update(drag.filter(|_| !local_drag).map(|(_, offset, _)| offset), delta_time);
        if let Some(local) = &mut self.local {
            let offset = drag.filter(|_| local_drag).map_or(0.0, |(_, offset, _)| offset);
            let mut position = (local.timeline.position_now() - offset).max(0.0);
            if local.track.duration_ms > 0 {
                position = position.min(local.track.duration_ms as f32);
            }
            local.timeline.update(-position, local_drag, delta_time);
        }
        if drag.is_some_and(|(_, _, released)| released) {
            let target = if local_drag {
                self.local.as_ref().map(|local| (None, -local.timeline.start_ms))
            } else {
                self.queue.span_at_playhead().map(|(index, position)| (Some(index), position))
            };
            if let Some((index, position)) = target {
                self.seek(index, position);
            }
        }
    }

    pub fn seek(&mut self, index: Option<usize>, position: f32) {
        if let Some(index) = index {
            let Some(track) = self.queue.tracks.get(index) else { return };
            let skip = (index as i64 - self.queue.index as i64).clamp(-10, 10) as i8;
            self.spotify.command(if skip == 0 {
                PlaybackCommand::Seek(position.clamp(0.0, track.duration_ms as f32).round() as u32)
            } else {
                PlaybackCommand::Skip(skip)
            });
        } else if let Some(local) = &mut self.local
            && local.can_seek
            && local.track.duration_ms > 0
        {
            let position = position.clamp(0.0, local.track.duration_ms as f32).round();
            #[cfg(target_os = "linux")]
            mpris::command(local, PlaybackCommand::Seek(position as u32));
            local.timeline.position_ms = position;
            local.timeline.observed_at = Instant::now();
        }
    }

    pub fn toggle_playing(&self) {
        if self.local_foreground()
            && let Some(local) = &self.local
        {
            if local.can_toggle {
                #[cfg(target_os = "linux")]
                mpris::command(local, PlaybackCommand::SetPlaying(!local.playing));
            }
            return;
        }
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
