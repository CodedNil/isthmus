use crate::{
    music::{Music, TRACK_SPACING_MS, Track},
    render::{
        BarLayout, GAP, PANEL_START, Program, TEXT_COLOR, UiContext,
        sdf::{deform, glass, hash, refract, simplex_noise},
    },
};
use core::{
    f32::consts::{FRAC_PI_2, TAU},
    f64,
};
use isthmus::{prelude::*, spirv_std::arch::kill};
use isthmus_sdf::prelude::*;
use serde::Deserialize;
use smallvec::SmallVec;
use std::{iter, sync::LazyLock};

/// Number of colors extracted from album artwork.
pub const PALETTE_COLORS: usize = 4;
/// Visual width, in pixels, of rating and playlist icons before hover growth.
const ICON_WIDTH: f32 = 21.6;
/// Center-to-center icon spacing for rating stars and playlist artwork.
const ICON_SPACING: f32 = 18.0;
/// Height added below the pill for ratings and saved playlists.
const PRIMARY_SUPPORT_DEPTH: f32 = 7.0;
/// Height added below the first icon row for other playlists.
const SECONDARY_SUPPORT_DEPTH: f32 = 18.0;
/// Space reserved for each collapsed history track, excluding its gap.
const HISTORY_WIDTH: f32 = 10.0;
/// Transparent texture used while artwork is unavailable and for rating icons.
static EMPTY_ART: LazyLock<Image> = LazyLock::new(|| Image::rgba8([1, 1], vec![0; 4]));

struct TrackLayout {
    queue_index: usize,
    start_ms: f32,
    natural_start: f32,
    width: f32,
    right: f32,
}

fn music_shape(pill: Rect, icon_supports: [Vec2; 2]) -> impl Sdf {
    let support = |index: usize| {
        Shape::pill(Rect::from_center_size(
            vec2(pill.center().x, pill.max.y - 6.0 + index as f32 * 7.0 + icon_supports[index].y * 0.5),
            icon_supports[index],
        ))
    };
    Shape::pill(pill).smooth_union(support(0), 9.0, icon_supports[0].y / PRIMARY_SUPPORT_DEPTH).smooth_union(
        support(1),
        9.0,
        icon_supports[1].y / SECONDARY_SUPPORT_DEPTH,
    )
}

/// Spotify audio characteristics normalized for shader and UI use.
#[derive(Clone, Copy, Default, ShaderData, Deserialize)]
#[shader_data(unorm16)]
pub struct AudioFeatures {
    pub energy: f32,
    pub danceability: f32,
    pub acousticness: f32,
    pub tempo: f32,
    pub valence: f32,
    pub instrumentalness: f32,
}

#[derive(Default)]
pub struct MusicView {
    particles: [Particle; 64] = [Particle { .. }; 64],
    particles_debt: f32,
    bar_split: f32,
    icon_presence: f32,
    icon_morph: f32,
    playlist_hover: Vec<f32>,
}

#[derive(Clone, Copy)]
struct Particle {
    origin: Vec2 = Vec2::ZERO,
    velocity: Vec2 = Vec2::ZERO,
    spawned_at: f32 = 0.0,
    expires_at: f32 = 0.0,
    color: Vec3 = Vec3::ZERO,
    size: Vec2 = Vec2::new(10.0, 5.0),
}

fn shade_icon(color: Vec3, shape: impl Sdf, point: Vec2) -> Vec4 {
    let sample = shape.outlined(8.0).sample_at(point);
    let bevel = sample.distance.smoothstep(-5.0, 0.0);
    sample
        .paint((color + bevel * bevel * 0.045).extend(1.0), Vec3::ZERO.extend((-sample.distance.max(0.0)).exp() * 0.2))
}

/// Twinkling points for acoustic tracks.
fn speckle(pixel: Vec2, time: f32, seed: f32, audio: AudioFeatures) -> f32 {
    let drift = vec2(0.16 + seed.fract() * 0.08, 0.055 + (seed * 0.7).sin() * 0.025);
    let uv = pixel / (8.0 - audio.acousticness) + (time * 0.5 + (time * 0.31).sin() * audio.energy) * drift;
    let cell = uv.floor();
    let phase = hash(vec2(cell.y, cell.x) + seed * 4096.0 + 2.71).x;
    let center = vec2(phase, (phase * 7.13).fract()) * 0.56 - 0.28;
    let twinkle = time * (0.7 + phase * 0.9) + phase * TAU + (time * 0.7).sin() * audio.energy;
    hash(cell + seed * 4096.0).x.smoothstep(0.985 - audio.acousticness * 0.09, 1.0)
        * (1.0 - (uv - cell - 0.5 - center).length().smoothstep(0.06, 0.28))
        * (twinkle.sin() * 0.5 + 0.5)
        * (0.12 + audio.acousticness * 0.48)
}

