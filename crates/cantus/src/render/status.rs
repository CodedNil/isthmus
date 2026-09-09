use crate::{
    app::Background,
    platform,
    render::{
        GAP, PANEL_START, Program, TEXT_COLOR, UiContext,
        sdf::{deform, hash, lens, refract, sample_deformation},
        weathertime::{StatusSky, WeatherCondition, scene, sky_phase},
    },
};
use arrayvec::ArrayString;
use core::f32::consts::TAU;
use isthmus::prelude::*;
use isthmus_sdf::Shape;
use std::{
    fmt::Write,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

const STATUS_HISTORY_SAMPLES: usize = 32;
pub const AUDIO_SPECTRUM_BANDS: usize = 7;
const DATA_WIDTH: f32 = 32.0;
const ACTION_WIDTH: f32 = 24.0;
const AUDIO_BAR_SPACING: f32 = 4.0;
const AUDIO_BAR_RADIUS: f32 = 1.25;
const AUDIO_HALF_WIDTH: f32 = AUDIO_BAR_SPACING * (AUDIO_SPECTRUM_BANDS - 1) as f32 * 0.5 + AUDIO_BAR_RADIUS;
/// CPU/GPU graphs stay this wide regardless of whether the battery slot is present.
const GRAPH_WIDTH: f32 = 60.0 + f32::midpoint(DATA_WIDTH, GAP);
const CHART_LINE_WIDTH: f32 = 0.85;
const USAGE_COLOR: Vec3 = Vec3::new(0.32, 0.68, 1.0);
const MEMORY_COLOR: Vec3 = Vec3::new(0.78, 0.3, 1.0);
const MUTED_COLOR: Vec3 = Vec3::new(1.0, 0.24, 0.3);
const HISTORY_END: usize = STATUS_HISTORY_SAMPLES - 1;
const STATUS_INSET: f32 = GAP * 2.0;
const BASE_WIDTH: f32 = GRAPH_WIDTH * 2.0 + DATA_WIDTH + ACTION_WIDTH * 2.0 + GAP * 4.0 + STATUS_INSET * 2.0;

#[derive(Default)]
pub struct ProcessorStatus {
    pub temperature: f32,
    target_temperature: f32,
    pub history: [Unorm16x2; STATUS_HISTORY_SAMPLES],
}

#[derive(Default)]
pub struct AudioMonitor {
    pub volume: AtomicU32,
    pub spectrum: [AtomicU32; AUDIO_SPECTRUM_BANDS],
}

#[derive(Default)]
pub struct StatusPanel {
    /// Battery charge magnitude, negated while charging.
    battery_level: Option<f32>,
    /// Logarithmic frequency-band levels sampled from the system audio monitor stream.
    audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS],
    /// Fractional scroll between the two newest history samples.
    history_scroll: f32,
    processors: [ProcessorStatus; 2],
    action_hover: [f32; 2],
    audio_monitor: Arc<AudioMonitor>,
}

#[derive(Clone, Copy)]
pub struct ProcessorSample {
    pub temperature: f32,
    pub usage: f32,
    pub memory: f32,
}

#[derive(Clone, Copy)]
pub struct SystemSample {
    pub cpu: ProcessorSample,
    pub gpu: Option<ProcessorSample>,
    pub battery_level: Option<f32>,
}

impl StatusPanel {
    /// Total on-screen width of the pill; grows to include the battery slot when shown.
    pub const fn width(&self) -> f32 {
        BASE_WIDTH + if self.battery_level.is_some() { DATA_WIDTH + GAP } else { 0.0 }
    }

    pub(crate) fn new(background: &Background) -> Self {
        let panel = Self::default();
        platform::start_status_monitor(background.updater.clone(), Arc::clone(&panel.audio_monitor));
        panel
    }

    pub(crate) fn record(&mut self, update: SystemSample) {
        for (processor, sample) in self.processors.iter_mut().zip([Some(update.cpu), update.gpu]) {
            if let Some(sample) = sample {
                processor.target_temperature = sample.temperature;
                processor.history.copy_within(1.., 0);
                processor.history[HISTORY_END] = Unorm16x2::from_vec2(vec2(sample.usage, sample.memory));
            }
        }
        self.battery_level = update.battery_level;
        self.history_scroll = 0.0;
    }

