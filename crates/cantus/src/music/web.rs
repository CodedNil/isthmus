use super::{MusicResult, PlaybackCommand, Track, TrackId, lyrics::Lyrics};
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
    responses: Arc<HashMap<String, Box<serde_json::value::RawValue>>>,
}

impl Spotify {
    pub(super) fn new(_config: &Config, updater: &AppUpdater) -> Self {
        let tracks: Vec<FixtureTrack> = serde_json::from_str(include_str!("../../../../assets/web/tracks.json"))
            .unwrap_or_else(|error| {
                tracing::error!(%error, "Invalid bundled Spotify tracks");
                Vec::new()
            });
        let mut queue = Vec::new();
        let mut responses = HashMap::new();
        for FixtureTrack { track, audio } in tracks {
            if let Some(id) = track.id {
                responses.insert(super::audio_features_path(id), audio);
            }
            queue.push(track);
        }
        fastrand::shuffle(&mut queue);
        send_update(updater, move |app| {
            app.music.replace_queue(queue, 3, 42_000.0, 1.0, Instant::now());
            app.music.playing = true;
        });
        Self { updater: updater.clone(), responses: Arc::new(responses) }
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        send_update(&self.updater, move |app| {
            let music = &mut app.music;
            let mut index = music.timeline.index;
            let mut position = music.timeline.position_now();
            match command {
                PlaybackCommand::PlayPlaylist(id) => {
                    tracing::debug!(?id, "Playlist playback is unavailable in the demo")
                }
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

    pub(super) fn lyrics(&self, _track_id: TrackId) -> Ready<MusicResult<Lyrics>> {
        ready(Ok(Lyrics::default()))
    }

    pub(super) async fn get_json(&self, path: &str) -> MusicResult<&str> {
        self.responses
            .get(path)
            .map(|response| response.get())
            .ok_or_else(|| format!("No demo Spotify response for {path}").into())
    }
}

/// Track metadata and its Spotify audio response stay together in the fixture.
#[derive(serde::Deserialize)]
struct FixtureTrack {
    #[serde(flatten)]
    track: Track,
    audio: Box<serde_json::value::RawValue>,
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
