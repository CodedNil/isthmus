use crate::{
    music::{Music, TRACK_SPACING_MS, Track},
    render::{
        BarLayout, Fragment, GAP, PANEL_START, UiContext,
        sdf::{Glass, Refraction, VISIBLE_ALPHA, hash, sample_pill, simplex_noise},
    },
};
use core::{
    f32::consts::{FRAC_PI_2, TAU},
    f64,
};
use isthmus::{
    Blend, ColorExt as _, Float as _, Image, Quad, Text, Unorm8x4,
    geometry::sdf::{self, Capsule, Circle, SdfShape as _, Shape, SmoothUnion},
    glam::{Vec2, Vec3, Vec4, vec2, vec3},
    shader,
    spirv_std::arch::{Derivative as _, kill},
};
use smallvec::SmallVec;

/// Number of colors extracted from album artwork.
pub const PALETTE_COLORS: usize = 4;
/// Visual width, in pixels, of rating and playlist icons before hover growth.
const ICON_WIDTH: f32 = 21.6;
/// Center-to-center icon spacing for rating stars and playlist artwork.
const ICON_SPACING: f32 = 18.0;
const ICON_REACTION_RADIUS: f32 = ICON_WIDTH * 2.5;
const PRIMARY_SUPPORT_DEPTH: f32 = 7.0;
const SECONDARY_SUPPORT_DEPTH: f32 = 18.0;

type MusicShape = SmoothUnion<SmoothUnion<Capsule, Capsule>, Capsule>;

fn music_shape(pill: Quad, supports: [Vec2; 2]) -> Shape<MusicShape> {
    let bottom = pill.center.y + pill.size.y * 0.5;
    let support = |index: usize| {
        Shape::pill(Quad::new(
            vec2(pill.center.x, bottom - 6.0 + index as f32 * 7.0 + supports[index].y * 0.5),
            supports[index],
            Vec2::X,
        ))
    };
    Shape::pill(pill).smooth_union(support(0), 9.0, supports[0].y / PRIMARY_SUPPORT_DEPTH).smooth_union(
        support(1),
        9.0,
        supports[1].y / SECONDARY_SUPPORT_DEPTH,
    )
}

/// Spotify audio characteristics normalized for shader and UI use.
#[derive(Clone, Copy, Default, isthmus::ShaderData, serde::Deserialize)]
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
}

#[derive(Clone, Copy)]
struct Particle {
    origin: Vec2 = Vec2::ZERO,
    velocity: Vec2 = Vec2::ZERO,
    spawned_at: f32 = 0.0,
    expires_at: f32 = 0.0,
    color: Vec3 = Vec3::ZERO,
    sway: f32 = 0.0,
    size: Vec2 = Vec2::new(10.0, 5.0),
}

fn particle_color(color: Vec3) -> Vec3 {
    Vec3::splat(color.dot(vec3(0.299, 0.587, 0.114))).lerp(color, 2.0).lerp(Vec3::ONE, 0.2) * 2.0
}

