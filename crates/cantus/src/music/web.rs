use super::{AudioFeatures, MusicResult, PlaybackCommand, Track, TrackId, spotify_lyrics::SpotifyLyrics};
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
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the browser fixture implements the native async client interface"
    )]
    pub(super) async fn get_lyrics(&self, _id: TrackId) -> MusicResult<Option<SpotifyLyrics>> {
        Ok(None)
    }

    pub(super) fn new(_config: &Config, updater: &AppUpdater) -> Self {
        let tracks: Vec<FixtureTrack> =
            serde_json::from_str(include_str!("../../assets/tracks.json")).unwrap_or_else(|error| {
                tracing::error!(%error, "Invalid bundled Spotify tracks");
                Vec::new()
            });
        let mut queue = Vec::new();
        let mut audio = HashMap::new();
        for fixture in tracks {
            if let Some(id) = fixture.track.id {
                audio.insert(id, fixture.audio);
            }
            queue.push(fixture.track);
        }
        fastrand::shuffle(&mut queue);
        send_update(updater, move |app| {
            app.music.queue.replace(queue, 3, 42_000.0);
            app.music.queue.observe(3, 42_000.0, 1.0, Instant::now());
            app.music.playing = true;
        });
        Self { updater: updater.clone(), audio: Arc::new(audio) }
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        send_update(&self.updater, move |app| {
            let music = &mut app.music;
            let mut index = music.queue.index;
            let mut position = music.queue.timeline.position_now();
            match command {
                PlaybackCommand::PlayPlaylist(id) => {
                    tracing::debug!(?id, "Playlist playback is unavailable in the demo");
                }
                PlaybackCommand::SetPlaying(playing) => music.playing = playing,
                PlaybackCommand::Seek(milliseconds) => position = milliseconds as f32,
                PlaybackCommand::Skip { forward } => {
                    index = index
                        .saturating_add_signed(if forward { 1 } else { -1 })
                        .min(music.queue.tracks.len().saturating_sub(1));
                    position = 0.0;
                }
                PlaybackCommand::JumpTo { uri, uid } => {
                    if let Some(target) =
                        music.queue.tracks.iter().position(|track| track.uri == uri && track.queue_uid == uid)
                    {
                        index = target;
                        position = 0.0;
                    }
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
            let duration = music.queue.tracks.get(index).map_or(0.0, |track| track.duration_ms as f32);
            music.queue.observe(index, position.clamp(0.0, duration), f32::from(music.playing), Instant::now());
        });
    }

    pub(super) fn audio_features(&self, ids: &[TrackId]) -> Ready<MusicResult<HashMap<TrackId, AudioFeatures>>> {
        ready(Ok(ids.iter().filter_map(|id| self.audio.get(id).map(|features| (*id, *features))).collect()))
    }
}

/// Track metadata and its Spotify audio response stay together in the fixture.
#[derive(serde::Deserialize)]
struct FixtureTrack {
    #[serde(flatten)]
    track: Track,
    audio: AudioFeatures,
}

impl super::Music {
    pub(crate) fn advance_demo(&mut self) {
        while self.playing {
            let index = self.queue.index;
            let Some(track) = self.queue.tracks.get(index) else { break };
            let duration = track.duration_ms as f32;
            let position = self.queue.timeline.position_now();
            if position < duration {
                break;
            }
            self.playing = index + 1 < self.queue.tracks.len();
            self.queue.observe(
                index + usize::from(self.playing),
                if self.playing { position - duration } else { duration },
                f32::from(self.playing),
                Instant::now(),
            );
        }
    }
}
