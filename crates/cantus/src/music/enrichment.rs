use super::{ART_SIZE, AudioFeatures, MusicResult, TrackId, lyrics::Lyrics};
use crate::{
    app::{Background, CantusApp},
    render::music::PALETTE_COLORS,
};
use arrayvec::ArrayVec;
use futures_util::future::join_all;
use image::{RgbaImage, imageops};
use isthmus::{Image, Unorm8x4, glam::Vec3};
use palette::{Clamp, IntoColor, Lch, color_theory::Analogous};
use reqwest::Client;
use std::{array, collections::HashMap, ops::Range, time::Duration};
#[cfg(not(target_arch = "wasm32"))]
use tokio::task::spawn_blocking;
use tracing::warn;
use web_time::Instant;

const RETRY_DELAY: Duration = Duration::from_secs(30);
#[derive(Clone)]
pub struct Enrichment {
    pub(crate) background: Background,
    pub(crate) http: Client,
}

impl Enrichment {
    pub(crate) fn new(background: Background) -> Self {
        let client = Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let client = client.timeout(Duration::from_secs(15));
        Self { background, http: client.build().expect("failed to construct HTTP client") }
    }
}

pub enum Fetch<T> {
    Missing(Instant),
    Fetching,
    Ready(T),
}

impl<T> Default for Fetch<T> {
    fn default() -> Self {
        Self::Missing(Instant::now())
    }
}

impl<T> Fetch<T> {
    pub fn retry() -> Self {
        Self::Missing(Instant::now() + RETRY_DELAY)
    }

    pub fn request(&mut self, now: Instant) -> bool {
        let Self::Missing(retry_at) = self else { return false };
        if *retry_at > now {
            return false;
        }
        *self = Self::Fetching;
        true
    }

    pub const fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Missing(_) | Self::Fetching => None,
        }
    }
}

pub struct AlbumArt {
    pub image: Image,
    palette: [Unorm8x4; PALETTE_COLORS],
}

#[derive(Default)]
pub struct Resources {
    pub art: HashMap<String, Fetch<AlbumArt>>,
    pub audio: HashMap<TrackId, Fetch<AudioFeatures>>,
    pub lyrics: HashMap<String, Fetch<Lyrics>>,
}

impl Resources {
    pub fn art(&self, url: Option<&str>) -> Option<&AlbumArt> {
        url.and_then(|url| self.art.get(url)).and_then(Fetch::ready)
    }

    pub fn palette(&self, url: Option<&str>) -> [Unorm8x4; PALETTE_COLORS] {
        self.art(url)
            .map_or_else(|| [Unorm8x4::from_vec3(Vec3::new(0.24, 0.32, 0.44)); PALETTE_COLORS], |art| art.palette)
    }

    pub fn audio(&self, id: Option<TrackId>) -> AudioFeatures {
        id.and_then(|id| self.audio.get(&id)).and_then(Fetch::ready).copied().unwrap_or_default()
    }
}

