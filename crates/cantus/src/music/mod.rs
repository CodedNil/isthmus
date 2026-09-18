use crate::{
    app::AppUpdater,
    config::Config,
    platform::{media_command, start_mpris},
    render::music::AudioFeatures,
};
use arrayvec::ArrayString;
use enrichment::TrackCache;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
    mem,
    sync::atomic::{AtomicU64, Ordering},
};
use tracing::{info, warn};
use web_time::Instant;

pub mod enrichment;
pub mod lyrics;
#[cfg(target_os = "linux")]
pub mod mpris;
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
    pub playing: bool,
    pub queue: Vec<Track>,
    pub playlists: Vec<CondensedPlaylist>,
    pub timeline: Timeline,
    pub last_toggle: Instant,
    pub local: Option<LocalTrack>,
    pub(crate) spotify: spotify::Spotify,
}

impl Music {
    pub(crate) fn spotify(config: &Config, updater: &AppUpdater) -> Self {
        start_mpris(updater.clone());
        Self {
            resources: TrackCache::default(),
            playing: false,
            queue: Vec::new(),
            playlists: Vec::new(),
            timeline: Timeline { observed_at: Instant::now(), .. },
            last_toggle: Instant::now(),
            local: None,
            spotify: spotify::Spotify::new(config, updater),
        }
    }
}

pub struct LocalTrack {
    pub track: Track,
    pub timeline: Timeline,
    pub playing: bool,
    pub source: String,
    pub(crate) mpris_id: String,
    #[cfg(target_os = "linux")]
    pub(crate) reported_position: Option<f32>,
    pub can_seek: bool,
    pub can_toggle: bool,
}

impl LocalTrack {
    fn command(&self, command: PlaybackCommand) {
        media_command(self.source.clone(), self.mpris_id.clone(), command);
    }
}

/// The observed and visually smoothed position of the playback queue.
pub struct Timeline {
    pub index: usize = 0,
    pub position_ms: f32 = 0.0,
    pub rate: f32 = 0.0,
    pub observed_at: Instant,
    pub queue_start_ms: f32 = 0.0,
    pub movement: f32 = 0.0,
}

impl Timeline {
    fn update(&mut self, target: f32, dragging: bool, delta_time: f32) {
        let predicted = self.queue_start_ms - self.rate * delta_time * 1000.0;
        let next = if dragging { target } else { predicted + (target - predicted) * (1.0 - (-delta_time / 0.3).exp()) };
        self.movement += ((next - self.queue_start_ms) * delta_time - self.movement) * (delta_time * 10.0).min(1.0);
        self.queue_start_ms = next;
    }

    pub fn position_now(&self) -> f32 {
        self.position_ms + self.observed_at.elapsed().as_secs_f32() * 1000.0 * self.rate
    }

    /// Returns the queue item covering the playhead, including its trailing spacing.
    pub fn span_at_playhead(&self, queue: &[Track]) -> Option<(usize, f32)> {
        let mut start_ms = self.queue_start_ms;
        queue.iter().enumerate().find_map(|(index, track)| {
            let elapsed = -start_ms;
            let current = (elapsed >= 0.0 && elapsed < track.queue_span_ms()).then_some((index, elapsed));
            start_ms += track.queue_span_ms();
            current
        })
    }
}

impl Music {
    fn observe(&mut self, index: usize, position_ms: f32, rate: f32, observed_at: Instant) {
        self.timeline.index = index.min(self.queue.len().saturating_sub(1));
        self.timeline.position_ms = position_ms;
        self.timeline.rate = rate;
        self.timeline.observed_at = observed_at;
    }

    /// Replaces an authoritative queue snapshot without moving its rendered contents.
    fn replace_queue(
        &mut self,
        mut queue: Vec<Track>,
        index: usize,
        position_ms: f32,
        rate: f32,
        observed_at: Instant,
    ) {
        let old_index = self.timeline.index.min(self.queue.len().saturating_sub(1));
        let origin = self.queue.get(old_index).map(|track| {
            let progress =
                -self.timeline.queue_start_ms - self.queue[..old_index].iter().map(Track::queue_span_ms).sum::<f32>();
            (track.uri.clone(), progress)
        });

        let mut old = HashMap::<String, VecDeque<Track>>::new();
        for track in mem::take(&mut self.queue) {
            old.entry(track.uri.clone()).or_default().push_back(track);
        }
        for track in &mut queue {
            if let Some(previous) = old.get_mut(&track.uri).and_then(VecDeque::pop_front) {
                track.interaction_id = previous.interaction_id;
                track.runtime = previous.runtime;
                if self.resources.art(previous.image.as_deref()).is_some() || track.image.is_none() {
                    track.image = previous.image;
                }
            }
        }

        let index = index.min(queue.len().saturating_sub(1));
        let rebased = origin.and_then(|(uri, progress)| {
            queue
                .iter()
                .enumerate()
                .filter(|(_, track)| track.uri == uri)
                .min_by_key(|(candidate, _)| candidate.abs_diff(index))
                .map(|(index, _)| (index, progress))
        });
        let (origin, progress) = rebased.unwrap_or((index, position_ms));
        self.timeline.queue_start_ms = -progress - queue[..origin].iter().map(Track::queue_span_ms).sum::<f32>();
        self.queue = queue;
        self.observe(index, position_ms, rate, observed_at);
    }

    pub fn update_timeline(&mut self, drag_offset_ms: f32, dragging: bool, delta_time: f32) {
        if self.queue.is_empty() {
            self.timeline.queue_start_ms = 0.0;
            self.timeline.movement = 0.0;
            return;
        }
        let index = self.timeline.index.min(self.queue.len() - 1);
        let target = -self.timeline.position_now() - self.queue[..index].iter().map(Track::queue_span_ms).sum::<f32>()
            + drag_offset_ms;
        self.timeline.update(target, dragging, delta_time);
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
        self.update_timeline(
            drag.filter(|_| !local_drag).map_or(0.0, |(_, offset, _)| offset),
            drag.is_some() && !local_drag,
            delta_time,
        );
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
                self.local.as_ref().map(|local| (None, -local.timeline.queue_start_ms))
            } else {
                self.timeline.span_at_playhead(&self.queue).map(|(index, position)| (Some(index), position))
            };
            if let Some((index, position)) = target {
                self.seek(index, position);
            }
        }
    }

    pub fn seek(&mut self, index: Option<usize>, position: f32) {
        if let Some(index) = index {
            let Some(track) = self.queue.get(index) else { return };
            let skip = (index as i64 - self.timeline.index as i64).clamp(-10, 10) as i8;
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
            local.command(PlaybackCommand::Seek(position as u32));
            local.timeline.position_ms = position;
            local.timeline.observed_at = Instant::now();
        }
    }

    pub fn toggle_playing(&self) {
        if self.local_foreground()
            && let Some(local) = &self.local
        {
            if local.can_toggle {
                local.command(PlaybackCommand::SetPlaying(!local.playing));
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
    #[serde(default)]
    pub uri: String,
    pub name: String,
    pub original_title: Option<String>,
    pub artists: Vec<String>,
    pub album: String,
    pub image: Option<String>,
    pub duration_ms: u32,
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

    pub(crate) fn is_youtube(&self) -> bool {
        self.uri.contains("youtube.com/") || self.uri.contains("youtu.be/")
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
