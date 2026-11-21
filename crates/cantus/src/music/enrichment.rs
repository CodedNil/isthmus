use super::{ART_SIZE, AudioFeatures, MusicResult, Track, TrackId, lyrics};
use crate::{
    app::{Background, CantusApp},
    render::{lyrics::PreparedLyrics, music::PALETTE_COLORS},
};
use arrayvec::ArrayVec;
use image::{RgbaImage, imageops};
use isthmus::{Image, Unorm8x4, glam::Vec3};
use palette::{Clamp, IntoColor, Lch, color_theory::Analogous};
use std::{
    array,
    collections::{HashMap, HashSet},
    fmt::Display,
    hash::Hash,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use strum_macros::{EnumIs, EnumTryAs};
use tokio::{sync::Mutex, task::spawn_blocking};
use tracing::warn;

const AUDIO_FEATURE_BATCH_SIZE: usize = 240;
const RETRY_DELAY: Duration = Duration::from_secs(30);

#[cfg(test)]
#[test]
fn fresh_fetch_starts_immediately_and_failed_fetch_waits() {
    let now = Instant::now();
    assert!(Fetch::<()>::default().request(now));

    let mut failed = Fetch::<()>::Missing(Some(now + RETRY_DELAY));
    assert!(!failed.request(now));
    assert!(failed.request(now + RETRY_DELAY));
}

#[derive(EnumIs, EnumTryAs)]
pub enum Fetch<T> {
    Missing(Option<Instant>),
    Fetching,
    Unavailable,
    Ready(T),
}

impl<T> Default for Fetch<T> {
    fn default() -> Self {
        Self::Missing(None)
    }
}

impl<T> Fetch<T> {
    pub fn request(&mut self, now: Instant) -> bool {
        let Self::Missing(retry_at) = self else { return false };
        if retry_at.is_some_and(|retry_at| retry_at > now) {
            return false;
        }
        *self = Self::Fetching;
        true
    }
}

impl Background {
    fn fetch<K: Eq + Hash + Display + Send + 'static, T: Send + 'static>(
        &self,
        key: K,
        task: impl Future<Output = MusicResult<T>> + Send + 'static,
        cache: fn(&mut TrackCache) -> &mut HashMap<K, Fetch<T>>,
    ) {
        self.spawn_update(async move {
            let result = match task.await {
                Ok(value) => Fetch::Ready(value),
                Err(error) => {
                    warn!(%error, %key, "Failed to fetch music resource");
                    Fetch::Missing(Some(Instant::now() + RETRY_DELAY))
                }
            };
            Some(move |app: &mut CantusApp| {
                if let Some(slot @ Fetch::Fetching) = cache(&mut app.music.resources).get_mut(&key) {
                    *slot = result;
                }
            })
        });
    }
}

pub struct AlbumArt {
    pub image: Image,
    palette: [Unorm8x4; PALETTE_COLORS],
}

#[derive(Default)]
pub struct TrackCache {
    pub art: HashMap<String, Fetch<AlbumArt>>,
    audio: HashMap<TrackId, Fetch<AudioFeatures>>,
    visible_tracks: Vec<u64>,
    pub(crate) visibility_changed: bool,
    pub lyrics: HashMap<String, Fetch<PreparedLyrics>>,
    lyrics_session: Arc<Mutex<lyrics::Session>>,
}

impl TrackCache {
    pub fn art(&self, url: Option<&str>) -> Option<&AlbumArt> {
        url.and_then(|url| self.art.get(url)).and_then(Fetch::try_as_ready_ref)
    }

    pub fn palette(&self, url: Option<&str>) -> [Unorm8x4; PALETTE_COLORS] {
        self.art(url)
            .map_or_else(|| [Unorm8x4::from_vec3(Vec3::new(0.24, 0.32, 0.44)); PALETTE_COLORS], |art| art.palette)
    }

    pub fn set_visible_tracks(&mut self, tracks: impl Iterator<Item = u64> + Clone) {
        if !self.visible_tracks.iter().copied().eq(tracks.clone()) {
            self.visible_tracks.clear();
            self.visible_tracks.extend(tracks);
            self.visibility_changed = true;
        }
    }

    pub fn audio(&self, id: Option<TrackId>) -> AudioFeatures {
        id.and_then(|id| self.audio.get(&id)).and_then(Fetch::try_as_ready_ref).copied().unwrap_or_default()
    }

    pub fn lyrics(&mut self, track: &Track) -> Option<&PreparedLyrics> {
        let Fetch::Ready(lyrics) = self.lyrics.get_mut(&track.uri)? else { return None };
        lyrics.prepare(track.duration_ms as f32);
        Some(lyrics)
    }
}

fn decode_art(bytes: &[u8]) -> Result<AlbumArt, image::ImageError> {
    let image = image::load_from_memory(bytes)?;
    let image = if image.width() > ART_SIZE || image.height() > ART_SIZE {
        image.resize_to_fill(ART_SIZE, ART_SIZE, imageops::FilterType::Triangle)
    } else {
        image
    }
    .into_rgba8();
    Ok(AlbumArt { palette: image_palette(&image), image: Image::rgba8(image.dimensions().into(), image.into_raw()) })
}

impl CantusApp {
    pub(crate) fn refresh_enrichment(&mut self) {
        let now = Instant::now();
        let publish_lyrics = self.config.lyrics_publish_enabled;
        let music = &mut self.music;
        let resources = &mut music.resources;
        let queue = &music.queue;
        let artwork: HashSet<_> = queue
            .tracks
            .iter()
            .filter_map(|track| track.image.as_deref())
            .chain(music.playlists.iter().filter_map(|playlist| playlist.image_url.as_deref()))
            .collect();
        let track_ids: HashSet<_> = queue.tracks.iter().filter_map(|track| track.id).collect();
        let track_uris: HashSet<_> = queue.tracks.iter().map(|track| track.uri.as_str()).collect();
        resources.art.retain(|url, state| state.is_fetching() || artwork.contains(url.as_str()));
        resources.audio.retain(|id, state| state.is_fetching() || track_ids.contains(id));
        resources.lyrics.retain(|uri, state| state.is_fetching() || track_uris.contains(uri.as_str()));
        let tracks = &queue.tracks;
        let focus = queue.span_at_playhead().map_or(queue.index, |(index, _)| index);
        let focus = focus.min(tracks.len());
        let nearby = tracks[focus..(focus + 3).min(tracks.len())]
            .iter()
            .chain(&tracks[focus.saturating_sub(1)..focus])
            .chain(tracks.get(queue.index));
        let visible = tracks.iter().enumerate().filter_map(|(index, track)| {
            tracks[index.saturating_sub(1)..(index + 2).min(tracks.len())]
                .iter()
                .any(|entry| resources.visible_tracks.contains(&entry.interaction_id))
                .then_some(track)
        });
        let demand = nearby.clone().chain(visible).filter(|_| !resources.visible_tracks.is_empty());
        if self.config.lyrics_enabled
            && let Some(track) = nearby
                .filter(|track| {
                    !track.name.trim().is_empty() && !track.primary_artist().trim().is_empty() && track.duration_ms > 0
                })
                .find(|track| resources.lyrics.entry(track.uri.clone()).or_default().request(now))
        {
            let track = track.clone();
            let session = Arc::clone(&resources.lyrics_session);
            let http = self.background.http.clone();
            let spotify = music.spotify.clone();
            self.background.fetch(
                track.uri.clone(),
                async move {
                    lyrics::fetch(&track, &http, &session, &spotify, publish_lyrics).await.map(|source| {
                        let contributed = lyrics::contributed_by_us(&track, &source);
                        PreparedLyrics::new(source, contributed)
                    })
                },
                |cache| &mut cache.lyrics,
            );
        }
        let audio_ids = demand
            .clone()
            .filter_map(|track| track.id)
            .filter(|id| resources.audio.entry(*id).or_default().request(now))
            .collect::<Vec<_>>();
        for batch in audio_ids.chunks(AUDIO_FEATURE_BATCH_SIZE) {
            let ids = batch.to_vec();
            let spotify = music.spotify.clone();
            self.background.spawn_update(async move {
                let result = spotify.audio_features(&ids).await;
                if let Err(error) = &result {
                    warn!(%error, tracks = ids.len(), "Failed to fetch Spotify audio features");
                }
                let retry_at = Instant::now() + RETRY_DELAY;
                Some(move |app: &mut Self| {
                    for id in ids {
                        if let Some(slot @ Fetch::Fetching) = app.music.resources.audio.get_mut(&id) {
                            let Ok(features) = &result else {
                                *slot = Fetch::Missing(Some(retry_at));
                                continue;
                            };
                            *slot = features.get(&id).map_or(Fetch::Unavailable, |features| {
                                Fetch::Ready(AudioFeatures {
                                    energy: features.energy.clamp(0.0, 1.0),
                                    danceability: features.danceability.clamp(0.0, 1.0),
                                    acousticness: features.acousticness.clamp(0.0, 1.0),
                                    tempo: (features.tempo / 300.0).clamp(0.0, 1.0),
                                    valence: features.valence.clamp(0.0, 1.0),
                                    instrumentalness: features.instrumentalness.clamp(0.0, 1.0),
                                })
                            });
                        }
                    }
                })
            });
        }
        for url in demand
            .filter_map(|track| track.image.as_ref())
            .chain(music.playlists.iter().filter_map(|playlist| playlist.image_url.as_ref()))
        {
            if !resources.art.entry(url.clone()).or_default().request(now) {
                continue;
            }
            let url = url.clone();
            let http = self.background.http.clone();
            self.background.fetch(
                url.clone(),
                async move {
                    let bytes = http.get(url).send().await?.error_for_status()?.bytes().await?;
                    let art = spawn_blocking(move || decode_art(&bytes)).await??;
                    Ok(art)
                },
                |cache| &mut cache.art,
            );
        }
    }
}

fn image_palette(image: &RgbaImage) -> [Unorm8x4; PALETTE_COLORS] {
    let srgb_to_lab = |pixel: &image::Rgba<u8>| {
        let color: palette::Lab =
            palette::Srgb::new(f32::from(pixel[0]) / 255.0, f32::from(pixel[1]) / 255.0, f32::from(pixel[2]) / 255.0)
                .into_color();
        Vec3::new(color.l, color.a, color.b)
    };
    let mut pixels: Vec<Vec3> = image
        .pixels()
        .filter(|pixel| {
            let max = pixel[0].max(pixel[1]).max(pixel[2]);
            let min = pixel[0].min(pixel[1]).min(pixel[2]);
            pixel[3] >= 128 && max - min > 30
        })
        .map(srgb_to_lab)
        .collect();
    let use_harmony = !pixels.is_empty();
    if !use_harmony {
        pixels.extend(image.pixels().filter(|pixel| pixel[3] >= 128).map(srgb_to_lab));
    }
    if pixels.is_empty() {
        return [Unorm8x4::default(); PALETTE_COLORS];
    }
    let mut buckets = ArrayVec::<Range<usize>, PALETTE_COLORS>::new();
    buckets.push(0..pixels.len());

    while buckets.len() < PALETTE_COLORS {
        let Some((bucket_index, channel)) = buckets
            .iter()
            .enumerate()
            .filter(|(_, range)| range.len() > 1)
            .map(|(index, range)| {
                let (min, max) = pixels[range.clone()]
                    .iter()
                    .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(min, max), &color| {
                        (min.min(color), max.max(color))
                    });
                let (channel, spread) = (0..3)
                    .map(|channel| (channel, max[channel] - min[channel]))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                (index, channel, spread * range.len() as f32)
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(index, channel, _)| (index, channel))
        else {
            break;
        };

        let range = buckets.swap_remove(bucket_index);
        pixels[range.clone()].select_nth_unstable_by(range.len() / 2, |a, b| a[channel].total_cmp(&b[channel]));
        let middle = range.start + range.len() / 2;
        buckets.push(range.start..middle);
        buckets.push(middle..range.end);
    }

    let mut colors = buckets
        .into_iter()
        .map(|range| {
            let weight = range.len() as f32;
            let mean = pixels[range].iter().copied().sum::<Vec3>() / weight;
            (palette::Lab::new(mean.x, mean.y, mean.z).into_color(), weight)
        })
        .collect::<ArrayVec<(Lch, f32), PALETTE_COLORS>>();
    if use_harmony {
        colors.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut index = 1;
        while index < colors.len() {
            let (color, weight) = colors[index];
            if let Some(duplicate) =
                colors[..index].iter().position(|(other, _)| (color.hue - other.hue).into_degrees().abs() < 20.0)
            {
                colors[duplicate].1 += weight;
                colors.remove(index);
            } else {
                index += 1;
            }
        }

        let measured = colors.len();
        for index in 0..PALETTE_COLORS - measured {
            let (source, weight) = colors[index % measured];
            let (lower, upper) = source.analogous();
            let mut generated = match index {
                2 if measured == 1 => source.analogous_secondary().0,
                index if index % 2 == 0 => lower,
                _ => upper,
            };
            generated.chroma = generated.chroma.max(35.0);
            colors.push((generated, weight * 0.5));
        }
        colors.sort_by(|a, b| a.0.l.total_cmp(&b.0.l));
    }
    let total: f32 = colors.iter().map(|(_, weight)| weight).sum();
    array::from_fn(|index| {
        let (color, weight) = colors[index % colors.len()];
        let rgb: palette::Srgb = color.into_color();
        let rgb = rgb.clamp();
        Unorm8x4::from_vec4(Vec3::new(rgb.red, rgb.green, rgb.blue).extend((weight / total).max(1.0 / 255.0)))
    })
}