    pub fn show(&mut self, context: &mut UiContext, sky: StatusSky) {
        let mut volume = f32::from_bits(self.audio_monitor.volume.load(Ordering::Relaxed));
        let height = context.config.height;
        let width = self.width();
        let x = context.frame.screen_size.x - width - GAP;
        let pill_quad = Quad::from_min_max(vec2(x, PANEL_START), vec2(x + width, PANEL_START + height));
        let mut cursor = STATUS_INSET;
        let section = |center: f32, section_width: f32| {
            Quad::new(vec2(x + center, PANEL_START + height * 0.5), vec2(section_width, height), Vec2::X)
        };

        shader!(
            context
                .frame
                .upload({
                    let conditions: WeatherCondition = sky.conditions;
                    let pill_quad: Quad;
                    let phase: Vec3 = sky_phase(sky.sun_height);
                })
                .vertex(|frame| deform(Shape::pill(pill_quad), frame))
                .fragment(|frame, surface| {
                    surface.glass(scene(
                        frame.time,
                        frame.globals.bar_height,
                        pill_quad.uv(surface.refracted) * pill_quad.size,
                        pill_quad.size.x,
                        phase,
                        conditions,
                    ))
                })
        );

        let temperature_blend = 1.0 - (-5.0 * context.frame.delta_time).exp();
        self.history_scroll = (self.history_scroll
            + context.frame.delta_time / platform::STATUS_SAMPLE_INTERVAL.as_secs_f32())
        .saturate();
        for processor in &mut self.processors {
            processor.temperature += (processor.target_temperature - processor.temperature) * temperature_blend;
            let center = cursor + GRAPH_WIDTH * 0.5;
            let graph_pill = Quad::from_min_max(
                vec2(x + cursor, PANEL_START + GAP),
                vec2(x + cursor + GRAPH_WIDTH, PANEL_START + height - GAP),
            );
            let mut label = ArrayString::<16>::new();
            let latest = processor.history[HISTORY_END].to_vec2() * 100.0;
            write!(label, "{:.0}% {:.0}% {:.0}\u{b0}C", latest.x, latest.y, processor.temperature).unwrap();
            let half_width = GRAPH_WIDTH * 0.5 - GAP * 0.5;
            shader!(
                context
                    .frame
                    .upload({
                        let pill_quad: Quad;
                        let history: Buffer<'_, Unorm16x2> = Buffer::new(&processor.history);
                        let history_scroll: f32 = self.history_scroll;
                        let graph_pill: Quad;
                        let frame_color: Vec3 = {
                            let heat = vec3(0.22, 0.62, 1.0)
                                .lerp(vec3(1.0, 0.38, 0.08), processor.temperature.smoothstep(60.0, 72.0))
                                .lerp(vec3(1.0, 0.08, 0.035), processor.temperature.smoothstep(72.0, 88.0));
                            vec3(0.025, 0.09, 0.15)
                                .lerp(USAGE_COLOR, 0.18 + history.load(HISTORY_END).to_vec2().x * 0.24)
                                .lerp(heat, processor.temperature.smoothstep(60.0, 86.0) * 0.9)
                        };
                    })
                    .vertex(|frame| lens(Shape::pill(pill_quad), frame, Shape::pill(graph_pill)))
                    .fragment(|_, surface| {
                        let point = graph_pill.local(surface.refracted);
                        let half_width = graph_pill.size.x * 0.5;
                        let radius = graph_pill.size.y * 0.5;
                        let history_step = half_width * 2.0 / HISTORY_END as f32;
                        let graph_height = radius - 2.0;
                        let curve = |channel: Vec2, color: Vec3, fill_strength: f32| {
                            let sample = Shape::from_fn(graph_pill.size, move |point| {
                                let sample = ((point.x + half_width) / history_step + history_scroll)
                                    .clamp(0.0, HISTORY_END as f32);
                                let index = sample.floor() as usize;
                                let start = (1.0 - history.load(index).to_vec2().dot(channel) * 2.0) * graph_height;
                                let end = (1.0
                                    - history.load((index + 1).min(HISTORY_END)).to_vec2().dot(channel) * 2.0)
                                    * graph_height;
                                let delta = end - start;
                                let t = sample.fract();
                                let graph_y = start + delta * t * t * (3.0 - 2.0 * t);
                                let slope = delta * 6.0 * t * (1.0 - t) / history_step;
                                (graph_y - point.y) / (1.0 + slope * slope).sqrt()
                            })
                            .sample_at(point);
                            color
                                * surface.sdf.fill()
                                * (sample.fill() * fill_strength + sample.band(-CHART_LINE_WIDTH..CHART_LINE_WIDTH))
                        };
                        let graphs = curve(Vec2::X, USAGE_COLOR, 0.156) + curve(Vec2::Y, MEMORY_COLOR, 0.084);
                        let cell = (((point + vec2(half_width, radius)) / vec2(7.0, 6.1)).fract() - 0.5).abs();
                        let grid = surface.sdf.fill()
                            * cell.x.smoothstep(0.49, 0.46).max(cell.y.smoothstep(0.49, 0.45))
                            * 0.045;
                        let hardware = surface.sdf.band(-1.45..1.45);
                        let color = vec3(0.004, 0.012, 0.026).lerp(frame_color, hardware) + Vec3::splat(grid) + graphs;
                        surface.glass(color)
                    })
            );
            let line = context
                .frame
                .resources
                .line(&label, 11.0, 700.0)
                .fit(GAP + 5.0, center - half_width..center + half_width)
                .translated(vec2(x, PANEL_START));
            context.paint_text(pill_quad, pill_quad.size.y * 0.5, line, TEXT_COLOR.extend(1.0));
            cursor += GRAPH_WIDTH + GAP;
        }

        if let Some(battery_level) = self.battery_level {
            let center = cursor + DATA_WIDTH * 0.5;
            shader!(
                context
                    .frame
                    .upload({
                        let pill_quad: Quad;
                        let quad: Quad = section(center, DATA_WIDTH);
                        let charging: f32 = if battery_level < 0.0 { 1.0 } else { 0.0 };
                        let level: f32 = battery_level.abs();
                        let liquid_color: Vec3 = vec3(1.0, 0.18, 0.10)
                            .lerp(vec3(1.0, 0.72, 0.12), level.smoothstep(0.08, 0.28))
                            .lerp(vec3(0.22, 0.95, 0.55), level.smoothstep(0.18, 0.72));
                    })
                    .vertex(quad)
                    .fragment(|frame, surface| {
                        let surface = sample_deformation(Shape::pill(pill_quad), frame, surface.pixel);
                        let point = quad.local(surface.refracted) / 0.8;
                        let body = Shape::rounded_rect(vec2(23.0, 30.0), 3.2).translated(vec2(0.0, 1.0));
                        let terminal = Shape::rounded_rect(vec2(8.0, 3.6), 0.8).translated(vec2(0.0, -15.6));
                        let shell = body.union(terminal).fill_at(point);
                        let inside = Shape::rounded_rect(vec2(17.0, 24.0), 1.7).translated(vec2(0.0, 1.0));
                        let liquid_y = 12.0 - level.saturate() * 24.0;
                        let liquid = inside
                            .intersection(Shape::from_fn(vec2(17.0, 28.0), move |point| {
                                let wave = (point.x * 0.62 + frame.time * (1.4 + charging * 1.2)).sin() * 1.15
                                    + (point.x * 0.27 - frame.time * 0.8).sin() * 0.45;
                                liquid_y + wave - (point.y - 1.0)
                            }))
                            .fill_at(point);
                        let column = (point.x / 3.0).floor();
                        let seed = hash(vec2(column, 0.0));
                        let cycle = (frame.time * (0.35 + seed.y * 0.5) + seed.x * 7.0).fract();
                        let center = vec2((column + 0.2 + seed.x * 0.6) * 3.0, 13.0 - cycle * 24.0);
                        let bubble = Shape::circle(center, 0.4 + seed.y * 0.5).stroke(0.9).intersection(inside);
                        let fade = cycle.smoothstep(0.0, 0.25) * cycle.smoothstep(1.0, 0.7);
                        let bubble = bubble.fill_at(point) * fade * charging;
                        let color = TEXT_COLOR.lerp(liquid_color, liquid) * shell
                            + liquid_color.lerp(Vec3::ONE, 0.72) * bubble * 0.9;
                        let alpha = shell.max(liquid).max(bubble);
                        (color / alpha.max(0.0001)).extend(alpha)
                    })
            );
            cursor += DATA_WIDTH + GAP;
        }

        for (damped, level) in self.audio_spectrum.iter_mut().zip(&self.audio_monitor.spectrum) {
            let target = f32::from_bits(level.load(Ordering::Relaxed));
            let response = if target > *damped { 18.0 } else { 6.0 };
            *damped += (target - *damped) * (1.0 - (-response * context.frame.delta_time).exp());
        }
        let center = cursor + DATA_WIDTH * 0.5;
        let scroll = context.interaction.interact("volume", Shape::rectangle(section(center, DATA_WIDTH))).scroll;
        if scroll != 0 {
            volume = (volume.abs() - scroll as f32 * 0.05).saturate() * volume.signum();
            self.audio_monitor.volume.store(volume.to_bits(), Ordering::Relaxed);
            platform::set_volume(volume.abs());
        }
        shader!(
            context
                .frame
                .upload({
                    let pill_quad: Quad;
                    let audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS] = self.audio_spectrum;
                    let volume: f32;
                    let audio_quad: Quad = section(center, DATA_WIDTH);
                })
                .vertex(|frame| lens(Shape::pill(pill_quad), frame, Shape::rectangle(audio_quad)))
                .fragment(|_, surface| {
                    let point = audio_quad.local(surface.refracted);
                    let muted = if volume < 0.0 { 1.0 } else { 0.0 };
                    let volume = volume.abs();
                    let middle = (AUDIO_SPECTRUM_BANDS - 1) as f32 * 0.5;
                    let bar =
                        (point.x / AUDIO_BAR_SPACING + middle).round().clamp(0.0, AUDIO_SPECTRUM_BANDS as f32 - 1.0);
                    let active = audio_spectrum[bar as usize] * (1.0 - muted);
                    let height = 1.2 + 7.7 * active;
                    let bars = Shape::pill(vec2(AUDIO_BAR_RADIUS, height) * 2.0)
                        .fill_at(point - vec2((bar - middle) * AUDIO_BAR_SPACING, -1.5));
                    let rail_point = point - vec2(0.0, 11.5);
                    let rail = Shape::pill(vec2(AUDIO_HALF_WIDTH, AUDIO_BAR_RADIUS) * 2.0).fill_at(rail_point);
                    let level_x = AUDIO_HALF_WIDTH * (volume.saturate() * 2.0 - 1.0);
                    let level = rail * rail_point.x.smoothstep(level_x + 0.8, level_x - 0.8);
                    let color = vec3(0.18, 0.96, 1.0);
                    let alpha = bars.max(level);
                    ((color * bars + color.lerp(MUTED_COLOR, muted) * level) / alpha.max(0.0001)).extend(alpha)
                })
        );
        cursor += DATA_WIDTH + GAP;

