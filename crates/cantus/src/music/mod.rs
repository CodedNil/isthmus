use crate::{
    app::Background,
    config::Config,
    render::music::{AudioFeatures, DropletMotion},
};
use arrayvec::ArrayString;
use enrichment::TrackCache;
use isthmus_2d::Ramp;
use queue::{Queue, Timeline};
use std::{
    collections::HashSet,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
use tracing::{info, warn};

pub mod enrichment;
pub mod lyrics;
pub mod queue;
mod spotify;

pub type TrackId = ArrayString<22>;
pub type PlaylistId = ArrayString<22>;
pub type MusicResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
pub const ART_SIZE: u32 = 128;
pub const TRACK_SPACING_MS: f32 = 3000.0;
const SCROLL_RESTART_AFTER_MS: f32 = 10_000.0;
pub const TRACK_RESTART_GRACE: Duration = Duration::from_secs(2);
static NEXT_QUEUE_ID: AtomicU64 = AtomicU64::new(1);

pub struct Music {
    pub resources: TrackCache,
    pub playing: bool = false,
    pub queue: Queue,
    pub playlists: Vec<CondensedPlaylist> = Vec::new(),
    pub last_toggle: Instant,
    scroll_restart: Option<(u64, Instant)> = None,
    pub(crate) spotify: spotify::Spotify,
}

impl Music {
    pub(crate) fn new(config: &Config, background: &Background, local_playback: watch::Receiver<bool>) -> Self {
        Self {
            resources: TrackCache::default(),
            queue: Queue { timeline: Timeline { observed_at: Instant::now(), .. }, .. },
            last_toggle: Instant::now(),
            spotify: spotify::Spotify::new(config, background, local_playback),
            ..
        }
    }

    /// `drag` is pointer and scroll drag in milliseconds, and whether it just ended.
    pub fn update_playback(&mut self, drag: Option<(f32, f32, bool)>) {
        self.queue.update(drag.map(|(pointer, scroll, _)| (pointer, scroll)));
        if drag.is_some_and(|(.., released)| released)
            && let Some((index, position)) = self.queue.span_at_playhead()
        {
            self.seek(index, position);
        }
    }

    pub fn seek(&self, index: usize, position: f32) {
        let Some(track) = self.queue.tracks.get(index) else { return };
        self.spotify.command(if index == self.queue.index {
            PlaybackCommand::Seek(position.clamp(0.0, track.duration_ms as f32).round() as u32)
        } else {
            PlaybackCommand::JumpTo { uri: track.uri.clone(), uid: track.queue_uid.clone() }
        });
    }

    /// Navigates from the actual playing track, even while the visible timeline is catching up.
    pub(crate) fn scroll_tracks(&mut self, notches: i32) {
        let index = self.queue.index;
        let Some(track) = self.queue.tracks.get(index) else {
            self.scroll_restart = None;
            return;
        };
        let now = Instant::now();
        let recently_restarted =
            self.scroll_restart.is_some_and(|(id, at)| id == track.interaction_id && now - at < TRACK_RESTART_GRACE);
        let (restart, skip) = scroll_playback(
            notches,
            self.queue.timeline.position_now(),
            recently_restarted,
            index,
            self.queue.tracks.len() - index - 1,
        );
        if restart {
            self.scroll_restart = Some((track.interaction_id, now));
        } else if notches > 0 || !recently_restarted {
            self.scroll_restart = None;
        }
        match skip {
            0 if restart => self.seek(index, 0.0),
            0 => {}
            -1 | 1 => self.spotify.command(PlaybackCommand::Skip { forward: skip > 0 }),
            _ => self.seek(index.saturating_add_signed(skip as isize), 0.0),
        }
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

pub fn should_restart_track(position_ms: f32, recently_restarted: bool) -> bool {
    position_ms > SCROLL_RESTART_AFTER_MS && !recently_restarted
}

fn scroll_playback(
    notches: i32,
    position_ms: f32,
    recently_restarted: bool,
    previous: usize,
    next: usize,
) -> (bool, i32) {
    let restart = notches < 0 && should_restart_track(position_ms, recently_restarted);
    let skip = (notches + i32::from(restart))
        .clamp(-i32::try_from(previous).unwrap_or(i32::MAX), i32::try_from(next).unwrap_or(i32::MAX));
    (restart, skip)
}

#[cfg(test)]
mod tests {
    use super::scroll_playback;

    #[test]
    fn wheel_scroll_restarts_then_skips_back() {
        assert_eq!(scroll_playback(1, 15_000.0, false, 2, 2), (false, 1));
        assert_eq!(scroll_playback(-1, 10_000.0, false, 2, 2), (false, -1));
        assert_eq!(scroll_playback(-1, 15_000.0, false, 2, 2), (true, 0));
        assert_eq!(scroll_playback(-1, 15_000.0, true, 2, 2), (false, -1));
        assert_eq!(scroll_playback(-2, 15_000.0, false, 2, 2), (true, -1));
        assert_eq!(scroll_playback(-1, 15_000.0, false, 0, 2), (true, 0));
        assert_eq!(scroll_playback(-1, 0.0, false, 0, 2), (false, 0));
        assert_eq!(scroll_playback(1, 0.0, false, 2, 0), (false, 0));
        assert_eq!(scroll_playback(25, 0.0, false, 2, 30), (false, 25));
    }
}

#[derive(Clone)]
pub struct Track {
    /// Spotify track ID used for playback and library requests.
    pub id: Option<TrackId>,
    /// Recording identifier shared by providers that expose an ISRC.
    pub isrc: Option<String>,
    /// The requested recording, kept when Spotify plays an alternative.
    pub uri: String,
    pub name: String,
    pub original_title: Option<String>,
    pub artists: Vec<String>,
    pub album: String,
    pub image: Option<String>,
    /// Zero means the duration is unknown, including live media.
    pub duration_ms: u32,
    /// Spotify's id for this queue entry, unique even for repeats.
    pub(crate) queue_uid: String,
    pub(crate) interaction_id: u64,
    pub runtime: TrackRuntime,
}

#[derive(Clone, Default)]
pub struct TrackRuntime {
    pub(crate) track_expansion: Ramp,
    /// Animated visibility of rating stars and primary playlist icons, respectively.
    pub(crate) icon_visibility: [Ramp; 2],
    pub(crate) droplet_motion: DropletMotion,
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
    pub tracks: Arc<HashSet<TrackId>>,
    pub rating_index: Option<u8>,
}

pub enum PlaybackCommand {
    PlayPlaylist(Option<PlaylistId>),
    SetPlaying(bool),
    Seek(u32),
    Skip { forward: bool },
    JumpTo { uri: String, uid: String },
    UpdateLibrary { track_id: TrackId, playlists: Vec<(PlaylistId, bool)>, liked: Option<bool> },
}
