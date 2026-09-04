//! Browser music placeholder.
//!
//! The native worker speaks Spotify's Connect protocol through librespot. That
//! protocol uses native sockets and cannot be carried into a browser directly;
//! this small implementation keeps the visual demo populated until a browser
//! Web API/Web Playback integration is added.

use std::sync::Arc;

use super::{AudioFeatures, LyricSegment, MusicResult, PlaybackCommand, Track, TrackRuntime};
use crate::{
    app::{AppUpdater, Background, send_update},
    config::Config,
    time::Instant,
};

#[derive(Clone)]
pub struct Spotify {
    updater: Arc<AppUpdater>,
}

impl Spotify {
    pub(super) fn new(_config: &Config, updater: &AppUpdater, _background: &Background) -> Self {
        let spotify = Self {
            updater: Arc::new(updater.clone()),
        };
        let queue = example_queue();
        send_update(updater, move |app| {
            app.music.replace_queue(queue, 0, 42_000.0, 1.0, Instant::now());
            app.music.playing = true;
        });
        spotify
    }

    pub(super) fn command(&self, command: PlaybackCommand) {
        if let PlaybackCommand::SetPlaying(playing) = command {
            let updater = Arc::clone(&self.updater);
            send_update(&updater, move |app| app.music.playing = playing);
        }
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