/// Broad, domain-warped light variation for instrumental tracks.
fn caustics(p: Vec2, time: f32, seed: f32, audio: AudioFeatures) -> f32 {
    let domain = p * 1.35 + vec2(seed * 17.0, time * 0.4 + (time * 0.2).sin() * audio.energy);
    let warp = vec2(simplex_noise(domain), simplex_noise(domain + vec2(19.1, -7.3)));
    let light = simplex_noise(domain - warp * 0.6) * 0.5 + 0.5;
    light * light * audio.instrumentalness * 0.3
}

impl MusicView {
    pub fn show(&mut self, context: &mut UiContext, music: &mut Music, bar: BarLayout) {
        let mut burst = false;
        let playlists = music.playlists.iter().filter(|playlist| playlist.rating_index.is_none());
        let count = playlists.clone().count();
        let show_playlists = context.config.ratings_enabled || count > 0;
        let button_size = (context.config.height + PRIMARY_SUPPORT_DEPTH) / 3.0;
        let tracks_left = if show_playlists { (count + 1).div_ceil(3) as f32 * button_size + GAP * 2.0 } else { 0.0 };
        if show_playlists {
            self.playlist_hover.resize(count + 1, 0.0);
            let mut icons = SmallVec::<[_; 8]>::new();
            for (index, playlist) in iter::once(None).chain(playlists.map(Some)).enumerate() {
                let center = vec2(
                    GAP + (index / 3) as f32 * button_size + button_size * 0.5,
                    PANEL_START + ((index % 3) as f32 + 0.5) * button_size,
                );
                let rect = Rect::from_center_size(center, Vec2::splat(button_size - 2.0));
                let response = context
                    .interaction
                    .interact(("play-playlist", playlist.map(|playlist| playlist.id)), Shape::rectangle(rect));
                context.interaction.input_region(rect);
                if response.clicked {
                    music.play_playlist(playlist.map(|playlist| playlist.id));
                    burst = true;
                }
                let hover = &mut self.playlist_hover[index];
                *hover = hover.move_towards(f32::from(response.hovered), context.frame.delta_time.min(0.1) / 0.18);
                icons.push((playlist, rect, hover.smoothstep(0.0, 1.0)));
            }
            icons.sort_by(|a, b| a.2.total_cmp(&b.2));
            for (playlist, rect, hover) in icons {
                let rect = rect.expanded((button_size - 2.0) * hover * 0.3);
                shader!(
                    context
                        .frame
                        .upload({
                            let rect: Rect;
                            let liked: bool = playlist.is_none();
                            let hover: f32;
                            let image: &Image = playlist
                                .and_then(|playlist| music.resources.art(playlist.image_url.as_deref()))
                                .map_or(&*EMPTY_ART, |art| &art.image);
                        })
                        .primitive(Shape::rounded_rect(rect, 3.0))
                        .fragment(|_, surface| {
                            let uv = rect.uv(surface.pixel);
                            let texture = image.sample(uv);
                            let background = vec3(0.3, 0.2, 0.65).lerp(vec3(0.65, 0.8, 0.7), uv.y);
                            let color = if liked {
                                let p = (uv - vec2(0.5, 0.43)) * vec2(2.0, -2.0);
                                let heart = vec2(p.x.abs(), p.y);
                                let distance = (heart - vec2(0.23, 0.0)).length() - 0.3;
                                let tip = (heart.x - heart.y * 0.75 - 0.35).max(heart.y);
                                background.lerp(Vec3::ONE, 1.0 - distance.min(tip).smoothstep(-0.02, 0.02))
                            } else {
                                background.lerp(texture.truncate(), texture.w)
                            };
                            (color * (1.0 + hover * 0.2)).extend(1.0)
                        })
                );
            }
        }
        let drag = context.interaction.drag_motion();

        let playhead_track = music
            .timeline
            .span_at_playhead(&music.queue)
            .filter(|(index, elapsed)| *elapsed < music.queue[*index].duration_ms as f32);
        if drag.is_some_and(|(_, released)| released)
            && let Some((index, position_ms)) = playhead_track
        {
            let track = &music.queue[index];
            if track.duration_ms > 0 {
                music.seek(index, track.duration_ms, position_ms / track.duration_ms as f32);
            }
        }

        let (history_width, panel_height) = (context.config.history_width, context.config.height);
        let future_end = history_width + context.config.timeline_future_minutes * 60_000.0 * bar.px_per_ms;
        let gap = GAP.max(TRACK_SPACING_MS * bar.px_per_ms);
        let trim = gap - TRACK_SPACING_MS * bar.px_per_ms;
        let mut start_ms = music.timeline.queue_start_ms + music.queue.iter().map(Track::queue_span_ms).sum::<f32>();
        let mut next_left = None;
        let mut visible = SmallVec::<[TrackLayout; 16]>::new();
        for (queue_index, track) in music.queue.iter_mut().enumerate().rev() {
            start_ms -= track.queue_span_ms();
            let natural_start = bar.playhead_x + start_ms * bar.px_per_ms;
            if natural_start >= future_end {
                track.runtime.track_expansion = 0.0;
                continue;
            }
            let end = natural_start + (track.duration_ms as f32 * bar.px_per_ms - trim).max(0.0);
            let gap = if end <= history_width { gap } else { gap.min(track.queue_span_ms() * bar.px_per_ms) };
            let clipped = end.min(future_end) - natural_start.max(history_width);
            let width = clipped.max(if natural_start < history_width { HISTORY_WIDTH } else { 0.0 });
            let right = next_left.map_or_else(|| end.clamp(history_width, future_end), |left: f32| left - gap);
            visible.push(TrackLayout { queue_index, start_ms, natural_start, width, right });
            next_left = Some(right - width);
        }
        visible.reverse();

        let mouse_pos = context.interaction.mouse_pos();
        let mouse_pressure = context.interaction.pressure();
        let mut seek_action = None;
        let mut rated_track = None;
        let mut playlist_toggle = None;

        visible.sort_by_key(|layout| music.queue[layout.queue_index].runtime.track_expansion.to_bits());

        let playhead_index = playhead_track.map(|(index, _)| index);
        if !music.queue.is_empty()
            && !visible.iter().any(|layout| Some(layout.queue_index) == playhead_index && layout.right > tracks_left)
        {
            self.show_playhead(context, music, bar.playhead_x);
        }

        for layout in visible {
            let track = &mut music.queue[layout.queue_index];
            if layout.right <= tracks_left {
                track.runtime.track_expansion = 0.0;
                continue;
            }
            let opacity = (future_end - layout.natural_start).smoothstep(0.0, panel_height)
                * (layout.right - tracks_left).smoothstep(0.0, panel_height);
            let mut width = layout.width.max(panel_height);
            let mut x = layout.right - width;
            let expansion = track.runtime.track_expansion.smoothstep(0.0, 1.0);

            let track_text = (width > panel_height + 26.0 || expansion > 0.0).then(|| {
                let artists = track.artists.iter().take(3).map(String::as_str).collect::<Vec<_>>().join(", ");
                let seconds = (layout.start_ms / 1000.0).abs();
                let time = if seconds >= 60.0 {
                    let whole_seconds = seconds as u32;
                    format!("{}m{}s", whole_seconds / 60, whole_seconds % 60)
                } else {
                    format!("{}s", seconds.round())
                };
                let compact_details = format!("{time}\u{2004}•\u{2004}{}", track.primary_artist());
                let full_details = format!("{time}\u{2004}•\u{2004}{artists}");
                [
                    (track.compact_title(), track.name.as_str(), 16.0),
                    (compact_details.as_str(), full_details.as_str(), 14.0),
                ]
                .map(|(compact, full, size)| {
                    (
                        context.frame.resources.shape(compact, size, 700.0),
                        context.frame.resources.shape(full, size, 700.0),
                    )
                })
            });
            if let Some(lines) = &track_text {
                let target = lines.iter().map(|(_, full)| full.text.width).fold(0.0, f32::max) + panel_height + 26.0;
                let extra_width = (target - width).max(0.0) * expansion;
                let anchor = if Some(layout.queue_index) == playhead_index {
                    ((bar.playhead_x - x) / width).saturate()
                } else {
                    0.5
                };
                x -= extra_width * anchor;
                width += extra_width;
            }
            width = width.min((future_end - tracks_left).max(0.0));
            x = x.clamp(tracks_left, (future_end - width).max(tracks_left));
            let mut playlist_icons = SmallVec::<[usize; 8]>::new();
            let mut primary_count = 0;
            let mut rating = (context.config.ratings_enabled && track.id.is_some()).then_some(0);
            for (index, playlist) in music.playlists.iter().enumerate() {
                let contains_track = track.id.is_some_and(|id| playlist.tracks.contains(&id));
                match playlist.rating_index {
                    Some(value) if contains_track && rating.is_some() => rating = Some(i32::from(value) + 1),
                    None if contains_track => {
                        playlist_icons.insert(primary_count, index);
                        primary_count += 1;
                    }
                    None if expansion > 0.0 && track.id.is_some() => playlist_icons.push(index),
                    _ => {}
                }
            }

            let secondary_count = playlist_icons.len() - primary_count;
            let stars = rating.map_or(0, |_| 5);
            let row_width = |icons: f32| ((icons - 1.0).max(0.0) * ICON_SPACING + ICON_WIDTH * 0.7).max(ICON_WIDTH);
            let targets = [
                stars > 0 && width >= row_width(stars as f32).max(panel_height + 8.0),
                primary_count > 0 && width >= row_width((stars + primary_count) as f32).max(panel_height + 8.0),
            ];
            for (visibility, target) in track.runtime.icon_visibility.iter_mut().zip(targets) {
                *visibility = visibility.move_towards(f32::from(target), context.frame.delta_time / 0.15);
            }
            let [star_alpha, playlist_alpha] = track.runtime.icon_visibility.map(|value| value.smoothstep(0.0, 1.0));
            let primary_icons = stars as f32 * star_alpha + primary_count as f32 * playlist_alpha;
            let primary_alpha = star_alpha.max(playlist_alpha);
            let secondary_alpha = expansion * secondary_count.min(1) as f32;
            let icon_supports: [Vec2; 2] = [
                vec2(row_width(primary_icons) * primary_alpha, primary_alpha * PRIMARY_SUPPORT_DEPTH),
                vec2(row_width(secondary_count as f32) * secondary_alpha, secondary_alpha * SECONDARY_SUPPORT_DEPTH),
            ];
            let pill = Rect::new(vec2(x, PANEL_START), vec2(x + width, PANEL_START + panel_height));
            let shape = music_shape(pill, icon_supports);
            let bounds = shape.bounds(0.0);
            let (min, max) = (bounds.min, bounds.max);
            // Halve the search interval until it is at most half a pixel wide.
            let steps = bounds.size().x.max(1.0).log2().ceil() as u32;
            for y in min.y.floor() as i32..max.y.ceil() as i32 {
                let point = vec2(pill.center().x, y as f32 + 0.5);
                if !shape.contains(point) {
                    continue;
                }
                let (mut inside, mut outside) = (0.0, bounds.size().x * 0.5);
                for _ in 0..steps {
                    let middle = (inside + outside) * 0.5;
                    if shape.contains(point + vec2(middle, 0.0)) {
                        inside = middle;
                    } else {
                        outside = middle;
                    }
                }
                context.interaction.input_region(Rect::from_center_size(point, vec2(inside * 2.0, 1.0)));
            }
            let body = context.interaction.drag(track.interaction_id, shape);
            let mut hovered = body.hovered;
            if body.clicked && track.duration_ms > 0 {
                let fraction = if layout.natural_start + track.duration_ms as f32 * bar.px_per_ms <= history_width
                    || layout.queue_index == music.timeline.index && mouse_pos.x <= x + pill.size().x * 0.05
                {
                    0.0
                } else {
                    ((mouse_pos.x - layout.natural_start) / (track.duration_ms as f32 * bar.px_per_ms)).saturate()
                };
                seek_action = Some((layout.queue_index, track.duration_ms, fraction));
            }

            let mut icons = Vec::new();
            if track.id.is_some() {
                for slot in 0..stars + primary_count + secondary_count {
                    let playlist_slot = slot.saturating_sub(stars);
                    let is_star = slot < stars;
                    let playlist = (!is_star).then(|| &music.playlists[playlist_icons[playlist_slot]]);
                    let secondary = slot >= stars + primary_count;
                    let (count, spread) =
                        if secondary { (secondary_count as f32, expansion) } else { (primary_icons, 1.0) };
                    let (icon, alpha) = if is_star {
                        (slot as f32 * star_alpha, star_alpha)
                    } else if secondary {
                        ((playlist_slot - primary_count) as f32, expansion)
                    } else {
                        (stars as f32 * star_alpha + playlist_slot as f32 * playlist_alpha, playlist_alpha)
                    };
                    if alpha <= 0.0 {
                        continue;
                    }
                    let center = vec2(
                        pill.center().x + (icon - (count - 1.0).max(0.0) * 0.5) * ICON_SPACING * spread,
                        PANEL_START + panel_height * 0.975 - 1.0 + f32::from(secondary) * ICON_SPACING * spread,
                    );
                    let response = context.interaction.interact(
                        (
                            "music-icon",
                            track.interaction_id,
                            is_star.then_some(slot),
                            playlist.map(|playlist| playlist.id),
                        ),
                        Shape::circle(center, ICON_WIDTH * 0.5),
                    );
                    hovered |= response.hovered;
                    icons.push((slot, playlist, secondary, is_star, alpha, center, response));
                }
            }
            let pointer_active = f32::from(hovered);
            let opacity = if hovered { 1.0 } else { opacity.lerp(1.0, expansion) };

            shader!(
                context
                    .frame
                    .upload({
                        let opacity: f32;
                        let pointer_active: f32;
                        let image: &Image =
                            music.resources.art(track.image.as_deref()).map_or(&*EMPTY_ART, |art| &art.image);
                        let seed: f32 = track.uri.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
                        }) as u32 as f32
                            * 2.328_306_4e-10;
                        let colors: [Unorm8x4; PALETTE_COLORS] = music.resources.palette(track.image.as_deref());
                        let audio: AudioFeatures = music.resources.audio(track.id);
                        let pill: Rect;
                        let icon_supports: [Vec2; 2];
                        let flow_time: f32 = {
                            let pace = ((audio.tempo - 0.2) * 2.5).saturate();
                            context.frame.time * 0.2 + seed + (context.frame.time * 0.17).sin() * (audio.energy + pace)
                        };
                        let beat: f32 = {
                            let pulse = ((f64::from(context.frame.time) * f64::from(audio.tempo) * 5.0).fract()
                                * f64::consts::TAU)
                                .sin() as f32
                                * 0.5
                                + 0.5;
                            pulse * pulse * audio.danceability * (0.025 + audio.energy * 0.055)
                        };
                        let turbulence: f32 = audio.energy * 0.65 + audio.danceability * 0.35;
                        let frequency: f32 =
                            (pill.size().x / pill.size().y * (0.5 + seed.fract() * 0.12 + turbulence * 0.18)).max(1.7);
                    })
                    .primitive(|mut frame| {
                        frame.globals.pressure *= pointer_active;
                        deform(music_shape(pill, icon_supports), frame)
                    })
                    .fragment(|frame, surface| {
                        let uv = pill.uv(surface.pixel);
                        let lens = (1.0 + surface.sdf.distance.min(0.0) / 120.0).saturate();
                        let deformation = (uv - 0.5) * lens * lens * 0.6 + surface.ripple;
                        let field = (uv.clamp(Vec2::ZERO, Vec2::ONE) - deformation * 0.08) * vec2(frequency, 1.6);
                        let warped = field
                            + vec2(
                                (field.y * 2.7 + flow_time).sin() + (field.x * 1.3 - flow_time * 0.7).cos(),
                                (field.x * 2.3 - flow_time * 0.8).cos() + (field.y * 1.7 + flow_time * 0.6).sin(),
                            ) * (0.14 + turbulence * 0.2 + beat);
                        let directions = [vec2(2.1, 0.7), vec2(0.6, -2.4), vec2(-1.5, 1.9), vec2(2.4, 1.6)];
                        let phases = [
                            flow_time,
                            seed + FRAC_PI_2 - flow_time * 0.8,
                            flow_time * 0.65 + 2.0,
                            seed + FRAC_PI_2 - flow_time * 0.55,
                        ];
                        let mut plasma = Vec4::ZERO;
                        for index in 0..PALETTE_COLORS {
                            let swatch = colors[index].to_vec4();
                            let wave = (warped.dot(directions[index]) + phases[index]).sin() * 0.5 + 0.5;
                            let weight = (0.12 + wave * wave) * (0.25 + swatch.w * 3.0);
                            plasma += (swatch.truncate() * weight).extend(weight);
                        }
                        let mut color = plasma.truncate() / plasma.w.max(0.001);
                        let luma = color.dot(vec3(0.2126, 0.7152, 0.0722));
                        color = Vec3::splat(luma)
                            .lerp(color, 1.55 + audio.valence * 0.4)
                            .clamp(Vec3::splat(0.035), Vec3::splat(0.92))
                            * (0.52 / luma.max(0.001)).min(1.0)
                            * (0.96 + audio.valence * 0.06 + beat * 0.5)
                            * (0.84 + pill.uv(surface.refracted).y.smoothstep(0.45, 1.0) * 0.1);
                        color += colors[3].to_vec3().lerp(Vec3::ONE, 0.25)
                            * speckle(uv * pill.size(), frame.time, seed, audio);
                        color *= 1.0 + caustics(uv * pill.size() / pill.size().y, frame.time, seed, audio);
                        let center = pill.center() + vec2((pill.size().x - pill.size().y) * 0.5, 0.0);
                        let radius = pill.size().y * 0.5;
                        let edge = (surface.pixel.distance(center) - radius).max(surface.sdf.distance);
                        source_over(
                            image.sample((surface.refracted - center) / (radius * 2.0) + 0.5)
                                * vec4(1.0, 1.0, 1.0, edge.smoothstep(0.0, -4.0)),
                            glass(surface, color),
                        ) * vec4(1.0, 1.0, 1.0, opacity)
                    })
            );

