use super::{AudioFeatures, MusicResult, PlaybackCommand, Track, TrackRuntime, lyrics::LyricSegment};
use crate::{
    app::{AppUpdater, send_update},
    config::Config,
};
use web_time::Instant;

#[derive(Clone)]
pub struct Spotify {
    updater: AppUpdater,
}

impl Spotify {
    pub(super) fn new(_config: &Config, updater: &AppUpdater) -> Self {
        let spotify = Self { updater: updater.clone() };
        let queue = example_queue();
        send_update(updater, move |app| {
            app.music.replace_queue(queue, 0, 42_000.0, 1.0, Instant::now());
            app.music.playing = true;
        });
        spotify
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        send_update(&self.updater, move |app| {
            let music = &mut app.music;
            let mut position = music.timeline.position_now();
            match command {
                PlaybackCommand::SetPlaying(playing) => music.playing = playing,
                PlaybackCommand::Seek(milliseconds) => position = milliseconds as f32,
                PlaybackCommand::Skip(offset) => {
                    music.timeline.index = music
                        .timeline
                        .index
                        .saturating_add_signed(isize::from(offset))
                        .min(music.queue.len().saturating_sub(1));
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
            let duration = music.queue.get(music.timeline.index).map_or(0.0, |track| track.duration_ms as f32);
            music.timeline.position_ms = position.clamp(0.0, duration);
            music.timeline.observed_at = Instant::now();
            music.timeline.rate = f32::from(music.playing);
        });
    }

    pub(super) async fn lyrics(&self, _track_id: super::TrackId) -> MusicResult<Vec<LyricSegment>> {
        Ok(Vec::new())
    }

    pub(super) async fn audio_features(&self, _track_id: super::TrackId) -> MusicResult<AudioFeatures> {
        Ok(AudioFeatures {
            energy: 0.68,
            danceability: 0.74,
            acousticness: 0.18,
            tempo: 0.41,
            valence: 0.62,
            instrumentalness: 0.04,
        })
    }
}

fn example_queue() -> Vec<Track> {
    [
        ("Night Drive", "Example Artist", "Synthetic Horizons", 214_000),
        ("Blue Hour", "Example Artist", "Synthetic Horizons", 192_000),
        ("Afterglow", "Example Artist", "Synthetic Horizons", 247_000),
    ]
    .into_iter()
    .map(|(name, artist, album, duration_ms)| Track {
        id: None,
        uri: format!("web:track:{name}"),
        name: name.into(),
        artist: artist.into(),
        album: album.into(),
        image: None,
        duration_ms,
        interaction_id: Track::next_interaction_id(),
        runtime: TrackRuntime::default(),
    })
    .collect()
}