fn shade_icon(color: Vec3, distance: f32) -> Vec4 {
    let mask = sdf::fill(distance);
    let shadow = (-distance.max(0.0)).exp() * 0.2;
    let bevel = distance.smoothstep(-5.0, 0.0);
    let coverage = mask.max(shadow);
    ((color + bevel * bevel * 0.045) * (mask / coverage.max(0.0001))).extend(coverage)
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
        if music.queue.is_empty() {
            return;
        }
        // Track layout
        let gap = TRACK_SPACING_MS * bar.px_per_ms;
        let width_trim = (GAP - gap).max(0.0);
        let (history_width, panel_height) = (context.config.history_width, context.config.height);
        let future_start_ms =
            (context.config.timeline_future_minutes - context.config.timeline_past_minutes) * 60_000.0;
        let future_end = history_width + context.config.timeline_future_minutes * 60_000.0 * bar.px_per_ms + width_trim;
        context
            .interaction
            .input_region(Quad::from_min_max(vec2(0.0, PANEL_START), vec2(future_end, PANEL_START + panel_height)));
        let mut compact_tracks = 0;
        let mut transition = 0.0;
        let mut start_ms = music.timeline.queue_start_ms + music.queue.iter().map(Track::queue_span_ms).sum::<f32>();
        for track in music.queue.iter().rev() {
            start_ms -= track.queue_span_ms();
            let natural_end = bar.playhead_x + (start_ms + track.duration_ms as f32) * bar.px_per_ms;
            if (history_width..history_width + panel_height).contains(&natural_end) {
                transition = (history_width + panel_height - natural_end) / panel_height;
            } else if natural_end < history_width {
                compact_tracks += 1;
            }
        }

        // Track render and interaction
        let drag = context.interaction.drag_motion();
        let mouse_pos = context.interaction.mouse_pos();
        let mouse_pressure = context.interaction.pressure();
        let mut seek_action = None;
        let mut rated_track = None;
        let mut playlist_toggle = None;

        let mut visible = SmallVec::<[_; 16]>::new();
        start_ms = music.timeline.queue_start_ms;
        for (queue_index, track) in music.queue.iter().enumerate() {
            let track_start_ms = start_ms;
            start_ms += track.queue_span_ms();
            let natural_start = bar.playhead_x + track_start_ms * bar.px_per_ms;
            let natural_end = natural_start + track.duration_ms as f32 * bar.px_per_ms;
            if track_start_ms > future_start_ms {
                break;
            }
            let (x, width) = if natural_end >= history_width + panel_height {
                let x = natural_start.max(history_width);
                (x, (natural_end.min(future_end) - x - width_trim).max(0.0))
            } else if natural_end >= history_width {
                (natural_end - panel_height, panel_height)
            } else {
                compact_tracks -= 1;
                let right = history_width - gap - (compact_tracks as f32 + transition) * panel_height * 0.55;
                (right - panel_height, panel_height)
            };
            if width <= 0.0 || x + width <= 0.0 {
                continue;
            }
            let hovered = Shape::pill(Quad::from_min_max(
                vec2(x, PANEL_START),
                vec2(x + width.max(panel_height), PANEL_START + panel_height),
            ))
            .contains(mouse_pos);
            visible.push((hovered, queue_index, track_start_ms, natural_start, x, width));
        }
        visible.sort_by_key(|&(hovered, ..)| hovered);

        for (_, queue_index, track_start_ms, natural_start, mut x, mut width) in visible {
            let track = &mut music.queue[queue_index];
            let expansion = track.runtime.track_expansion.smoothstep(0.0, 1.0);
            // Track content
            let track_text = (width > panel_height + 26.0 || expansion > 0.0).then(|| {
                let title = track
                    .name
                    .split_once(" -")
                    .map_or(track.name.as_str(), |(name, _)| name)
                    .split('(')
                    .next()
                    .unwrap_or_default()
                    .trim();
                let title = if title.is_empty() { &track.name } else { title };
                let seconds = (track_start_ms / 1000.0).abs();
                let details = if seconds >= 60.0 {
                    let whole_seconds = seconds as u32;
                    format!("{}m{}s\u{2004}•\u{2004}{}", whole_seconds / 60, whole_seconds % 60, track.artist)
                } else {
                    format!("{}s\u{2004}•\u{2004}{}", seconds.round(), track.artist)
                };
                (context.frame.text.shape(title, 16.0, 700.0), context.frame.text.shape(&details, 14.0, 700.0))
            });
            if expansion > 0.0
                && let Some((title, details)) = &track_text
            {
                let target = title.width.max(details.width) + panel_height + 20.0;
                let extra_width = (target - width).max(0.0) * expansion;
                x -= extra_width * 0.5;
                width += extra_width;
            }
            let mut playlist_icons = SmallVec::<[usize; 8]>::new();
            let mut primary_count = 0;
            let mut rating = None;
            if let Some(track_id) = track.id {
                if context.config.ratings_enabled {
                    rating = Some(0);
                }
                for (index, playlist) in music.playlists.iter().enumerate() {
                    let contains_track = playlist.tracks.contains(&track_id);
                    if let Some(value) = playlist.rating_index {
                        if contains_track && rating.is_some() {
                            rating = Some(i32::from(value) + 1);
                        }
                    } else if contains_track {
                        playlist_icons.insert(primary_count, index);
                        primary_count += 1;
                    } else if expansion > 0.0 {
                        playlist_icons.push(index);
                    }
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
            let icon_supports = [
                vec2(row_width(primary_icons) * primary_alpha, primary_alpha * PRIMARY_SUPPORT_DEPTH),
                vec2(row_width(secondary_count as f32) * secondary_alpha, secondary_alpha * SECONDARY_SUPPORT_DEPTH),
            ];
            let pill =
                Quad::from_min_max(vec2(x, PANEL_START), vec2(x + width.max(panel_height), PANEL_START + panel_height));
            let colors = track.runtime.art.palette();
            let art = track.runtime.art.ready();
            let alpha = width.smoothstep(panel_height, panel_height + 26.0).max(f32::from(track_start_ms <= 0.0));
            let seed =
                track.uri.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
                }) as u32 as f32
                    * 2.328_306_4e-10;
            let audio = track.runtime.audio_features.ready().copied().unwrap_or_default();
            let pulse = ((f64::from(context.frame.time) * f64::from(audio.tempo) * 5.0).fract() * f64::consts::TAU)
                .sin() as f32
                * 0.5
                + 0.5;
            let shape = music_shape(pill, icon_supports);
            if let Some(bounds) = shape.bounds(0.0) {
                context.interaction.input_region(bounds);
            }
            let body = context.interaction.drag(track.interaction_id, shape);
            let mut hovered = body.hovered;
            if body.clicked && track.id.is_some() {
                let pointer_x = mouse_pos.x;
                let near_visible_start = pointer_x <= x + pill.size.x * 0.05;
                let fraction = if queue_index == music.timeline.index && near_visible_start {
                    0.0
                } else {
                    ((pointer_x - natural_start) / (track.duration_ms as f32 * bar.px_per_ms)).clamp(0.0, 1.0)
                };
                seek_action = Some((queue_index, track.duration_ms, fraction));
            }
            context.frame.paint(
                shape.with_effect(Glass),
                shader!({
                    let alpha: f32 = alpha;
                    let seed: f32 = seed;
                    let colors: [Unorm8x4; PALETTE_COLORS] = colors;
                    let audio: AudioFeatures = audio;
                    let pulse: f32 = pulse;
                    |fragment: Fragment<MusicShape>| {
                        let surface = Glass::sample(fragment.base.base.quad, fragment.geometry, &fragment);

                        if surface.alpha * alpha <= VISIBLE_ALPHA {
                            kill();
                        }

                        let uv = surface.uv();
                        let pace = ((audio.tempo - 0.2) * 2.5).saturate();
                        let flow_time =
                            fragment.time * 0.2 + seed + (fragment.time * 0.17).sin() * (audio.energy + pace);
                        let beat = pulse * pulse * audio.danceability * (0.025 + audio.energy * 0.055);
                        let turbulence = audio.energy * 0.65 + audio.danceability * 0.35;

                        // Distort pill-local coordinates into a slow, overlapping plasma field.
                        let lens = (1.0 + surface.distance.min(0.0) / 120.0).saturate();
                        let deformation = (uv - 0.5) * lens * lens * 0.6 + surface.ripple;
                        let frequency = (surface.size.x / surface.size.y
                            * (0.5 + seed.fract() * 0.12 + turbulence * 0.18))
                            .max(1.7);
                        let field = (uv.clamp(Vec2::ZERO, Vec2::ONE) - deformation * 0.08) * vec2(frequency, 1.6);
                        let warped = field
                            + vec2(
                                (field.y * 2.7 + flow_time).sin() + (field.x * 1.3 - flow_time * 0.7).cos(),
                                (field.x * 2.3 - flow_time * 0.8).cos() + (field.y * 1.7 + flow_time * 0.6).sin(),
                            ) * (0.14 + turbulence * 0.2 + beat);

                        // Weight each wave by its palette colour's prevalence.
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

                        // Normalize brightness before applying track character and playback state.
                        let luma = color.dot(vec3(0.2126, 0.7152, 0.0722));
                        color = Vec3::splat(luma)
                            .lerp(color, 1.55 + audio.valence * 0.4)
                            .clamp(Vec3::splat(0.035), Vec3::splat(0.92))
                            * (0.52 / luma.max(0.001)).min(1.0)
                            * (0.96 + audio.valence * 0.06 + beat * 0.5)
                            * (0.84 + (surface.refracted / surface.size).y.smoothstep(0.45, 1.0) * 0.1);

                        // Material details sit above the plasma but below refraction/ripple response.
                        color += colors[3].to_vec3().lerp(Vec3::ONE, 0.25)
                            * speckle(surface.local, fragment.time, seed, audio);
                        color *= 1.0 + caustics(surface.local / surface.size.y, fragment.time, seed, audio);
                        surface.color(color).opacity(alpha)
                    }
                }),
            );

            // Artwork render
            if let Some(art) = art {
                let center = pill.center + vec2((pill.size.x - pill.size.y) * 0.5, 0.0);
                context.frame.paint(
                    Shape::circle(center, pill.size.y * 0.5).with_effect(Glass),
                    shader!({
                        let pill: Quad = pill;
                        let image: &Image = &art.image;
                        let alpha: f32 = alpha;
                        |fragment: Fragment<Circle>| {
                            let surface = sample_pill(pill, &fragment);
                            let image_center =
                                vec2(surface.size.x - surface.size.y, 0.0) + Vec2::splat(surface.size.y * 0.5);
                            let offset = surface.local - image_center;
                            let radius = surface.size.y * 0.5 + surface.bulge() * 0.5;
                            let texture = image.sample(offset / (radius * 2.0) + 0.5);
                            let alpha =
                                texture.w * (offset.length() - radius).smoothstep(0.0, -4.0) * surface.mask * alpha;
                            texture.truncate().extend(alpha)
                        }
                    }),
                );
            }

            // Text render
            let (left, right) = (18.0, width - panel_height - 8.0);
            if right > left
                && let Some((title, details)) = track_text
            {
                for (line, y) in [(title, 0.26), (details, 0.57)] {
                    let line = context.frame.text.fit(&line, (panel_height * y).floor(), left..right);
                    context.frame.paint(
                        line.with_effect(Refraction).translated(pill.center - pill.size * 0.5),
                        shader!({
                            let pill: Quad = pill;
                            let alpha: f32 = alpha;
                            |text: Fragment<Text>| {
                                let surface = sample_pill(pill, &text);
                                let image_center =
                                    vec2(surface.size.x - surface.size.y, 0.0) + Vec2::splat(surface.size.y * 0.5);
                                let alpha = Shape::circle(Vec2::ZERO, surface.size.y * 0.5)
                                    .distance_at(surface.refracted - image_center)
                                    .smoothstep(2.0, 18.0)
                                    * alpha;
                                surface.text(&text).opacity(alpha)
                            }
                        }),
                    );
                }
            }
            // Icon interaction and render
            let mut toggled_playlist = None;
            let mut burst = false;
            if let Some(track_id) = track.id {
                for slot in 0..stars + primary_count + secondary_count {
                    let playlist_slot = slot.saturating_sub(stars);
                    let is_star = slot < stars;
                    let (icon, count, alpha, expansion, secondary) = if is_star {
                        (slot as f32 * star_alpha, primary_icons, star_alpha, 1.0, false)
                    } else if playlist_slot < primary_count {
                        (
                            stars as f32 * star_alpha + playlist_slot as f32 * playlist_alpha,
                            primary_icons,
                            playlist_alpha,
                            1.0,
                            false,
                        )
                    } else {
                        ((playlist_slot - primary_count) as f32, secondary_count as f32, expansion, expansion, true)
                    };
                    if alpha <= 0.0 {
                        continue;
                    }
                    let center = vec2(
                        pill.center.x + (icon - (count - 1.0).max(0.0) * 0.5) * ICON_SPACING * expansion,
                        PANEL_START + panel_height * 0.975 - 1.0 + f32::from(secondary) * ICON_SPACING * expansion,
                    );
                    let response = context.interaction.interact(Shape::circle(center, ICON_WIDTH * 0.5));
                    hovered |= response.hovered;
                    let mouse_distance = center.distance(mouse_pos);
                    let proximity = mouse_distance.smoothstep(ICON_REACTION_RADIUS, ICON_WIDTH * 0.25)
                        * mouse_pressure.clamp(0.0, 1.0);
                    let x_push = (center.x - mouse_pos.x) * proximity * 0.5;
                    let radius = ICON_WIDTH * 0.5 * (1.05 + 0.63 * proximity);
                    let quad = Quad::new(
                        center + vec2(x_push, 0.0),
                        Vec2::splat(radius * 2.0 + 14.0),
                        Vec2::from_angle(x_push * 0.01),
                    );

                    if !is_star {
                        let playlist_index = playlist_icons[playlist_slot];
                        let playlist = &music.playlists[playlist_index];
                        if response.clicked {
                            toggled_playlist = Some(playlist.id);
                        }
                        if let Some(art) = playlist.art.ready() {
                            context.frame.paint(
                                quad,
                                shader!({
                                    let image: &Image = &art.image;
                                    let alpha: f32 = alpha;
                                    let desaturation: f32 = 0.2
                                        * f32::from(
                                            secondary && (mouse_pressure <= 0.0 || mouse_distance > ICON_WIDTH * 0.5),
                                        );
                                    |fragment: Fragment<Quad>| {
                                        shade_icon(
                                            image.sample(fragment.uv).truncate().lerp(Vec3::splat(0.24), desaturation),
                                            Shape::circle(Vec2::ZERO, 6.7)
                                                .distance_at((fragment.uv * 2.0 - 1.0) * 18.0),
                                        )
                                        .opacity(alpha)
                                    }
                                }),
                            );
                        }
                    } else if let Some(rating) = rating.as_mut() {
                        let right_half = mouse_pos.x >= center.x;
                        if response.hovered {
                            *rating = slot as i32 * 2 + 1 + i32::from(right_half);
                        }
                        if response.clicked {
                            rated_track = Some((track_id, slot as u8 * 2 + u8::from(right_half)));
                            burst = true;
                        }
                        context.frame.paint(
                            quad,
                            shader!({
                                let fill: f32 = (*rating as f32 * 0.5 - slot as f32).saturate();
                                let alpha: f32 = alpha;
                                |fragment: Fragment<Quad>| {
                                    let split = fragment.uv.x - fill;
                                    let unselected = (split / split.fwidth() + 0.5).saturate();
                                    shade_icon(
                                        vec3(1.0, 0.85, 0.2).lerp(Vec3::splat(0.33), unselected),
                                        Shape::star(5.6, 3.58).distance_at((fragment.uv * 2.0 - 1.0) * 18.0) - 1.12,
                                    )
                                    .opacity(alpha)
                                }
                            }),
                        );
                    }
                }
                if let Some(playlist_id) = toggled_playlist {
                    playlist_toggle = Some((track_id, playlist_id));
                    burst = true;
                }
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
                        color: particle_color(vec3(1.0, 0.843, 0.196)),
                        ..
                    };
                }
            }
            track.runtime.track_expansion = track.runtime.track_expansion.move_towards(
                f32::from(hovered && width > panel_height && alpha >= 1.0),
                context.frame.delta_time.min(0.1) / 0.16,
            );
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

        // Track dragging
        let drag_offset_ms = drag.map_or(0.0, |(offset, _)| offset.x / bar.px_per_ms);
        music.update_timeline(drag_offset_ms, drag.is_some(), context.frame.delta_time);
        let playhead_track = music.timeline.track_at_playhead(&music.queue);
        if drag.is_some_and(|(_, released)| released)
            && let Some((index, position_ms)) = playhead_track
        {
            let track = &music.queue[index];
            if track.id.is_some() {
                music.seek(index, track.duration_ms, position_ms / track.duration_ms as f32);
            }
        }

        // Particle emission
        let time = context.frame.time;
        if let Some((index, _)) = playhead_track {
            let track = &music.queue[index];
            let audio = track.runtime.audio_features.ready().copied().unwrap_or_default();
            self.particles_debt = (self.particles_debt + context.frame.delta_time * (20.0 + audio.energy * 35.0))
                * f32::from(music.timeline.movement.abs() > 0.00001);
            let emit_count = self.particles_debt.floor() as usize;
            self.particles_debt -= emit_count as f32;
            let horizontal_bias =
                (music.timeline.movement.abs().powf(0.2) * music.timeline.movement.signum()).clamp(-3.0, 3.0);
            let palette = track.runtime.art.palette();
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
                    color: particle_color(palette[fastrand::usize(0..palette.len())].to_vec3()),
                    sway: (fastrand::f32() - 0.5) * (4.0 + audio.danceability * 18.0),
                    size: vec2(7.0 + audio.acousticness * 9.0, 3.0 + audio.energy * 4.0),
                };
            }
        } else {
            self.particles_debt = 0.0;
        }

        // Particle render
        for particle in self.particles.iter().filter(|particle| particle.expires_at > time) {
            let elapsed = time - particle.spawned_at;
            let age = elapsed / (particle.expires_at - particle.spawned_at);
            context.frame.paint(
                Quad::oriented(
                    particle.origin
                        + particle.velocity * elapsed
                        + vec2(0.0, (elapsed * 4.0).sin() * elapsed * particle.sway),
                    particle.size * (age + 0.5),
                    particle.velocity,
                ),
                shader!(Blend::Add, {
                    let color: Vec3 = particle.color;
                    let opacity: f32 = (1.0 - age) * elapsed.smoothstep(0.0, 0.15);
                    |fragment: Fragment<Quad>| {
                        let radius = (fragment.uv * 2.0 - 1.0).length_squared();
                        let shape = (1.0 - radius).max(0.0).powi(3);
                        color.extend(opacity * shape)
                    }
                }),
            );
        }
        // Playhead interaction and render
        let half_width = panel_height * 0.4;
        let playhead =
            Quad::new(vec2(bar.playhead_x, PANEL_START + panel_height * 0.5), Vec2::splat(half_width * 2.0), Vec2::X);
        let response = context.interaction.interact(Shape::rectangle(playhead));
        let speed = context.frame.delta_time * 5.5;
        let last_toggle = music.last_toggle.elapsed().as_secs_f32() / 0.7;
        if !response.hovered && music.playing && last_toggle < 1.0 {
            self.bar_split = 1.0 - last_toggle;
            self.icon_presence = 1.0 - last_toggle;
            self.icon_morph = self.icon_morph.move_towards(1.0, speed * 1.5);
        } else {
            let show_icon = f32::from(response.hovered || !music.playing);
            self.bar_split = self.bar_split.move_towards(show_icon, speed);
            self.icon_presence = self.icon_presence.max(show_icon);
            self.icon_presence = self.icon_presence.move_towards(show_icon, speed);
            self.icon_morph = self.icon_morph.move_towards(f32::from(response.hovered && !music.playing), speed);
        }
        if response.clicked {
            music.toggle_playing();
        }
        context.frame.paint(
            Quad::from_min_max(
                vec2(bar.playhead_x - half_width, PANEL_START - 5.0),
                vec2(bar.playhead_x + half_width, PANEL_START + panel_height + 5.0),
            ),
            shader!({
                let bar_split: f32 = self.bar_split;
                let icon_presence: f32 = self.icon_presence;
                let icon_morph: f32 = self.icon_morph;
                |fragment: Fragment<Quad>| {
                    let mirrored = fragment.local.abs();
                    let bar_len = fragment.globals.bar_height * (0.5 - 0.375 * bar_split);
                    let bar_distance = Shape::pill(vec2(bar_len + 9.0, 9.0))
                        .distance_at(vec2(mirrored.y - (fragment.globals.bar_height - bar_len) * 0.5, mirrored.x));
                    let pause_distance = vec2(
                        (mirrored.x - 4.0 * bar_split).abs(),
                        (mirrored.y - fragment.globals.bar_height * 0.1).max(0.0),
                    )
                    .length()
                        - 3.5;
                    let play_scale = fragment.globals.bar_height * 0.18 * (1.0 + icon_morph * (1.0 - icon_presence));
                    let play_distance =
                        Shape::rounded_triangle(play_scale, play_scale * 0.5).distance_at(fragment.local.perp());
                    let icon_distance = pause_distance.lerp(play_distance, icon_morph);
                    let bar_mask = sdf::fill(bar_distance);
                    let icon_mask = sdf::fill(icon_distance) * icon_presence;
                    let alpha = icon_mask.max(bar_mask);
                    if alpha <= 0.0 {
                        kill();
                    }
                    let color = vec3(1.0, 0.878, 0.824)
                        .lerp(Vec3::splat(0.15), bar_distance.min(icon_distance).smoothstep(-2.5, -1.0));
                    color.extend(alpha)
                }
            }),
        );
    }
}
