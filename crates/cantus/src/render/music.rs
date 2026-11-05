use crate::{
    music::{Music, PlaylistId, TRACK_SPACING_MS, Track, TrackId},
    render::{
        BarLayout, GAP, HISTORY_WIDTH, PANEL_START, Program, SUPPORT_BLEND, SUPPORT_INSET, TEXT_BODY, TEXT_COLOR,
        TEXT_TITLE, UiContext,
        sdf::{deform, glass_masked, hash, refract},
        support_row,
    },
};
use core::{
    f32::consts::{FRAC_PI_2, PI, TAU},
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
const ICON_WIDTH: f32 = 24.0;
/// Center-to-center icon spacing for rating stars and playlist artwork.
const ICON_SPACING: f32 = 20.0;
/// Height added below the pill for ratings and saved playlists.
const PRIMARY_SUPPORT_DEPTH: f32 = 8.0;
/// Height added below the first icon row for other playlists.
const SECONDARY_SUPPORT_DEPTH: f32 = 20.0;
/// Space reserved for each collapsed history track, excluding its gap.
const HISTORY_TRACK_WIDTH: f32 = 12.0;
/// Corner radius of an unclipped track edge, as a fraction of bar height.
const TRACK_RADIUS: f32 = 0.4;
/// Fraction of the bar height spanned by the playhead line.
const PLAYHEAD_HEIGHT: f32 = 0.8;
/// Transparent texture used while artwork is unavailable and for rating icons.
static EMPTY_ART: LazyLock<Image> = LazyLock::new(|| Image::rgba8([1, 1], vec![0; 4]));

struct TrackLayout {
    queue_index: usize,
    start_ms: f32,
    width: f32,
    right: f32,
    /// Pixels of the track cut off past the right end of the timeline.
    overhang_right: f32,
}

impl TrackLayout {
    fn new(index: usize, duration_ms: u32, start_ms: f32, bar: BarLayout, future_end: f32, left: f32) -> Self {
        let natural_start = bar.playhead_x + start_ms * bar.px_per_ms;
        let end = natural_start + duration_ms as f32 * bar.px_per_ms;
        let clipped = end.min(future_end) - natural_start.max(left);
        let width = clipped.max(if natural_start < left { HISTORY_TRACK_WIDTH } else { 0.0 });
        Self {
            queue_index: index,
            start_ms,
            width,
            right: end.clamp(left, future_end),
            overhang_right: (end - future_end).max(0.0),
        }
    }
}

/// Layout shared by every track drawn in one frame.
struct TrackFrame {
    bar: BarLayout,
    tracks_left: f32,
    future_end: f32,
}

/// Interactions collected while drawing tracks, applied once all of them are drawn.
#[derive(Default)]
struct Actions {
    burst: bool,
    seek: Option<(usize, f32)>,
    rated: Option<(TrackId, u8)>,
    playlist_toggle: Option<(TrackId, PlaylistId)>,
}

/// Mirror the future-edge fade: keep the minimum-size card inside the boundary
/// while its remaining timeline span fades away.
fn history_visibility(right: f32, boundary: f32, height: f32) -> f32 {
    (right - boundary).smoothstep(0.0, height)
}

fn shadow_fade(point: Vec2, viewport: Rect) -> f32 {
    (point.x - viewport.min.x + GAP).smoothstep(0.0, GAP)
}

fn history_collapse(size: Vec2, history_depth: f32) -> f32 {
    let tween = size.y * 0.16;
    history_depth.smoothstep(0.0, tween) * (1.0 - (size.x - size.y).smoothstep(0.0, tween))
}

/// Integrates bounded motion rather than chasing a target, so a fast scrub
/// cannot leave a backlog of animation after the pointer stops.
#[derive(Clone, Default)]
pub struct DropletMotion {
    previous: Option<Vec2>,
    progress: Vec2,
}

impl DropletMotion {
    fn advance(&mut self, target: Vec2, delta_time: f32) -> Vec2 {
        if let Some(previous) = self.previous {
            let limit = Vec2::splat(delta_time.clamp(0.0, 0.1) * 2.0);
            self.progress += (target - previous).clamp(-limit, limit);
        } else {
            self.progress = target;
        }
        self.previous = Some(target);
        self.progress
    }
}

/// Two overlapping droplets per band keep all six bands of each edge alive.
/// Paths and shape variations are prepared once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderData)]
struct Drop {
    center: Vec2,
    radii: Vec2,
    axis: Vec2,
    opacity: f32,
    skew: f32,
}

#[derive(Clone, Copy, Default, ShaderData)]
struct Droplets {
    balls: [Drop; 24],
    count: u32,
    vertical_limits: Vec2,
}

impl Droplets {
    fn new(pill: Rect, rounding: Vec2, strength: Vec2, seed: f32, progress: Vec2) -> Self {
        let mut result = Self { vertical_limits: vec2(pill.min.y, pill.max.y), ..Self::default() };
        let height = pill.size().y;
        let body = track_shape(pill, rounding);
        for side in 0..2 {
            let strength = strength[side];
            if strength == 0.0 {
                continue;
            }
            let direction = if side == 0 { -1.0 } else { 1.0 };
            let edge = if side == 0 { pill.min.x } else { pill.max.x };
            for index in 0..12 {
                let band = index / 2;
                let clock = progress[side] + band as f32 * 0.618_034 + (index % 2) as f32 * 0.5 + seed;
                let phase = clock - clock.floor();
                let random = hash(vec2(seed * 65536.0 + band as f32 * 17.0, clock.floor() * 31.0 + side as f32 * 97.0));
                let arc = ((band as f32 + 0.5) / 6.0 - 0.5) * PI * 0.64 + (random.y - 0.5) * 0.03;
                let y = pill.center().y + height * 0.5 * arc.sin();
                let (mut inside, mut outside) = (edge - direction * height.min(pill.size().x) * 0.5, edge);
                for _ in 0..6 {
                    let middle = f32::midpoint(inside, outside);
                    if body.contains(vec2(middle, y)) {
                        inside = middle;
                    } else {
                        outside = middle;
                    }
                }
                let angle = (0.6 + random.y * 0.4) * (PI / 9.0) * if band < 3 { -1.0 } else { 1.0 };
                let center =
                    vec2(inside, y) + vec2(direction * angle.cos(), angle.sin()) * height * (phase * 0.38 - 0.08);
                let envelope = (phase * PI).sin();
                let alpha = (1.0 - phase).smoothstep(0.0, 0.7) * envelope.smoothstep(0.0, 0.2) * strength;
                let radius = height * (0.08 + random.x * 0.017) * envelope;
                let stretch = 0.97 + random.y * 0.06;
                result.balls[result.count as usize] = Drop {
                    center,
                    radii: vec2(radius * stretch, radius / stretch),
                    axis: vec2(direction * angle.cos(), angle.sin()),
                    opacity: alpha,
                    skew: (random.x - 0.5) * 0.12,
                };
                result.count += 1;
            }
        }
        result
    }

    /// The same field defines both the contour and its material transparency.
    fn field(self, base: f32, point: Vec2, height: f32) -> Vec2 {
        let blend = height * 0.22;
        let mut distance = base;
        let mut alpha = 1.0 - base.smoothstep(0.0, blend * 0.25);
        for index in 0..self.count as usize {
            let ball = self.balls[index];
            if ball.opacity == 0.0 {
                continue;
            }
            let offset = point - ball.center;
            let local = vec2(offset.dot(ball.axis), offset.dot(ball.axis.perp())) / ball.radii;
            let droplet = (local.length() - 1.0 - ball.skew * local.x.clamp(-1.0, 1.0)) * ball.radii.min_element();
            let neck = (1.0 - (base - droplet).abs() / blend).saturate();
            let merged = base.min(droplet) - neck * neck * blend * 0.25;
            distance = distance.min(base.lerp(merged, ball.opacity.smoothstep(0.0, 0.2)));
            alpha = alpha.max(ball.opacity * (1.0 - (droplet - distance).abs() / blend).saturate());
        }
        let edge_margin = (point.y - self.vertical_limits.x).min(self.vertical_limits.y - point.y);
        distance = base.lerp(distance, edge_margin.smoothstep(0.0, height * 0.15));
        let fade = 1.0 - base.smoothstep(0.0, height * 0.32);
        vec2(distance, 1.0f32.lerp(alpha * fade, (base - distance).smoothstep(0.0, 0.5)))
    }

    fn shape(self, body: impl Sdf, height: f32) -> impl Sdf {
        let reach = if self.count == 0 { 0.0 } else { height * 0.4 };
        Shape::from_fn(body.bounds(reach), move |point| self.field(body.distance_at(point), point, height).x)
    }
}

/// Squircle corners on complete edges blend into circular ends as they are clipped.
fn track_shape(rect: Rect, rounding: Vec2) -> impl Sdf {
    // Narrow incoming cards keep the curved ends and full height as they grow.
    let scale = vec2((rect.size().y / rect.size().x.max(0.001)).max(1.0), 1.0);
    Shape::from_fn(rect, move |point| {
        let local = (point - rect.center()) * scale;
        let amount = if local.x < 0.0 { rounding.x } else { rounding.y };
        let radius = rect.size().y * TRACK_RADIUS.lerp(0.5, amount);
        let corner = local.abs() - rect.size() * scale * 0.5 + radius;
        let outside = corner.max(Vec2::ZERO);
        ((outside * outside).length().sqrt().lerp(outside.length(), amount) + corner.max_element().min(0.0) - radius)
            / scale.x
    })
}

fn music_shape(pill: Rect, rounding: Vec2, icon_supports: [Vec2; 2]) -> impl Sdf {
    let support = |index: usize| {
        Shape::pill(Rect::from_center_size(
            vec2(
                pill.center().x,
                pill.max.y - SUPPORT_INSET + index as f32 * PRIMARY_SUPPORT_DEPTH + icon_supports[index].y * 0.5,
            ),
            icon_supports[index],
        ))
    };
    track_shape(pill, rounding)
        .smooth_union(support(0), SUPPORT_BLEND, icon_supports[0].y / PRIMARY_SUPPORT_DEPTH)
        .smooth_union(support(1), SUPPORT_BLEND, icon_supports[1].y / SECONDARY_SUPPORT_DEPTH)
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

/// Drifting water caustics: thin bright filaments where iterated sine warps pinch together.
fn caustics(p: Vec2, time: f32, seed: f32, audio: AudioFeatures) -> f32 {
    const SHARPNESS: f32 = 0.005;
    let base = p * 2.2 + Vec2::splat(-250.0) + vec2(seed * 7.0, seed * 3.0);
    let time = time * 0.45 + seed * 30.0 + (time * 0.2).sin() * audio.energy;
    let mut warped = base;
    let mut density = 1.0;
    for step in 0..5 {
        let t = time * (1.0 - 3.5 / (step as f32 + 1.0));
        warped = base + vec2((t - warped.x).cos() + (t + warped.y).sin(), (t - warped.y).sin() + (t + warped.x).cos());
        density += 1.0
            / vec2(base.x / ((warped.x + t).sin() / SHARPNESS), base.y / ((warped.y + t).cos() / SHARPNESS)).length();
    }
    let filament = (1.17 - (density / 5.0).powf(1.4)).abs().powi(8);
    filament.min(1.0) * audio.instrumentalness * 0.6
}

impl MusicView {
    /// Draws the play-playlist tiles at the left edge, returning where the tracks start and whether one was clicked.
    fn show_playlist_buttons(&mut self, context: &mut UiContext, music: &Music) -> (f32, bool) {
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
                let rows = (count + 1 - index / 3 * 3).min(3);
                let center = vec2(
                    GAP + (index / 3) as f32 * button_size + button_size * 0.5,
                    PANEL_START + ((index % 3) as f32 + 0.5 + (3 - rows) as f32 * 0.5) * button_size,
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
        (tracks_left, burst)
    }

    pub fn show(&mut self, context: &mut UiContext, music: &mut Music, bar: BarLayout) {
        let (tracks_left, burst) = self.show_playlist_buttons(context, music);
        let playhead_track = music
            .queue
            .span_at_playhead()
            .filter(|(index, elapsed)| *elapsed < music.queue.tracks[*index].duration_ms as f32);

        let future_end = HISTORY_WIDTH + context.config.timeline_future_minutes * 60_000.0 * bar.px_per_ms;
        let gap = TRACK_SPACING_MS * bar.px_per_ms;
        let mut start_ms = music.queue.timeline.start_ms + music.queue.offset_ms(music.queue.tracks.len());
        let mut next_left = None;
        let mut visible = SmallVec::<[_; 16]>::new();
        for (queue_index, track) in music.queue.tracks.iter_mut().enumerate().rev() {
            start_ms -= f64::from(track.queue_span_ms());
            let mut layout =
                TrackLayout::new(queue_index, track.duration_ms, start_ms as f32, bar, future_end, HISTORY_WIDTH);
            // Queue snapshots arrive before metadata and artwork. Keep unresolved entries out
            // of layout and interaction instead of drawing minimum-size placeholder circles.
            if track.duration_ms == 0
                || track.name.trim().is_empty()
                || track.image.is_some() && music.resources.art(track.image.as_deref()).is_none()
                || bar.playhead_x + start_ms as f32 * bar.px_per_ms >= future_end
            {
                track.runtime.track_expansion = 0.0;
                continue;
            }
            let gap = if layout.right <= HISTORY_WIDTH { gap } else { gap.min(track.queue_span_ms() * bar.px_per_ms) };
            if let Some(left) = next_left {
                layout.right = left - gap;
            }
            next_left = Some(layout.right - layout.width);
            visible.push(layout);
        }
        visible.reverse();

        let mouse_pos = context.interaction.mouse_pos();
        let frame = TrackFrame { bar, tracks_left, future_end };
        let mut actions = Actions { burst, ..Default::default() };
        visible.sort_by_key(|layout| music.queue.tracks[layout.queue_index].runtime.track_expansion.to_bits());
        for layout in visible {
            Self::show_track(context, music, &frame, &layout, &mut actions);
        }
        let Actions { burst, seek, rated, playlist_toggle } = actions;
        if let Some((index, position)) = seek {
            music.seek(index, position);
        }
        if let Some((track_id, rating)) = rated {
            music.rate_track(track_id, rating);
        }
        if let Some((track_id, playlist_id)) = playlist_toggle {
            music.toggle_playlist(track_id, playlist_id);
        }

        if !music.queue.tracks.is_empty() {
            self.show_playhead(context, music, bar.playhead_x);
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
        self.show_particles(context, music, bar, playhead_track.map(|(index, _)| &music.queue.tracks[index]));
    }

    fn show_track(
        context: &mut UiContext,
        music: &mut Music,
        TrackFrame { bar, tracks_left, future_end }: &TrackFrame,
        layout: &TrackLayout,
        actions: &mut Actions,
    ) {
        let (bar, tracks_left, future_end) = (*bar, *tracks_left, *future_end);
        let panel_height = context.config.height;
        let mouse_pos = context.interaction.mouse_pos();
        let mouse_pressure = context.interaction.pressure();
        let track = &mut music.queue.tracks[layout.queue_index];
        let scale = bar.px_per_ms;
        let natural_start = bar.playhead_x + layout.start_ms * scale;
        if layout.right <= tracks_left {
            track.runtime.track_expansion = 0.0;
            return;
        }
        let history_alpha = history_visibility(layout.right, tracks_left, panel_height);
        let opacity = (future_end - natural_start).smoothstep(0.0, panel_height);
        if opacity * history_alpha <= 1.0 / 1024.0 {
            track.runtime.track_expansion = 0.0;
            return;
        }
        let can_expand = layout.width > panel_height;
        if !can_expand {
            track.runtime.track_expansion = 0.0;
        }
        let mut width = if opacity < 1.0 { layout.width } else { layout.width.max(panel_height) };
        let mut x = layout.right - width;
        let pill_top = PANEL_START;
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
                (track.compact_title(), track.name.as_str(), TEXT_TITLE),
                (compact_details.as_str(), full_details.as_str(), TEXT_BODY),
            ]
            .map(|(compact, full, size)| {
                let compact = context.frame.resources.shape(compact, size, 700.0);
                let full = context.frame.resources.shape(full, size, 700.0);
                (context.frame.resources.cached_distance(&compact), context.frame.resources.cached_distance(&full))
            })
        });
        if let Some(lines) = &track_text {
            let target = lines.iter().map(|(_, full)| full.text.width).fold(0.0, f32::max) + panel_height + 26.0;
            let extra_width = (target - width).max(0.0) * expansion;
            x -= extra_width * 0.5;
            width += extra_width;
        }
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
        x = x.max(tracks_left);
        let pill = Rect::new(vec2(x, pill_top), vec2(x + width, pill_top + panel_height));
        // Clipping rounds each end; hover expansion restores the complete squircle shape.
        let clipping = vec2((HISTORY_WIDTH - natural_start).max(0.0), layout.overhang_right);
        // Normal playback runs at 0.625 cycles/second; scrubbing is capped at 2.
        let droplet_progress =
            track.runtime.droplet_motion.advance(clipping / (scale * 1600.0).max(0.001), context.frame.delta_time);
        let collapse = history_collapse(pill.size(), HISTORY_WIDTH - natural_start);
        let rounding = (clipping.smoothstep(Vec2::ZERO, Vec2::splat(panel_height * 2.0)) * (1.0 - expansion))
            .lerp(Vec2::ONE, collapse);
        let droplet_strength =
            clipping.smoothstep(Vec2::ZERO, Vec2::splat(panel_height * 0.3)) * (1.0 - expansion) * (1.0 - collapse);
        let viewport = Rect::new(vec2(tracks_left, 0.0), context.frame.screen_size);
        let visible_area = Shape::rectangle(viewport);
        let shape = music_shape(pill, rounding, icon_supports);
        let hit_shape = shape.offset(if expansion > 0.0 { 12.0 } else { 0.0 });
        let bounds = hit_shape.bounds(0.0);
        let (min, max) = (bounds.min, bounds.max);
        // Halve the search interval until it is at most half a pixel wide.
        let steps = bounds.size().x.max(1.0).log2().ceil() as u32;
        for y in min.y.floor() as i32..max.y.ceil() as i32 {
            let point = vec2(pill.center().x, y as f32 + 0.5);
            if !hit_shape.contains(point) {
                continue;
            }
            let (mut inside, mut outside) = (0.0, bounds.size().x * 0.5);
            for _ in 0..steps {
                let middle = f32::midpoint(inside, outside);
                if hit_shape.contains(point + vec2(middle, 0.0)) {
                    inside = middle;
                } else {
                    outside = middle;
                }
            }
            let region = Rect::from_center_size(point, vec2(inside * 2.0, 1.0));
            context.interaction.input_region(region.intersection(visible_area.bounds(0.0)));
        }
        let body = context.interaction.drag(track.interaction_id, hit_shape.intersection(visible_area));
        let mut hovered = body.hovered;
        if body.clicked && track.duration_ms > 0 {
            let fraction = if natural_start + track.duration_ms as f32 * scale <= HISTORY_WIDTH
                || layout.queue_index == music.queue.index && mouse_pos.x <= x + pill.size().x * 0.05
            {
                0.0
            } else {
                ((mouse_pos.x - natural_start) / (track.duration_ms as f32 * scale)).saturate()
            };
            actions.seek = Some((layout.queue_index, track.duration_ms as f32 * fraction));
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
                    support_row(pill).y + f32::from(secondary) * ICON_SPACING * spread,
                );
                let response = context.interaction.interact(
                    ("music-icon", track.interaction_id, is_star.then_some(slot), playlist.map(|playlist| playlist.id)),
                    Shape::circle(center, ICON_WIDTH * 0.5).intersection(visible_area),
                );
                hovered |= response.hovered;
                if is_star && response.hovered {
                    rating = Some(slot as i32 * 2 + 1 + i32::from(mouse_pos.x >= center.x));
                }
                icons.push((slot, playlist, secondary, is_star, alpha, center, response));
            }
        }
        let pointer_active = f32::from(hovered);
        let opacity = (if hovered { 1.0 } else { opacity.lerp(1.0, expansion) }) * history_alpha;

        shader!(
            context
                .frame
                .upload({
                    let viewport: Rect;
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
                    let rounding: Vec2;
                    let droplets: Droplets = Droplets::new(pill, rounding, droplet_strength, seed, droplet_progress);
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
                    deform(droplets.shape(music_shape(pill, rounding, icon_supports), pill.size().y), frame)
                })
                .fragment(|frame, surface| {
                    let shadow = shadow_fade(surface.pixel, viewport);
                    if shadow <= 0.0 {
                        kill();
                    }
                    let face = if Shape::rectangle(viewport).contains(surface.pixel) { 1.0 } else { 0.0 };
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
                    color +=
                        colors[3].to_vec3().lerp(Vec3::ONE, 0.25) * speckle(uv * pill.size(), frame.time, seed, audio);
                    color *= 1.0 + caustics(uv * pill.size() / pill.size().y, frame.time, seed, audio);
                    let center = pill.center() + vec2((pill.size().x - pill.size().y) * 0.5, 0.0);
                    let radius = f32::midpoint(pill.size().y, surface.bulge);
                    // Follow the animated contour halfway from the original
                    // crop, retaining a slight inset around the glass rim.
                    let original_crop =
                        track_shape(Rect::from_center_size(center, Vec2::splat(radius * 2.0)), vec2(1.0, rounding.y))
                            .distance_at(surface.pixel)
                            .max(surface.sdf.distance);
                    let edge = original_crop.lerp(surface.sdf.distance, 0.5) + 1.5;
                    let art_uv = (surface.refracted - center) / (radius * 2.0) + 0.5;
                    let art_alpha = edge.smoothstep(0.0, -1.0) * art_uv.x.smoothstep(0.0, 0.08) * face;
                    let base = music_shape(pill, rounding, icon_supports).distance_at(surface.pixel);
                    let alpha = opacity * droplets.field(base, surface.pixel, pill.size().y).y;
                    source_over(
                        image.sample(art_uv) * vec4(1.0, 1.0, 1.0, art_alpha),
                        glass_masked(surface, color, vec2(face, shadow)),
                    ) * vec4(1.0, 1.0, 1.0, alpha)
                })
        );

        let (left, right) = (18.0, width - panel_height - 8.0);
        if let Some(lines) = track_text.filter(|_| right > left) {
            for ((compact, full), y) in lines.into_iter().zip([0.26, 0.57]) {
                let text_width = compact.text.width.lerp(full.text.width, expansion);
                let origin =
                    pill.min + vec2(left + ((right - left - text_width) * 0.5).max(0.0), (panel_height * y).floor());
                shader!(
                    context
                        .frame
                        .upload({
                            let pill: Rect;
                            let rounding: Vec2;
                            let viewport: Rect;
                            let opacity: f32;
                            let expansion: f32;
                            let pointer_active: f32;
                            let compact: Text = context.frame.resources.place(&compact, origin);
                            let full: Text = context.frame.resources.place(&full, origin);
                        })
                        .primitive(|mut frame| {
                            frame.globals.pressure *= pointer_active;
                            refract(track_shape(pill, rounding), frame, compact.union(full))
                        })
                        .fragment(|_, surface| {
                            if !Shape::rectangle(viewport).contains(surface.pixel) {
                                kill();
                            }
                            let image_center = pill.center() + vec2((pill.size().x - pill.size().y) * 0.5, 0.0);
                            if surface.refracted.x >= image_center.x {
                                kill();
                            }
                            let alpha = (surface.refracted.distance(image_center)
                                - f32::midpoint(pill.size().y, surface.bulge))
                            .smoothstep(2.0, 18.0);
                            // Shared glyphs stay opaque while the additional text fades in.
                            let ink = if expansion <= 0.0 {
                                compact.fill_at(surface.content)
                            } else if expansion >= 1.0 {
                                full.fill_at(surface.content)
                            } else {
                                compact.fill_at(surface.content).lerp(full.fill_at(surface.content), expansion)
                            };
                            TEXT_COLOR.extend(alpha * opacity * ink / surface.coverage.max(f32::MIN_POSITIVE))
                        })
                );
            }
        }

        if let Some(track_id) = track.id {
            for (slot, playlist, secondary, is_star, alpha, center, response) in icons {
                let alpha = alpha * opacity;
                let proximity = center.distance(mouse_pos).smoothstep(ICON_WIDTH * 2.5, ICON_WIDTH * 0.25)
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
                    actions.burst = true;
                    if let Some(playlist) = playlist {
                        actions.playlist_toggle = Some((track_id, playlist.id));
                    } else {
                        actions.rated = Some((track_id, slot as u8 * 2 + u8::from(mouse_pos.x >= center.x)));
                    }
                }
                shader!(
                    context
                        .frame
                        .upload({
                            let image: &Image = playlist
                                .and_then(|playlist| music.resources.art(playlist.image_url.as_deref()))
                                .map_or(&*EMPTY_ART, |art| &art.image);
                            let viewport: Rect;
                            let is_star: bool;
                            let fill: f32 = (rating.unwrap_or(0) as f32 * 0.5 - slot as f32).saturate();
                            let desaturation: f32 = 0.2 * f32::from(secondary && !response.hovered);
                            let alpha: f32;
                            let quad: Quad;
                        })
                        .primitive(quad)
                        .fragment(|_, surface| {
                            if !Shape::rectangle(viewport).contains(surface.pixel) {
                                kill();
                            }
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
            .move_towards(f32::from(hovered && can_expand), context.frame.delta_time.min(0.1) / 0.16);
    }

    fn show_particles(&mut self, context: &mut UiContext, music: &Music, bar: BarLayout, track: Option<&Track>) {
        let panel_height = context.config.height;
        let time = context.frame.time;
        if let Some(track) = track {
            let audio = music.resources.audio(track.id);
            self.particles_debt = (self.particles_debt + context.frame.delta_time * (20.0 + audio.energy * 35.0))
                * f32::from(music.queue.timeline.movement.abs() > 0.00001);
            let emit_count = self.particles_debt.floor() as usize;
            self.particles_debt -= emit_count as f32;
            let horizontal_bias = (music.queue.timeline.movement.abs().powf(0.2)
                * music.queue.timeline.movement.signum())
            .clamp(-3.0, 3.0);
            let palette = music.resources.palette(track.image.as_deref());
            for particle in self.particles.iter_mut().filter(|particle| particle.expires_at <= time).take(emit_count) {
                let y = fastrand::f32();
                *particle = Particle {
                    origin: vec2(bar.playhead_x, PANEL_START + panel_height * (0.5 + (y - 0.5) * 0.68)),
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
        let playing = music.playing;
        let panel_height = context.config.height;
        let half_width = panel_height * 0.4;
        let playhead = Rect::from_center_size(vec2(x, PANEL_START + panel_height * 0.5), Vec2::splat(half_width * 2.0));
        context.interaction.input_region(playhead);
        let response = context.interaction.interact("playhead", Shape::rectangle(playhead));
        let speed = context.frame.delta_time * 5.5;
        let last_toggle = music.last_toggle.elapsed().as_secs_f32() / 0.7;
        if !response.hovered && playing && last_toggle < 1.0 {
            self.bar_split = 1.0 - last_toggle;
            self.icon_presence = 1.0 - last_toggle;
            self.icon_morph = self.icon_morph.move_towards(1.0, speed * 1.5);
        } else {
            let show_icon = f32::from(response.hovered || !playing);
            self.bar_split = self.bar_split.move_towards(show_icon, speed);
            self.icon_presence = self.icon_presence.max(show_icon).move_towards(show_icon, speed);
            self.icon_morph = self.icon_morph.move_towards(f32::from(response.hovered && !playing), speed);
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
                    let span = frame.globals.bar_height * PLAYHEAD_HEIGHT;
                    let bar_len = span * (0.5 - 0.375 * bar_split);
                    let bar = Shape::pill(vec2(bar_len + 9.0, 9.0))
                        .sample_at(vec2(mirrored.y - (span - bar_len) * 0.5, mirrored.x));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_compaction_tweens_before_crossing_the_stack_boundary() {
        let bar = BarLayout { playhead_x: HISTORY_WIDTH, px_per_ms: 0.1 };
        for (remaining, expected) in [(60.0, 0.0), (54.0, 0.5), (50.0, 1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let start = (remaining - 100.0) / bar.px_per_ms;
            let layout = TrackLayout::new(0, 1000, start, bar, HISTORY_WIDTH + 500.0, HISTORY_WIDTH);
            let collapse = history_collapse(vec2(layout.width.max(50.0), 50.0), -start * bar.px_per_ms);
            assert!((collapse - expected).abs() < 0.001);
        }
        assert!((history_collapse(Vec2::splat(50.0), 4.0) - 0.5).abs() < 0.001);
    }

    #[test]
    fn track_shadows_fade_only_at_playlist_edge() {
        let viewport = Rect::new(vec2(100.0, 0.0), vec2(500.0, 120.0));
        for (x, alpha) in [(100.0 - GAP, 0.0), (100.0 - GAP * 0.5, 0.5), (100.0, 1.0)] {
            for y in [0.0, 60.0, 120.0] {
                assert_eq!(shadow_fade(vec2(x, y), viewport), alpha);
            }
        }
    }

    #[test]
    fn droplet_motion_caps_scrubbing_without_a_catch_up_backlog() {
        for hz in [30, 60, 165] {
            let mut motion = DropletMotion::default();
            let dt = 1.0 / hz as f32;
            motion.advance(Vec2::ZERO, dt);
            let direction = vec2(1.0, -1.0);
            for step in 1..=hz {
                let target = direction * step as f32 * dt * 0.625;
                assert_eq!(motion.advance(target, dt), target);
            }
            let start = motion.progress;
            for step in 1..=hz {
                motion.advance(direction * step as f32 * 1000.0, dt);
            }
            let stopped = motion.progress;
            assert!((stopped - start - direction * 2.0).abs().max_element() < 0.000_01);
            for _ in 0..hz {
                assert_eq!(motion.advance(direction * hz as f32 * 1000.0, dt), stopped);
            }
            let reversed = motion.advance(Vec2::ZERO, dt);
            assert!((reversed - (stopped - direction * 2.0 * dt)).abs().max_element() < 0.000_01);
        }
    }

    #[test]
    fn droplets_cover_the_edge_continuously_and_fade() {
        let mut translucent = false;
        for (height, seed) in [(20.0, 0.0), (50.0, 0.371), (80.0, 0.99)] {
            let pill = Rect::from_center_size(vec2(200.0, 100.0), vec2(240.0, height));
            let body = track_shape(pill, Vec2::ONE);
            let build = |progress| Droplets::new(pill, Vec2::ONE, Vec2::ONE, seed, progress);
            let compact = Rect::from_center_size(pill.center(), Vec2::splat(height));
            let strength = Vec2::splat(1.0 - history_collapse(compact.size(), height));
            let square = Droplets::new(compact, Vec2::ONE, strength, seed, Vec2::ONE);
            assert_eq!(history_collapse(compact.size(), -height), 0.0);
            let partial = history_collapse(vec2(height * 1.08, height), height);
            assert!(partial > 0.0 && partial < 1.0);
            assert!(Droplets::new(compact, Vec2::ONE, Vec2::ONE, seed, Vec2::ONE).count > 0);
            assert!(square.balls.iter().all(|ball| *ball == Drop::default()));
            for step in 0..64 {
                let progress = Vec2::splat(step as f32 / 16.0 - 2.0);
                let (drops, next) = (build(progress), build(progress + 0.000_01));
                assert_eq!(drops.balls, build(progress).balls);
                assert_ne!(drops.balls, build(progress + 1.0).balls);
                let mut covered = 0;
                for side in [-1.0, 1.0] {
                    for row in 0..40 {
                        let cap = Vec2::from_angle((row as f32 / 39.0 - 0.5) * PI * 0.70) * height * 0.5;
                        let point = pill.center() + vec2(side * ((pill.size().x - height) * 0.5 + cap.x), cap.y);
                        let base = body.distance_at(point);
                        let sample = drops.field(base, point, height);
                        assert!(sample.x.is_finite() && (0.0..=1.0).contains(&sample.y));
                        assert!((sample.x - next.field(base, point, height).x).abs() < 0.05);
                        covered += usize::from(base - sample.x > height * 0.005);
                    }
                }
                assert!(covered >= 64, "only {covered}/80 edge samples affected at {progress:?}, height {height}");
                assert_eq!(drops.field(body.distance_at(pill.center()), pill.center(), height).y, 1.0);
                for ball in drops.balls {
                    let point = ball.center;
                    let base = body.distance_at(point);
                    let sample = drops.field(base, point, height);
                    translucent |= sample.x < 0.0 && sample.y > 0.0 && sample.y < 0.8;
                    if base >= height * 0.16 && sample.x < 0.0 {
                        assert!(sample.y <= 0.501, "distant droplet remained opaque");
                    }
                    if sample.x < 0.0 {
                        assert!(Shape::rectangle(drops.shape(body, height).bounds(0.0)).contains(point));
                    }
                    for y in [pill.min.y - 0.5, pill.max.y + 0.5] {
                        let point = vec2(ball.center.x, y);
                        let base = body.distance_at(point);
                        assert_eq!(drops.field(base, point, height).x, base, "droplet increased the pill height");
                    }
                }
            }
        }
        assert!(translucent, "detached droplets never became translucent");
    }

    #[test]
    fn narrow_incoming_tracks_keep_full_height_and_curved_corners() {
        for width in [2.0, 8.0, 25.0, 50.0] {
            let pill = Rect::from_center_size(vec2(200.0, 100.0), vec2(width, 50.0));
            let shape = track_shape(pill, Vec2::ONE);
            assert!(shape.contains(pill.center() + vec2(0.0, 24.0)));
            assert!(shape.contains(pill.center() + vec2(width * 0.45, 0.0)));
            assert!(!shape.contains(pill.center() + vec2(width * 0.45, 24.0)));
        }
    }

    #[test]
    fn track_shapes_remain_finite_at_different_bar_heights() {
        for height in [20.0, 32.0, 40.0, 50.0, 80.0] {
            let pill = Rect::from_center_size(vec2(200.0, 100.0), vec2(240.0, height));
            for amount in [0.0, 0.5, 1.0] {
                let rounding = vec2(0.0, amount);
                let shape = music_shape(pill, rounding, [Vec2::ZERO; 2]);
                assert!(shape.contains(pill.center()), "missing track at height {height}");
                for x in 0..=24 {
                    for y in 0..=10 {
                        let point = pill.min + vec2(x as f32 * 10.0, y as f32 * height / 10.0);
                        assert!(shape.distance_at(point).is_finite(), "invalid track at height {height}");
                    }
                }
                let artwork =
                    track_shape(Rect::from_center_size(pill.center(), Vec2::splat(height)), vec2(1.0, rounding.y));
                assert!(artwork.distance_at(pill.center()).is_finite());
            }
        }
    }

    #[test]
    fn history_fades_only_at_playlist_boundary() {
        assert_eq!(history_visibility(100.0, 100.0, 50.0), 0.0);
        assert_eq!(history_visibility(150.0, 100.0, 50.0), 1.0);
        assert!((history_visibility(125.0, 100.0, 50.0) - 0.5).abs() < 0.001);
    }

    #[test]
    fn history_hit_regions_stop_at_playlist_boundary() {
        let pill = Rect::new(vec2(30.0, 0.0), vec2(90.0, 60.0));
        let visible = Shape::rectangle(Rect::new(vec2(70.0, 0.0), vec2(400.0, 200.0)));
        let hit = music_shape(pill, Vec2::ONE, [Vec2::ZERO; 2]).offset(12.0).intersection(visible);
        assert!(!hit.contains(vec2(60.0, 30.0)));
        assert!(hit.contains(vec2(80.0, 30.0)));
    }

    #[test]
    fn clips_tracks_and_preserves_collapsed_history() {
        let bar = BarLayout { playhead_x: 200.0, px_per_ms: 0.1 };
        for (start_ms, width, right) in
            [(0.0, 80.0, 280.0), (-500.0, 50.0, 250.0), (-2000.0, HISTORY_TRACK_WIDTH, 200.0), (1000.0, 0.0, 280.0)]
        {
            let layout = TrackLayout::new(0, 1000, start_ms, bar, 280.0, 200.0);
            assert!((layout.width - width).abs() < 0.001, "unexpected width at {start_ms} ms");
            assert!((layout.right - right).abs() < 0.001, "unexpected right edge at {start_ms} ms");
        }
    }
}