        for action in [1usize, 0] {
            let center = cursor + ACTION_WIDTH * 0.5;
            let shape = Shape::circle(vec2(x + center, PANEL_START + height * 0.5), ACTION_WIDTH.min(height) * 0.5);
            let response = context.interaction.interact(("power", action), shape);
            self.action_hover[action] =
                self.action_hover[action].move_towards(f32::from(response.hovered), context.frame.delta_time / 0.12);
            if response.hovered && response.held_for(1.5) {
                platform::run_power_action(if action == 0 {
                    platform::PowerAction::PowerOff
                } else {
                    platform::PowerAction::Reboot
                });
            }
            shader!(
                context
                    .frame
                    .upload({
                        let panel: Quad = pill_quad;
                        let reboot: bool = action == 1;
                        let hover: f32 = self.action_hover[action];
                        let selected: f32 = f32::from(response.held && response.hovered);
                        let charge: f32 = (response.held_seconds / 1.5).saturate() * selected;
                        let progress: f32 = 1.0 - selected + charge;
                        let direction: Vec2 = Vec2::from_angle(TAU * (0.08 + 0.82 * progress));
                        let button_center: Vec2 = vec2(x + center, PANEL_START + height * 0.5);
                        let button_radius: f32 = ACTION_WIDTH.min(height) * 0.5;
                    })
                    .vertex(|frame| refract(
                        Shape::pill(panel),
                        frame,
                        Shape::circle(button_center, button_radius).bounds(0.0)
                    ))
                    .fragment(|frame, surface| {
                        let point = (surface.refracted - button_center) / (1.0 + hover * 0.07);
                        let (icon, expanded) = if reboot {
                            let ring =
                                Shape::arc(Vec2::ZERO, 7.1, TAU * 0.08, TAU * (progress * 0.82 - 0.045).max(0.0))
                                    .sample_at(point);
                            let offset = point - direction * 7.1;
                            let arrow = Shape::chevron(vec2(-3.2, 2.1))
                                .sample_at(vec2(offset.dot(direction.perp()), offset.dot(direction)));
                            let glyph = |expansion: f32| {
                                (ring.band(-1.05 - expansion..1.05 + expansion) * progress.smoothstep(0.0, 0.02))
                                    .max(arrow.band(-f32::MAX..1.0 + expansion))
                            };
                            (glyph(0.0), glyph(0.8))
                        } else {
                            let ease = charge.smoothstep(0.0, 1.0);
                            let radius = 7.5 - charge * 4.6 + (frame.time * 8.0).sin() * charge * (1.0 - charge) * 0.16;
                            let ring = Shape::circle(Vec2::ZERO, radius).stroke(2.1 + ease * 1.4);
                            let gap =
                                Shape::rounded_rect(vec2(6.0 * (1.0 - charge), 6.0), 0.5).translated(vec2(0.0, -7.0));
                            let stem = Shape::rounded_rect(vec2(2.1 + ease * 0.9, 9.2 - charge * 6.0), 0.7)
                                .translated(vec2(0.0, -5.0 + charge * 3.5));
                            let sample = ring.difference(gap).union(stem).sample_at(point);
                            (sample.fill(), sample.band(-f32::MAX..0.8))
                        };
                        let color = TEXT_COLOR.lerp(vec3(0.95, 0.42, 0.4), hover.max(selected * (0.5 + charge * 0.5)));
                        (color * (1.0 + charge * 0.45)).extend(icon).over(Vec4::W.opacity(expanded * 0.18))
                    })
            );
            cursor += ACTION_WIDTH + GAP;
        }
        context.interaction.input_region(pill_quad);
    }
}