async fn fetch_art(http: &Client, url: &str) -> Fetch<AlbumArt> {
    let result: MusicResult<_> = async {
        let bytes = http.get(url).send().await?.error_for_status()?.bytes().await?;
        #[cfg(not(target_arch = "wasm32"))]
        let art = spawn_blocking(move || decode_art(&bytes)).await??;
        #[cfg(target_arch = "wasm32")]
        let art = decode_art(&bytes)?;
        Ok(art)
    }
    .await;
    match result {
        Ok(art) => Fetch::Ready(art),
        Err(error) => {
            warn!(%error, %url, "Failed to load image");
            Fetch::retry()
        }
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
    let (width, height) = image.dimensions();
    Ok(AlbumArt { palette: image_palette(&image), image: Image::rgba8((width, height).into(), image.into_raw()) })
}

impl CantusApp {
    pub(crate) fn refresh_enrichment(&mut self) {
        let now = Instant::now();
        let music = &mut self.music;
        let resources = &mut music.resources;
        resources.art.retain(|url, state| {
            matches!(state, Fetch::Fetching)
                || music.queue.iter().any(|track| track.image.as_ref() == Some(url))
                || music.playlists.iter().any(|playlist| playlist.image_url.as_ref() == Some(url))
        });
        resources.audio.retain(|id, state| {
            matches!(state, Fetch::Fetching) || music.queue.iter().any(|track| track.id == Some(*id))
        });
        resources
            .lyrics
            .retain(|uri, state| matches!(state, Fetch::Fetching) || music.queue.iter().any(|track| &track.uri == uri));
        if self.config.lyrics_enabled {
            let start = music.timeline.index.saturating_sub(1).min(music.queue.len());
            let end = music.timeline.index.saturating_add(3).min(music.queue.len());
            for track in &music.queue[start..end] {
                if !track.name.trim().is_empty() && resources.lyrics.entry(track.uri.clone()).or_default().request(now)
                {
                    self.enrichment.request_lyrics(track.clone(), music.spotify.clone());
                }
            }
        }
        let audio: Vec<_> = music
            .queue
            .iter()
            .filter_map(|track| track.id)
            .filter(|id| resources.audio.entry(*id).or_default().request(now))
            .collect();
        if !audio.is_empty() {
            let spotify = music.spotify.clone();
            self.enrichment.background.spawn_update(async move {
                let features = join_all(audio.into_iter().map(|id| {
                    let spotify = &spotify;
                    async move {
                        let features = spotify
                            .audio_features(id)
                            .await
                            .inspect_err(|error| warn!(%error, %id, "Failed to fetch Spotify audio features"))
                            .ok();
                        (id, features)
                    }
                }))
                .await;
                Some(move |app: &mut Self| {
                    for (id, features) in features {
                        if let Some(slot @ Fetch::Fetching) = app.music.resources.audio.get_mut(&id) {
                            *slot = features.map_or_else(Fetch::retry, Fetch::Ready);
                        }
                    }
                })
            });
        }
        for url in music
            .queue
            .iter()
            .filter_map(|track| track.image.as_ref())
            .chain(music.playlists.iter().filter_map(|playlist| playlist.image_url.as_ref()))
        {
            if !resources.art.entry(url.clone()).or_default().request(now) {
                continue;
            }
            let url = url.clone();
            let http = self.enrichment.http.clone();
            self.enrichment.background.spawn_update(async move {
                let state = fetch_art(&http, &url).await;
                Some(move |app: &mut Self| {
                    if let Some(slot @ Fetch::Fetching) = app.music.resources.art.get_mut(&url) {
                        *slot = state;
                    }
                })
            });
        }
    }
}

fn complete_palette(colors: &mut ArrayVec<(Lch, f32), PALETTE_COLORS>) {
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

fn palette_color((color, weight): (Lch, f32), total: f32) -> Unorm8x4 {
    let rgb: palette::Srgb = color.into_color();
    let rgb = rgb.clamp();
    Unorm8x4::from_vec4(Vec3::new(rgb.red, rgb.green, rgb.blue).extend((weight / total).max(1.0 / 255.0)))
}

fn dominant_colors(pixels: &mut [Vec3]) -> ArrayVec<(Lch, f32), PALETTE_COLORS> {
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

    buckets
        .into_iter()
        .map(|range| {
            let weight = range.len() as f32;
            let mean = pixels[range].iter().copied().sum::<Vec3>() / weight;
            (palette::Lab::new(mean.x, mean.y, mean.z).into_color(), weight)
        })
        .collect()
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
    let mut colors = dominant_colors(&mut pixels);
    if use_harmony {
        complete_palette(&mut colors);
    }
    let total = colors.iter().map(|(_, weight)| weight).sum();
    array::from_fn(|index| palette_color(colors[index % colors.len()], total))
}
