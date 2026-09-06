use crate::{
    app::Background,
    platform::Platform,
    render::{
        Fragment, GAP, PANEL_START, TEXT_COLOR, UiContext,
        sdf::{Glass, Refraction, SurfaceSample, VISIBLE_ALPHA, hash, sample_pill},
        weathertime::{StatusSky, scene, sky_phase},
    },
};
use arrayvec::ArrayString;
use core::f32::consts::TAU;
use isthmus::{
    Float as _, Quad, ShaderData, Text, Unorm16x2,
    geometry::{
        FragmentGeometry,
        sdf::{self, Capsule, Circle, RoundedRect, SdfShape as _, Shape},
    },
    glam::{Vec2, Vec3, vec2, vec3},
    shader,
    spirv_std::arch::kill,
};
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

fn sample_graph<G: for<'a> FragmentGeometry<'a>>(outer: Quad, graph: Quad, fragment: &Fragment<G>) -> SurfaceSample {
    let surface = sample_pill(outer, fragment);
    surface.layer(Shape::pill(graph).distance_at(surface.refract(fragment.pixel)))
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct ProcessorStatus {
    pub temperature: f32,
    pub history: [Unorm16x2; STATUS_HISTORY_SAMPLES],
}

impl ProcessorStatus {
    fn record(&mut self, sample: ProcessorSample) {
        self.history.copy_within(1.., 0);
        self.history[HISTORY_END] = Unorm16x2::from_vec2(vec2(sample.usage, sample.memory));
    }
}

#[derive(Default)]
pub struct AudioMonitor {
    pub volume: AtomicU32,
    pub spectrum: [AtomicU32; AUDIO_SPECTRUM_BANDS],
}

pub struct StatusPanel {
    /// Battery charge magnitude, negated while charging.
    battery_level: Option<f32>,
    /// Logarithmic frequency-band levels sampled from the system audio monitor stream.
    audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS],
    /// Fractional scroll between the two newest history samples.
    history_scroll: f32,
    /// CPU temperature and usage history.
    cpu: ProcessorStatus,
    /// GPU temperature and usage history.
    gpu: ProcessorStatus,
    target_temperature: [f32; 2],
    action_hover: [f32; 2],
    audio_monitor: Arc<AudioMonitor>,
    background: Background,
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
        let audio_monitor = Arc::<AudioMonitor>::default();
        Platform::start_status_monitor(background.updater.clone(), Arc::clone(&audio_monitor));
        Self {
            battery_level: None,
            audio_spectrum: Default::default(),
            history_scroll: 0.0,
            cpu: ProcessorStatus::default(),
            gpu: ProcessorStatus::default(),
            target_temperature: [0.0; 2],
            action_hover: [0.0; 2],
            audio_monitor,
            background: background.clone(),
        }
    }

    pub(crate) fn record(&mut self, update: SystemSample) {
        self.target_temperature[0] = update.cpu.temperature;
        self.cpu.record(update.cpu);
        if let Some(gpu) = update.gpu {
            self.target_temperature[1] = gpu.temperature;
            self.gpu.record(gpu);
        }
        self.battery_level = update.battery_level;
        self.history_scroll = 0.0;
    }

    pub fn show(&mut self, context: &mut UiContext, sky: StatusSky) {
        let mut volume = f32::from_bits(self.audio_monitor.volume.load(Ordering::Relaxed));
        let height = context.config.height;
        let width = self.width();
        let x = context.frame.screen_size.x - width - GAP;
        let pill = Shape::pill(Quad::from_min_max(vec2(x, PANEL_START), vec2(x + width, PANEL_START + height)));
        let pill_quad = pill.quad;
        let mut cursor = STATUS_INSET;
        let section = |center: f32, section_width: f32| {
            Quad::new(vec2(x + center, PANEL_START + height * 0.5), vec2(section_width, height), Vec2::X)
        };

        context.frame.paint(
            pill.with_effect(Glass),
            shader!({
                let sky: StatusSky = sky;
                |fragment: Fragment<Capsule>| {
                    let sample = sample_pill(fragment.quad, &fragment);
                    if sample.alpha <= VISIBLE_ALPHA {
                        kill();
                    }

                    // Weather-reactive glass body and interaction flash.
                    let color = scene(
                        fragment.time,
                        fragment.globals.bar_height,
                        sample.refracted,
                        sample.size.x,
                        sky_phase(sky.sun_height),
                        sky.conditions,
                    );
                    sample.color(color)
                }
            }),
        );

        let temperature_blend = 1.0 - (-5.0 * context.frame.delta_time).exp();
        for (processor, target) in [&mut self.cpu, &mut self.gpu].into_iter().zip(self.target_temperature) {
            processor.temperature += (target - processor.temperature) * temperature_blend;
        }
        self.history_scroll = (self.history_scroll
            + context.frame.delta_time / Platform::STATUS_SAMPLE_INTERVAL.as_secs_f32())
        .saturate();
        for processor in [self.cpu, self.gpu] {
            let center = cursor + GRAPH_WIDTH * 0.5;
            let graph_pill = Quad::from_min_max(
                vec2(x + cursor, PANEL_START + GAP),
                vec2(x + cursor + GRAPH_WIDTH, PANEL_START + height - GAP),
            );
            let mut label = ArrayString::<16>::new();
            write!(
                label,
                "{:.0}% {:.0}% {:.0}\u{b0}C",
                processor.history[HISTORY_END].to_vec2().x * 100.0,
                processor.history[HISTORY_END].to_vec2().y * 100.0,
                processor.temperature,
            )
            .unwrap();
            let half_width = GRAPH_WIDTH * 0.5 - GAP * 0.5;
            let line =
                context.frame.text.line(&label, 11.0, 700.0).fit(GAP + 5.0, center - half_width..center + half_width);
            context.frame.paint(
                Shape::pill(graph_pill).with_effect(Glass),
                shader!({
                    let pill_quad: Quad = pill_quad;
                    let history: isthmus::Buffer<'_, Unorm16x2> = isthmus::Buffer::new(&processor.history);
                    let temperature: f32 = processor.temperature;
                    let history_scroll: f32 = self.history_scroll;
                    |fragment: Fragment<Capsule>| {
                        let graph_pill = fragment.quad;
                        let surface = sample_graph(pill_quad, graph_pill, &fragment);
                        let point = graph_pill.local(surface.refract(fragment.pixel));
                        let half_width = graph_pill.size.x * 0.5;
                        let radius = graph_pill.size.y * 0.5;

                        // Scrolling usage and memory histories.
                        let history_step = half_width * 2.0 / HISTORY_END as f32;
                        let sample =
                            ((point.x + half_width) / history_step + history_scroll).clamp(0.0, HISTORY_END as f32);
                        let index = sample.floor() as usize;
                        let graph_height = radius - 2.0;
                        let start = (1.0 - history.load(index).to_vec2() * 2.0) * graph_height;
                        let end = (1.0 - history.load((index + 1).min(HISTORY_END)).to_vec2() * 2.0) * graph_height;
                        let curve = |start: f32, end: f32, color: Vec3, fill_strength: f32| {
                            let delta = end - start;
                            let t = sample.fract();
                            let graph_y = start + delta * t * t * (3.0 - 2.0 * t);
                            let slope = delta * 6.0 * t * (1.0 - t) / history_step;
                            let field = (graph_y - point.y) / (1.0 + slope * slope).sqrt();
                            color
                                * surface.mask
                                * (sdf::fill(field) * fill_strength + sdf::stroke(field, CHART_LINE_WIDTH))
                        };
                        let graphs =
                            curve(start.x, end.x, USAGE_COLOR, 0.156) + curve(start.y, end.y, MEMORY_COLOR, 0.084);

                        // Grid and smoothly temperature-tinted package context.
                        let cell = (((point + vec2(half_width, radius)) / vec2(7.0, 6.1)).fract() - 0.5).abs();
                        let grid =
                            surface.mask * cell.x.smoothstep(0.49, 0.46).max(cell.y.smoothstep(0.49, 0.45)) * 0.045;
                        let heat = vec3(0.22, 0.62, 1.0)
                            .lerp(vec3(1.0, 0.38, 0.08), temperature.smoothstep(60.0, 72.0))
                            .lerp(vec3(1.0, 0.08, 0.035), temperature.smoothstep(72.0, 88.0));
                        let frame_color = vec3(0.025, 0.09, 0.15)
                            .lerp(USAGE_COLOR, 0.18 + history.load(HISTORY_END).to_vec2().x * 0.24)
                            .lerp(heat, temperature.smoothstep(60.0, 86.0) * 0.9);
                        let hardware = sdf::stroke(surface.distance, 1.45);
                        let color = vec3(0.004, 0.012, 0.026).lerp(frame_color, hardware) + Vec3::splat(grid) + graphs;
                        surface.fill_color(color)
                    }
                }),
            );
            context.frame.paint(
                line.with_effect(Refraction).translated(vec2(x, PANEL_START)),
                shader!({
                    let pill_quad: Quad = pill_quad;
                    let graph_pill: Quad = graph_pill;
                    |text: Fragment<Text>| {
                        let surface = sample_graph(pill_quad, graph_pill, &text);
                        surface.text(&text)
                    }
                }),
            );
            cursor += GRAPH_WIDTH + GAP;
        }

        if let Some(battery_level) = self.battery_level {
            let center = cursor + DATA_WIDTH * 0.5;
            context.frame.paint(
                section(center, DATA_WIDTH),
                shader!({
                    let pill_quad: Quad = pill_quad;
                    let battery_level: f32 = battery_level;
                    |fragment: Fragment<Quad>| {
                        let surface = sample_pill(pill_quad, &fragment);
                        let point = (fragment.local + surface.displacement()) / 0.8;

                        // Battery shell and fill boundary.
                        let charging = if battery_level < 0.0 { 1.0 } else { 0.0 };
                        let level = battery_level.abs();
                        let body = Shape::rounded_rect(vec2(23.0, 30.0), 3.2).translated(vec2(0.0, 1.0));
                        let terminal = Shape::rounded_rect(vec2(8.0, 3.6), 0.8).translated(vec2(0.0, -15.6));
                        let shell = body.union(terminal).fill_at(point);
                        let inside = Shape::rounded_rect(vec2(17.0, 24.0), 1.7).distance_at(point - vec2(0.0, 1.0));
                        let surface = 12.0 - level.saturate() * 24.0;
                        let wave = (point.x * 0.62 + fragment.time * (1.4 + charging * 1.2)).sin() * 1.15
                            + (point.x * 0.27 - fragment.time * 0.8).sin() * 0.45;
                        let liquid = sdf::fill(inside.max(surface + wave - (point.y - 1.0)));

                        // Charge-dependent liquid color.
                        let liquid_color = vec3(1.0, 0.18, 0.10)
                            .lerp(vec3(1.0, 0.72, 0.12), level.smoothstep(0.08, 0.28))
                            .lerp(vec3(0.22, 0.95, 0.55), level.smoothstep(0.18, 0.72));

                        // Rising bubbles only while charging.
                        let column = (point.x / 3.0).floor();
                        let seed = hash(vec2(column, 0.0));
                        let cycle = (fragment.time * (0.35 + seed.y * 0.5) + seed.x * 7.0).fract();
                        let center = vec2((column + 0.2 + seed.x * 0.6) * 3.0, 13.0 - cycle * 24.0);
                        let distance = (point - center).length() - (0.4 + seed.y * 0.5);
                        let fade = cycle.smoothstep(0.0, 0.25) * cycle.smoothstep(1.0, 0.7);
                        let bubble = sdf::fill((distance.abs() - 0.45).max(inside)) * fade * charging;
                        let color = TEXT_COLOR.lerp(liquid_color, liquid) * shell
                            + liquid_color.lerp(Vec3::ONE, 0.72) * bubble * 0.9;
                        let alpha = shell.max(liquid).max(bubble);
                        (color / alpha.max(0.0001)).extend(alpha)
                    }
                }),
            );
            cursor += DATA_WIDTH + GAP;
        }

        for (damped, level) in self.audio_spectrum.iter_mut().zip(&self.audio_monitor.spectrum) {
            let target = f32::from_bits(level.load(Ordering::Relaxed));
            let response = if target > *damped { 18.0 } else { 6.0 };
            *damped += (target - *damped) * (1.0 - (-response * context.frame.delta_time).exp());
        }
        let center = cursor + DATA_WIDTH * 0.5;
        let audio = Shape::rectangle(section(center, DATA_WIDTH));
        let scroll = context.interaction.interact(audio).scroll;
        if scroll != 0 {
            let sign = volume.signum();
            volume = (volume.abs() - scroll as f32 * 0.05).saturate() * sign;
            self.audio_monitor.volume.store(volume.to_bits(), Ordering::Relaxed);
            Platform::set_volume(volume.abs());
        }
        context.frame.paint(
            audio.with_effect(Glass).with_effect(Refraction),
            shader!({
                let pill_quad: Quad = pill_quad;
                let audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS] = self.audio_spectrum;
                let volume: f32 = volume;
                |fragment: Fragment<RoundedRect>| {
                    let surface = sample_pill(pill_quad, &fragment);
                    let point = fragment.quad.local(surface.refract(fragment.pixel));
                    // Seven-band spectrum.
                    let muted = if volume < 0.0 { 1.0 } else { 0.0 };
                    let volume = volume.abs();
                    let middle = (AUDIO_SPECTRUM_BANDS - 1) as f32 * 0.5;
                    let bar =
                        (point.x / AUDIO_BAR_SPACING + middle).round().clamp(0.0, AUDIO_SPECTRUM_BANDS as f32 - 1.0);
                    let active = audio_spectrum[bar as usize] * (1.0 - muted);
                    let height = 1.2 + 7.7 * active;
                    let bars = Shape::pill(vec2(AUDIO_BAR_RADIUS, height) * 2.0)
                        .fill_at(point - vec2((bar - middle) * AUDIO_BAR_SPACING, -1.5));

                    // Volume rail and filled level.
                    let rail_point = point - vec2(0.0, 11.5);
                    let rail = Shape::pill(vec2(AUDIO_HALF_WIDTH, AUDIO_BAR_RADIUS) * 2.0).fill_at(rail_point);
                    let level_x = AUDIO_HALF_WIDTH * (volume.saturate() * 2.0 - 1.0);
                    let level = rail * rail_point.x.smoothstep(level_x + 0.8, level_x - 0.8);

                    let color = vec3(0.18, 0.96, 1.0);
                    let spectrum_color = color * bars;
                    let level_color = color.lerp(MUTED_COLOR, muted) * level;
                    let alpha = bars.max(level);
                    ((spectrum_color + level_color) / alpha.max(0.0001)).extend(alpha)
                }
            }),
        );
        cursor += DATA_WIDTH + GAP;

        for action in [1usize, 0] {
            let center = cursor + ACTION_WIDTH * 0.5;
            let shape = Shape::circle(vec2(x + center, PANEL_START + height * 0.5), ACTION_WIDTH.min(height) * 0.5);
            let response = context.interaction.interact(shape);
            self.action_hover[action] =
                self.action_hover[action].move_towards(f32::from(response.hovered), context.frame.delta_time / 0.12);
            if response.hovered && response.held_for(1.5) {
                Platform::run_power_action(&self.background, action);
            }
            context.frame.paint(
                shape.with_effect(Glass),
                shader!({
                    let panel: Quad = pill_quad;
                    let reboot: bool = action == 1;
                    let hover: f32 = self.action_hover[action];
                    let selected: f32 = f32::from(response.held && response.hovered);
                    let charge: f32 = (response.held_seconds / 1.5).saturate() * selected;
                    let progress = 1.0 - selected + charge;
                    let ring: sdf::Arc =
                        Shape::arc(Vec2::ZERO, 7.1, TAU * 0.08, TAU * (progress * 0.82 - 0.045).max(0.0)).shape;
                    let direction: Vec2 = Vec2::from_angle(TAU * (0.08 + 0.82 * progress));
                    |fragment: Fragment<Circle>| {
                        let surface = sample_pill(panel, &fragment);
                        let point = (surface.refract(fragment.pixel) - fragment.center) / (1.0 + hover * 0.07);

                        // Power icon morphs inward as the hold completes.
                        let (icon, expanded) = if reboot {
                            // Reboot icon draws a progressing circular arrow.
                            let progress = 1.0 - selected + charge;
                            let tangent = vec2(-direction.y, direction.x);
                            let arrow = point - direction * 7.1;
                            let arrow = vec2(arrow.dot(tangent), arrow.dot(direction));
                            let ring = ring.distance_at(point);
                            let arrow = Shape::chevron(vec2(-3.2, 2.1)).offset(1.0).distance_at(arrow);
                            let glyph = |expansion: f32| {
                                (sdf::stroke(ring, 1.05 + expansion) * progress.smoothstep(0.0, 0.02))
                                    .max(sdf::fill(arrow - expansion))
                            };
                            (glyph(0.0), glyph(0.8))
                        } else {
                            let ease = charge.smoothstep(0.0, 1.0);
                            let radius =
                                7.5 - charge * 4.6 + (fragment.time * 8.0).sin() * charge * (1.0 - charge) * 0.16;
                            let ring = Shape::circle(Vec2::ZERO, radius).stroke(1.05 + ease * 0.7);
                            let gap =
                                Shape::rounded_rect(vec2(6.0 * (1.0 - charge), 6.0), 0.5).translated(vec2(0.0, -7.0));
                            let stem = Shape::rounded_rect(vec2(2.1 + ease * 0.9, 9.2 - charge * 6.0), 0.7)
                                .translated(vec2(0.0, -5.0 + charge * 3.5));
                            let (fill, outline) = ring.difference(gap).union(stem).fill_outline_at(point, 0.8);
                            (fill, fill + outline)
                        };
                        let color = TEXT_COLOR.lerp(vec3(0.95, 0.42, 0.4), hover.max(selected * (0.5 + charge * 0.5)));
                        let outline = (expanded - icon).max(0.0) * 0.18;
                        let alpha = icon + outline;
                        (color * (1.0 + charge * 0.45) * icon / alpha.max(0.0001)).extend(alpha)
                    }
                }),
            );
            cursor += ACTION_WIDTH + GAP;
        }
        context.interaction.input_region(pill_quad);
    }
}
