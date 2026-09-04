use core::f32::consts::TAU;

#[cfg(target_arch = "spirv")]
use isthmus::Float as _;
use isthmus::{
    Quad, Sdf, ShaderData,
    glam::{FloatExt, Vec2, Vec3, vec2, vec3},
    spirv_std::arch::kill,
};

use crate::render::{
    Fragment, GAP, Globals, PANEL_START, TEXT_COLOR, TextFragment,
    sdf::{
        PILL_MARGIN, SurfaceSample, VISIBLE_ALPHA, hash, sample_pill, sd_capsule_box, sd_chevron, sd_rounded_box,
        segment_distance,
    },
    weathertime::{StatusSky, scene, sky_phase},
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

fn sample_graph(outer: Quad, graph: Quad, pixel: Vec2, globals: Globals, time: f32) -> SurfaceSample {
    let surface = sample_pill(outer, pixel, globals, time);
    let radius = graph.size.y * 0.5;
    surface.layer(sd_capsule_box(
        graph.local(surface.refract(pixel)),
        graph.size.x * 0.5 - radius,
        radius,
    ))
}

#[repr(C)]
#[derive(Clone, Copy, Default, ShaderData)]
pub struct ProcessorStatus {
    pub temperature: f32,
    pub usage: [f32; STATUS_HISTORY_SAMPLES],
    pub memory: [f32; STATUS_HISTORY_SAMPLES],
}

#[isthmus::paint]
mod host {
    use std::{
        fmt::Write,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
            mpsc::{self, Receiver},
        },
    };

    use arrayvec::ArrayString;

    use super::*;
    use crate::{app::Background, interaction::Rect, platform::Platform, render::UiContext};

    impl ProcessorStatus {
        fn record(&mut self, sample: ProcessorSample) {
            for (history, value) in [(&mut self.usage, sample.usage), (&mut self.memory, sample.memory)] {
                history.copy_within(1.., 0);
                history[HISTORY_END] = value.saturate();
            }
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
        updates: Receiver<SystemSample>,
    }

    #[derive(Clone, Copy)]
    pub struct ProcessorSample {
        pub temperature: f32,
        pub usage: f32,
        pub memory: f32,
    }

    pub struct SystemSample {
        pub cpu: ProcessorSample,
        pub gpu: Option<ProcessorSample>,
        pub battery_level: Option<f32>,
    }

    impl StatusPanel {
        /// Total on-screen width of the pill; grows to include the battery slot when shown.
        pub const fn width(&self) -> f32 {
            BASE_WIDTH
                + if self.battery_level.is_some() {
                    DATA_WIDTH + GAP
                } else {
                    0.0
                }
        }

        pub(crate) fn new(background: &Background) -> Self {
            let audio_monitor = Arc::<AudioMonitor>::default();
            let (updates, inbox) = mpsc::channel();
            Platform::start_status_monitor(updates, Arc::clone(&audio_monitor));
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
                updates: inbox,
            }
        }

        pub fn show(&mut self, context: &mut UiContext, sky: StatusSky) {
            while let Ok(update) = self.updates.try_recv() {
                self.target_temperature[0] = update.cpu.temperature;
                self.cpu.record(update.cpu);
                if let Some(gpu) = update.gpu {
                    self.target_temperature[1] = gpu.temperature;
                    self.gpu.record(gpu);
                }
                self.battery_level = update.battery_level;
                self.history_scroll = 0.0;
            }
            let mut volume = f32::from_bits(self.audio_monitor.volume.load(Ordering::Relaxed));
            let height = context.config.height;
            let width = self.width();
            let x = context.frame.screen_size.x - width - GAP;
            let pill = Rect::new(x, PANEL_START, x + width, PANEL_START + height);
            let pill_quad: Quad = pill.into();
            let mut cursor = STATUS_INSET;
            let section = |center: f32, section_width: f32| {
                Rect::from_center(
                    vec2(x + center, PANEL_START + height * 0.5),
                    vec2(section_width * 0.5, height * 0.5),
                )
            };

            // GPU: Status background.
            context.frame.paint(
                pill_quad.expanded(PILL_MARGIN),
                |fragment: Fragment, pill_quad: Quad, sky: StatusSky| {
                    let sample = sample_pill(pill_quad, fragment.pixel, fragment.globals, fragment.time);
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
                },
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
                    processor.usage[HISTORY_END] * 100.0,
                    processor.memory[HISTORY_END] * 100.0,
                    processor.temperature,
                )
                .unwrap();
                let half_width = GRAPH_WIDTH * 0.5 - GAP * 0.5;
                let line = context
                    .frame
                    .text()
                    .line(&label, 11.0, 700.0)
                    .fit(GAP + 5.0, center - half_width..center + half_width);
                // GPU: Processor monitor.
                context.frame.paint(
                    graph_pill.expanded(PILL_MARGIN),
                    |fragment: Fragment,
                     pill_quad: Quad,
                     graph_pill: Quad,
                     processor: ProcessorStatus,
                     history_scroll: f32| {
                        let surface =
                            sample_graph(pill_quad, graph_pill, fragment.pixel, fragment.globals, fragment.time);
                        let point = graph_pill.local(surface.refract(fragment.pixel));
                        let half_width = graph_pill.size.x * 0.5;
                        let radius = graph_pill.size.y * 0.5;

                        // Scrolling usage and memory histories.
                        let history_step = half_width * 2.0 / HISTORY_END as f32;
                        let sample =
                            ((point.x + half_width) / history_step + history_scroll).clamp(0.0, HISTORY_END as f32);
                        let index = sample.floor() as usize;
                        let graph_height = radius - 2.0;
                        let curve = |history: &[f32; STATUS_HISTORY_SAMPLES], color: Vec3, fill_strength: f32| {
                            let height = |i: usize| graph_height * (1.0 - history[i.min(HISTORY_END)] * 2.0);
                            let at =
                                |i: usize| vec2((i as f32 - history_scroll) * history_step - half_width, height(i));
                            let start = at(index);
                            let end = at(index + 1);
                            let line = segment_distance(point, start, end).stroke(CHART_LINE_WIDTH);
                            let graph_y = start.y.lerp(end.y, sample.fract().smoothstep(0.0, 1.0));
                            color * surface.mask * (Sdf::new(graph_y - point.y).fill() * fill_strength + line)
                        };
                        let graphs =
                            curve(&processor.usage, USAGE_COLOR, 0.156) + curve(&processor.memory, MEMORY_COLOR, 0.084);

                        // Grid and smoothly temperature-tinted package context.
                        let cell = (((point + vec2(half_width, radius)) / vec2(7.0, 6.1)).fract() - 0.5).abs();
                        let grid =
                            surface.mask * cell.x.smoothstep(0.49, 0.46).max(cell.y.smoothstep(0.49, 0.45)) * 0.045;
                        let heat = vec3(0.22, 0.62, 1.0)
                            .lerp(vec3(1.0, 0.38, 0.08), processor.temperature.smoothstep(60.0, 72.0))
                            .lerp(vec3(1.0, 0.08, 0.035), processor.temperature.smoothstep(72.0, 88.0));
                        let frame_color = vec3(0.025, 0.09, 0.15)
                            .lerp(USAGE_COLOR, 0.18 + processor.usage[HISTORY_END] * 0.24)
                            .lerp(heat, processor.temperature.smoothstep(60.0, 86.0) * 0.9);
                        let hardware = Sdf::new(surface.distance).stroke(1.45);
                        let color = vec3(0.004, 0.012, 0.026).lerp(frame_color, hardware) + Vec3::splat(grid) + graphs;
                        surface.fill_color(color)
                    },
                );
                // GPU: Processor label.
                context.frame.paint_text(
                    line.translated(vec2(x, PANEL_START)),
                    |text: TextFragment, pill_quad: Quad, graph_pill: Quad| {
                        let surface = sample_graph(pill_quad, graph_pill, text.pixel, text.globals, text.time);
                        text.color(text.alpha_at(surface.content_point(text.pixel)) * surface.mask)
                    },
                );
                cursor += GRAPH_WIDTH + GAP;
            }

            if let Some(battery_level) = self.battery_level {
                let center = cursor + DATA_WIDTH * 0.5;
                // GPU: Battery indicator.
                context.frame.paint(
                    section(center, DATA_WIDTH),
                    |fragment: Fragment, pill_quad: Quad, battery_level: f32| {
                        let surface = sample_pill(pill_quad, fragment.pixel, fragment.globals, fragment.time);
                        let point = (fragment.local + surface.displacement()) / 0.8;

                        // Battery shell and fill boundary.
                        let charging = if battery_level < 0.0 { 1.0 } else { 0.0 };
                        let level = battery_level.abs();
                        let body = sd_rounded_box(point - vec2(0.0, 1.0), vec2(11.5, 15.0), 3.2);
                        let terminal = sd_rounded_box(point - vec2(0.0, -15.6), vec2(4.0, 1.8), 0.8);
                        let shell = body.union(terminal).fill();
                        let inside = sd_rounded_box(point - vec2(0.0, 1.0), vec2(8.5, 12.0), 1.7).fill();
                        let surface = 12.0 - level.saturate() * 24.0;
                        let wave = (point.x * 0.62 + fragment.time * (1.4 + charging * 1.2)).sin() * 1.15
                            + (point.x * 0.27 - fragment.time * 0.8).sin() * 0.45;
                        let liquid = inside * (point.y - 1.0).smoothstep(surface + wave - 0.7, surface + wave + 0.7);

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
                        let bubble = Sdf::new(distance).stroke(0.45) * fade * inside * charging;
                        let color = TEXT_COLOR.lerp(liquid_color, liquid) * shell
                            + liquid_color.lerp(Vec3::ONE, 0.72) * bubble * 0.9;
                        let alpha = shell.max(liquid).max(bubble);
                        (color / alpha.max(0.0001)).extend(alpha)
                    },
                );
                cursor += DATA_WIDTH + GAP;
            }

            for (damped, level) in self.audio_spectrum.iter_mut().zip(&self.audio_monitor.spectrum) {
                let target = f32::from_bits(level.load(Ordering::Relaxed));
                let response = if target > *damped { 18.0 } else { 6.0 };
                *damped += (target - *damped) * (1.0 - (-response * context.frame.delta_time).exp());
            }
            let center = cursor + DATA_WIDTH * 0.5;
            let audio_rect = section(center, DATA_WIDTH);
            let scroll = context.interaction.interact(audio_rect).scroll;
            let audio: Quad = audio_rect.into();
            if scroll != 0 {
                let sign = volume.signum();
                volume = (volume.abs() - scroll as f32 * 0.05).saturate() * sign;
                self.audio_monitor.volume.store(volume.to_bits(), Ordering::Relaxed);
                Platform::set_volume(volume.abs());
            }
            // GPU: Audio spectrum and volume.
            context.frame.paint(
                audio.expanded(PILL_MARGIN),
                |fragment: Fragment,
                 pill_quad: Quad,
                 audio: Quad,
                 audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS],
                 volume: f32| {
                    let surface = sample_pill(pill_quad, fragment.pixel, fragment.globals, fragment.time);
                    let point = audio.local(surface.refract(fragment.pixel));
                    // Seven-band spectrum.
                    let muted = if volume < 0.0 { 1.0 } else { 0.0 };
                    let volume = volume.abs();
                    let middle = (AUDIO_SPECTRUM_BANDS - 1) as f32 * 0.5;
                    let bar = (point.x / AUDIO_BAR_SPACING + middle)
                        .round()
                        .clamp(0.0, AUDIO_SPECTRUM_BANDS as f32 - 1.0);
                    let active = audio_spectrum[bar as usize] * (1.0 - muted);
                    let height = 1.2 + 7.7 * active;
                    let bars = sd_rounded_box(
                        point - vec2((bar - middle) * AUDIO_BAR_SPACING, -1.5),
                        vec2(AUDIO_BAR_RADIUS, height),
                        AUDIO_BAR_RADIUS,
                    )
                    .fill();

                    // Volume rail and filled level.
                    let rail_point = point - vec2(0.0, 11.5);
                    let rail =
                        sd_rounded_box(rail_point, vec2(AUDIO_HALF_WIDTH, AUDIO_BAR_RADIUS), AUDIO_BAR_RADIUS).fill();
                    let level_x = AUDIO_HALF_WIDTH * (volume.saturate() * 2.0 - 1.0);
                    let level = rail * rail_point.x.smoothstep(level_x + 0.8, level_x - 0.8);

                    let color = vec3(0.18, 0.96, 1.0);
                    let spectrum_color = color * bars;
                    let level_color = color.lerp(MUTED_COLOR, muted) * level;
                    let alpha = bars.max(level);
                    ((spectrum_color + level_color) / alpha.max(0.0001)).extend(alpha)
                },
            );
            cursor += DATA_WIDTH + GAP;

            for action in [1usize, 0] {
                let center = cursor + ACTION_WIDTH * 0.5;
                let response = context.interaction.interact(section(center, ACTION_WIDTH));
                self.action_hover[action] = self.action_hover[action]
                    .move_towards(f32::from(response.hovered()), context.frame.delta_time / 0.12);
                if response.hovered() && response.held_for(1.5) {
                    Platform::run_power_action(&self.background, action);
                }
                let hover = self.action_hover[action];
                let selected = f32::from(response.held() && response.hovered());
                let power_progress = (response.held_seconds / 1.5).saturate();
                let reboot = action == 1;
                // GPU: Power action icon.
                context.frame.paint(
                    <Quad>::from(section(center, ACTION_WIDTH)).expanded(PILL_MARGIN),
                    |fragment: Fragment,
                     pill_quad: Quad,
                     reboot: bool,
                     hover: f32,
                     selected: f32,
                     power_progress: f32| {
                        let surface = sample_pill(pill_quad, fragment.pixel, fragment.globals, fragment.time);
                        let point = (fragment.local + surface.displacement()) / (1.0 + hover * 0.07);
                        let charge = power_progress * selected;

                        // Power icon morphs inward as the hold completes.
                        let (icon, expanded) = if reboot {
                            // Reboot icon draws a progressing circular arrow.
                            const START: f32 = TAU * 0.08;
                            const SWEEP: f32 = TAU * 0.82;
                            let progress = 1.0 - selected + charge;
                            let phase = ((point.y.atan2(point.x) - START) / TAU + 1.0).fract();
                            let arc_end = (progress * 0.82 - 0.045).max(0.0);
                            let arc_mask =
                                phase.smoothstep(arc_end + 0.008, arc_end - 0.008) * progress.smoothstep(0.0, 0.02);
                            let angle = START + SWEEP * progress;
                            let direction = vec2(angle.cos(), angle.sin());
                            let tangent = vec2(-direction.y, direction.x);
                            let arrow = point - direction * 7.1;
                            let arrow = vec2(arrow.dot(tangent), arrow.dot(direction));
                            let glyph = |expansion: f32| {
                                (Sdf::new(point.length() - 7.1).stroke(1.05 + expansion) * arc_mask)
                                    .max((sd_chevron(arrow, vec2(-3.2, 2.1)) - 1.0 - expansion).fill())
                            };
                            (glyph(0.0), glyph(0.8))
                        } else {
                            let ease = charge.smoothstep(0.0, 1.0);
                            let radius =
                                7.5 - charge * 4.6 + (fragment.time * 8.0).sin() * charge * (1.0 - charge) * 0.16;
                            let glyph = |expansion: f32| {
                                let ring = Sdf::new(point.length() - radius).stroke(1.05 + ease * 0.7 + expansion);
                                let gap = sd_rounded_box(point - vec2(0.0, -7.0), vec2(3.0 * (1.0 - charge), 3.0), 0.5)
                                    .fill();
                                let stem = (sd_rounded_box(
                                    point - vec2(0.0, -5.0 + charge * 3.5),
                                    vec2(1.05 + ease * 0.45, 4.6 - charge * 3.0),
                                    0.7,
                                ) - expansion)
                                    .fill();
                                (ring * (1.0 - gap)).max(stem)
                            };
                            (glyph(0.0), glyph(0.8))
                        };
                        let color = TEXT_COLOR.lerp(vec3(0.95, 0.42, 0.4), hover.max(selected * (0.5 + charge * 0.5)));
                        let outline = (expanded - icon).max(0.0) * 0.18;
                        let alpha = icon + outline;
                        (color * (1.0 + charge * 0.45) * icon / alpha.max(0.0001)).extend(alpha)
                    },
                );
                cursor += ACTION_WIDTH + GAP;
            }
            context.interaction.input_region(pill);
        }
    }
}