            let (left, right) = (18.0, width - panel_height - 8.0);
            if let Some(lines) = track_text.filter(|_| right > left) {
                for ((compact, full), y) in lines.into_iter().zip([0.26, 0.57]) {
                    let text_width = compact.text.width.lerp(full.text.width, expansion);
                    let origin = pill.min
                        + vec2(left + ((right - left - text_width) * 0.5).max(0.0), (panel_height * y).floor());
                    shader!(
                        context
                            .frame
                            .upload({
                                let pill: Rect;
                                let opacity: f32;
                                let expansion: f32;
                                let pointer_active: f32;
                                let compact: Text = context.frame.resources.place(&compact, origin);
                                let full: Text = context.frame.resources.place(&full, origin);
                            })
                            .primitive(|mut frame| {
                                frame.globals.pressure *= pointer_active;
                                refract(Shape::pill(pill), frame, compact.union(full))
                            })
                            .fragment(|_, surface| {
                                let image_center = pill.center() + vec2((pill.size().x - pill.size().y) * 0.5, 0.0);
                                if surface.refracted.x >= image_center.x {
                                    kill();
                                }
                                let alpha = (surface.refracted.distance(image_center) - pill.size().y * 0.5)
                                    .smoothstep(2.0, 18.0);
                                // Shared glyphs stay opaque while the additional text fades in.
                                let ink =
                                    compact.fill_at(surface.content).lerp(full.fill_at(surface.content), expansion);
                                TEXT_COLOR.extend(alpha * opacity * ink / surface.coverage.max(f32::MIN_POSITIVE))
                            })
                    );
                }
            }

