use super::{AudioFeatures, MusicResult, PlaybackCommand, Track, TrackId, TrackRuntime, lyrics::LyricSegment};
use crate::{
    app::{AppUpdater, send_update},
    config::Config,
};
use std::{
    collections::HashMap,
    future::{Ready, ready},
    sync::Arc,
};
use web_time::Instant;

#[derive(Clone)]
pub struct Spotify {
    updater: AppUpdater,
    audio: Arc<HashMap<TrackId, AudioFeatures>>,
}

impl Spotify {
    pub(super) fn new(_config: &Config, updater: &AppUpdater) -> Self {
        let tracks: Vec<DemoTrack> = serde_json::from_str(include_str!("../../../../assets/web/tracks.json"))
            .unwrap_or_else(|error| {
                tracing::error!(%error, "Invalid bundled demo tracks");
                Vec::new()
            });
        let mut audio = HashMap::new();
        let mut queue = Vec::new();
        for mut track in tracks {
            track.audio.tempo /= 300.0;
            audio.insert(track.id, track.audio);
            queue.push(Track {
                id: Some(track.id),
                uri: format!("spotify:track:{}", track.id),
                name: track.name,
                artist: track.artist,
                album: track.album,
                image: Some(track.image),
                duration_ms: track.duration_ms,
                interaction_id: Track::next_interaction_id(),
                runtime: TrackRuntime::default(),
            });
        }
        fastrand::shuffle(&mut queue);
        send_update(updater, move |app| {
            app.music.replace_queue(queue, 3, 42_000.0, 1.0, Instant::now());
            app.music.playing = true;
        });
        Self { updater: updater.clone(), audio: Arc::new(audio) }
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        send_update(&self.updater, move |app| {
            let music = &mut app.music;
            let mut index = music.timeline.index;
            let mut position = music.timeline.position_now();
            match command {
                PlaybackCommand::SetPlaying(playing) => music.playing = playing,
                PlaybackCommand::Seek(milliseconds) => position = milliseconds as f32,
                PlaybackCommand::Skip(offset) => {
                    index = index.saturating_add_signed(isize::from(offset)).min(music.queue.len().saturating_sub(1));
                    position = 0.0;
                }
                PlaybackCommand::UpdateLibrary { track_id, playlists, liked } => {
                    for (id, include) in playlists {
                        if let Some(playlist) = music.playlists.iter_mut().find(|playlist| playlist.id == id) {
                            if include {
                                playlist.tracks.insert(track_id);
                            } else {
                                playlist.tracks.remove(&track_id);
                            }
                        }
                    }
                    tracing::debug!(%track_id, ?liked, "Browser demo does not persist Spotify library changes");
                }
            }
            let duration = music.queue.get(index).map_or(0.0, |track| track.duration_ms as f32);
            music.observe(index, position.clamp(0.0, duration), f32::from(music.playing), Instant::now());
        });
    }

    pub(super) fn lyrics(&self, _track_id: TrackId) -> Ready<MusicResult<Vec<LyricSegment>>> {
        ready(Ok(Vec::new()))
    }

    pub(super) fn audio_features(&self, track_id: TrackId) -> Ready<MusicResult<AudioFeatures>> {
        ready(self.audio.get(&track_id).copied().ok_or_else(|| "Unknown demo track".into()))
    }
}

// Apple catalog metadata and Spotify audio features captured on 2026-09-09.
// Tempo stays in BPM in the fixture and is normalized on loading, as in the native provider.
#[derive(serde::Deserialize)]
struct DemoTrack {
    id: TrackId,
    name: String,
    artist: String,
    album: String,
    image: String,
    duration_ms: u32,
    audio: AudioFeatures,
}

impl super::Music {
    pub(crate) fn advance_demo(&mut self) {
        while self.playing {
            let index = self.timeline.index;
            let Some(track) = self.queue.get(index) else { break };
            let duration = track.duration_ms as f32;
            let position = self.timeline.position_now();
            if position < duration {
                break;
            }
            self.playing = index + 1 < self.queue.len();
            self.observe(
                index + usize::from(self.playing),
                if self.playing { position - duration } else { duration },
                f32::from(self.playing),
                Instant::now(),
            );
        }
    }
}