            if let Some(track_id) = track.id {
                for (slot, playlist, secondary, is_star, alpha, center, response) in icons {
                    let alpha = alpha * opacity;
                    let mouse_distance = center.distance(mouse_pos);
                    let proximity = mouse_distance.smoothstep(ICON_WIDTH * 2.5, ICON_WIDTH * 0.25)
                        * mouse_pressure.clamp(0.0, 1.0)
                        * pointer_active;
                    let x_push = (center.x - mouse_pos.x) * proximity * 0.5;
                    let radius = ICON_WIDTH * 0.5 * (1.05 + 0.63 * proximity);
                    let quad = Quad::new(
                        center + vec2(x_push, 0.0),
                        Vec2::splat(radius * 2.0 + 14.0),
                        Vec2::from_angle(x_push * 0.01),
                    );

                    if response.clicked {
                        burst = true;
                        if let Some(playlist) = playlist {
                            playlist_toggle = Some((track_id, playlist.id));
                        } else {
                            rated_track = Some((track_id, slot as u8 * 2 + u8::from(mouse_pos.x >= center.x)));
                        }
                    }
                    if is_star && response.hovered {
                        rating = Some(slot as i32 * 2 + 1 + i32::from(mouse_pos.x >= center.x));
                    }
                    shader!(
                        context
                            .frame
                            .upload({
                                let image: &Image = playlist
                                    .and_then(|playlist| music.resources.art(playlist.image_url.as_deref()))
                                    .map_or(&*EMPTY_ART, |art| &art.image);
                                let is_star: bool;
                                let fill: f32 = (rating.unwrap_or(0) as f32 * 0.5 - slot as f32).saturate();
                                let desaturation: f32 = 0.2 * f32::from(secondary && !response.hovered);
                                let alpha: f32;
                                let quad: Quad;
                            })
                            .primitive(quad)
                            .fragment(|_, surface| {
                                let point = (surface.uv * 2.0 - 1.0) * 18.0;
                                let color = if is_star {
                                    let split = surface.uv.x - fill;
                                    shade_icon(
                                        vec3(1.0, 0.85, 0.2)
                                            .lerp(Vec3::splat(0.33), (split / split.fwidth() + 0.5).saturate()),
                                        Shape::star(5.6, 3.58).offset(1.12),
                                        point,
                                    )
                                } else {
                                    let texture = image.sample(surface.uv);
                                    shade_icon(
                                        texture.truncate().lerp(Vec3::splat(0.24), desaturation),
                                        Shape::circle(Vec2::ZERO, 6.7),
                                        point,
                                    ) * vec4(1.0, 1.0, 1.0, texture.w)
                                };
                                color * vec4(1.0, 1.0, 1.0, alpha)
                            })
                    );
                }
            }
            track.runtime.track_expansion = track
                .runtime
                .track_expansion
                .move_towards(f32::from(hovered), context.frame.delta_time.min(0.1) / 0.16);
            if Some(layout.queue_index) == playhead_index {
                self.show_playhead(context, music, bar.playhead_x);
            }
        }
        if let Some((index, duration_ms, fraction)) = seek_action {
            music.seek(index, duration_ms, fraction);
        }
        if let Some((track_id, rating)) = rated_track {
            music.rate_track(track_id, rating);
        }
        if let Some((track_id, playlist_id)) = playlist_toggle {
            music.toggle_playlist(track_id, playlist_id);
        }

        if burst {
            for particle in
                self.particles.iter_mut().filter(|particle| particle.expires_at <= context.frame.time).take(20)
            {
                *particle = Particle {
                    origin: mouse_pos,
                    velocity: Vec2::from_angle(fastrand::f32() * TAU) * (30.0 + fastrand::f32() * 20.0),
                    spawned_at: context.frame.time,
                    expires_at: context.frame.time + 0.5 + fastrand::f32(),
                    color: vec3(1.0, 0.843, 0.196),
                    ..
                };
            }
        }
        self.show_particles(context, music, bar, playhead_track.map(|(index, _)| &music.queue[index]));
    }

    fn show_particles(&mut self, context: &mut UiContext, music: &Music, bar: BarLayout, track: Option<&Track>) {
        let panel_height = context.config.height;
        // Particle emission
        let time = context.frame.time;
        if let Some(track) = track {
            let audio = music.resources.audio(track.id);
            self.particles_debt = (self.particles_debt + context.frame.delta_time * (20.0 + audio.energy * 35.0))
                * f32::from(music.timeline.movement.abs() > 0.00001);
            let emit_count = self.particles_debt.floor() as usize;
            self.particles_debt -= emit_count as f32;
            let horizontal_bias =
                (music.timeline.movement.abs().powf(0.2) * music.timeline.movement.signum()).clamp(-3.0, 3.0);
            let palette = music.resources.palette(track.image.as_deref());
            for particle in self.particles.iter_mut().filter(|particle| particle.expires_at <= time).take(emit_count) {
                let y = fastrand::f32();
                *particle = Particle {
                    origin: vec2(bar.playhead_x, PANEL_START + panel_height * (0.1 + y * 0.85)),
                    velocity: vec2(
                        (25.0 + fastrand::f32() * 20.0 + audio.energy * 30.0) * horizontal_bias,
                        (y - 0.5) * (8.0 + audio.danceability * 30.0),
                    ),
                    spawned_at: time,
                    expires_at: time + 0.8 + audio.acousticness * 0.8 + fastrand::f32() * 0.3,
                    color: palette[fastrand::usize(0..palette.len())].to_vec3(),
                    size: vec2(7.0 + audio.acousticness * 9.0, 3.0 + audio.energy * 4.0),
                };
            }
        } else {
            self.particles_debt = 0.0;
        }

        for particle in self.particles.iter().filter(|particle| particle.expires_at > time) {
            let elapsed = time - particle.spawned_at;
            let age = elapsed / (particle.expires_at - particle.spawned_at);
            shader!(
                context
                    .frame
                    .blend(Blend::Add)
                    .upload({
                        let quad: Quad = Quad::oriented(
                            particle.origin + particle.velocity * elapsed,
                            particle.size * (age + 0.5),
                            particle.velocity,
                        );
                        let color: Vec4 = particle.color.extend((1.0 - age) * elapsed.smoothstep(0.0, 0.15));
                    })
                    .primitive(quad)
                    .fragment(|_, surface| {
                        // Lighten and brighten the source colour for additive sparks.
                        (color.truncate().lerp(Vec3::ONE, 0.2) * 2.0)
                            .extend(color.w * (1.0 - (surface.uv * 2.0 - 1.0).length_squared()).max(0.0).powi(3))
                    })
            );
        }
    }

    fn show_playhead(&mut self, context: &mut UiContext, music: &Music, x: f32) {
        let panel_height = context.config.height;
        // Playhead interaction and rendering
        let half_width = panel_height * 0.4;
        let playhead = Rect::from_center_size(vec2(x, PANEL_START + panel_height * 0.5), Vec2::splat(half_width * 2.0));
        context.interaction.input_region(playhead);
        let response = context.interaction.interact("playhead", Shape::rectangle(playhead));
        let speed = context.frame.delta_time * 5.5;
        let last_toggle = music.last_toggle.elapsed().as_secs_f32() / 0.7;
        if !response.hovered && music.playing && last_toggle < 1.0 {
            self.bar_split = 1.0 - last_toggle;
            self.icon_presence = 1.0 - last_toggle;
            self.icon_morph = self.icon_morph.move_towards(1.0, speed * 1.5);
        } else {
            let show_icon = f32::from(response.hovered || !music.playing);
            self.bar_split = self.bar_split.move_towards(show_icon, speed);
            self.icon_presence = self.icon_presence.max(show_icon).move_towards(show_icon, speed);
            self.icon_morph = self.icon_morph.move_towards(f32::from(response.hovered && !music.playing), speed);
        }
        if response.clicked {
            music.toggle_playing();
        }
        shader!(
            context
                .frame
                .upload({
                    let bar_split: f32 = self.bar_split;
                    let icon_presence: f32 = self.icon_presence;
                    let icon_morph: f32 = self.icon_morph;
                    let rect: Rect = Rect::new(
                        vec2(x - half_width, PANEL_START - 5.0),
                        vec2(x + half_width, PANEL_START + panel_height + 5.0),
                    );
                })
                .primitive(rect)
                .fragment(|frame, surface| {
                    let local = rect.local(surface.pixel);
                    let mirrored = local.abs();
                    let bar_len = frame.globals.bar_height * (0.5 - 0.375 * bar_split);
                    let bar = Shape::pill(vec2(bar_len + 9.0, 9.0))
                        .sample_at(vec2(mirrored.y - (frame.globals.bar_height - bar_len) * 0.5, mirrored.x));
                    let pause = Shape::pill(vec2(7.0, frame.globals.bar_height * 0.2 + 7.0))
                        .translated(vec2(4.0 * bar_split, 0.0));
                    let play_scale = frame.globals.bar_height * 0.18 * (1.0 + icon_morph * (1.0 - icon_presence));
                    let play = Shape::rounded_triangle(play_scale, play_scale * 0.5);
                    let icon = Sample::new(
                        pause.distance_at(local.abs()).lerp(play.distance_at(local.perp()), icon_morph),
                        0.0,
                    );
                    let alpha = (icon.fill() * icon_presence).max(bar.fill());
                    if alpha <= 0.0 {
                        kill();
                    }
                    let color = vec3(1.0, 0.878, 0.824)
                        .lerp(Vec3::splat(0.15), bar.distance.min(icon.distance).smoothstep(-2.5, -1.0));
                    color.extend(alpha)
                })
        );
    }
}
