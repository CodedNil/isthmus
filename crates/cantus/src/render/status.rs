use crate::{
    app::Background,
    platform,
    render::{
        GAP, PANEL_START, Program, TEXT_COLOR, TEXT_SHADOW, TEXT_SMALL, UiContext,
        sdf::{deform, glass, hash, lens, sample_deformation},
        weathertime::{StatusSky, WeatherCondition, scene, sky_phase},
    },
};
use arrayvec::ArrayString;
use core::f32::consts::TAU;
use isthmus::prelude::*;
use isthmus_2d::prelude::*;
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
const AUDIO_WIDTH: f32 = 20.0;
const ACTION_WIDTH: f32 = 24.0;
/// Width of the displayed range below each band's peak, in the meter's normalised decibels (about 21 dB).
const AUDIO_SPAN: f32 = 0.22;
/// Share of the visualiser height over which the volume level fades.
const VOLUME_FADE: f32 = 0.24;
/// Fraction of the panel height at each end where releasing the power icon triggers an action.
const SNAP_FRACTION: f32 = 0.2;
/// Pull at which the icon is armed, i.e. the pointer has entered a snap zone.
const ARMED_PULL: f32 = 1.0 - SNAP_FRACTION * 2.0;
const ICON_RADIUS: f32 = 8.6;
/// One stroke width for every part of the power icon.
const ICON_STROKE: f32 = 2.4;
const POWER_DRAG_ID: u64 = u64::MAX;
/// Graph width, fixed so the battery slot does not resize it.
const GRAPH_WIDTH: f32 = (60.0 + f32::midpoint(DATA_WIDTH, GAP)) * 0.8;
const CHART_LINE_WIDTH: f32 = 0.85;
const USAGE_COLOR: Vec3 = Vec3::new(0.32, 0.68, 1.0);
const MEMORY_COLOR: Vec3 = Vec3::new(0.78, 0.3, 1.0);

const HISTORY_END: usize = STATUS_HISTORY_SAMPLES - 1;
/// Padding around the contents, matching the gap above and below the graphs.
const STATUS_INSET: f32 = GAP;
const BASE_WIDTH: f32 = GRAPH_WIDTH * 2.0 + AUDIO_WIDTH + ACTION_WIDTH + GAP * 3.0 + STATUS_INSET * 2.0;

fn spectrum_level(spectrum: [f32; AUDIO_SPECTRUM_BANDS], y: f32, half_height: f32) -> f32 {
    let position = ((y / half_height * 0.5 + 0.5).saturate()) * (AUDIO_SPECTRUM_BANDS - 1) as f32;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(AUDIO_SPECTRUM_BANDS - 1);
    spectrum[lower].lerp(spectrum[upper], position.fract().smoothstep(0.0, 1.0)).saturate()
}

fn power_action(pointer_y: f32, panel: Rect) -> Option<platform::PowerAction> {
    let snap_depth = panel.size().y * SNAP_FRACTION;
    if pointer_y <= panel.min.y + snap_depth {
        Some(platform::PowerAction::PowerOff)
    } else if pointer_y >= panel.max.y - snap_depth {
        Some(platform::PowerAction::Reboot)
    } else {
        None
    }
}

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
    /// Frequency-band levels from the system audio monitor.
    audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS],
    /// Slowly decaying per-band peak, used to auto-scale each band to the full visual range.
    audio_peak: [f32; AUDIO_SPECTRUM_BANDS],
    /// Fractional scroll between the two newest history samples.
    history_scroll: f32,
    processors: [ProcessorStatus; 2],
    action_origin: Option<Vec2>,
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
    /// Pill width, including the battery slot when shown.
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
        let panel = Rect::new(vec2(x, PANEL_START), vec2(x + width, PANEL_START + height));
        let mut cursor = STATUS_INSET;
        let section = |center: f32, section_width: f32| {
            Rect::from_center_size(vec2(x + center, PANEL_START + height * 0.5), vec2(section_width, height))
        };

        let conditions: WeatherCondition = sky.conditions;
        let phase = sky_phase(sky.sun_height);
        shader!(
            context.frame,
            |frame, conditions: WeatherCondition, panel: Rect, phase: Vec3| deform(Shape::pill(panel), frame),
            |frame, surface| {
                glass(
                    surface,
                    scene(
                        frame.time,
                        frame.globals.bar_height,
                        surface.refracted - panel.min,
                        panel.size().x,
                        phase,
                        conditions,
                    ),
                )
            },
        );

        let temperature_blend = 1.0 - (-5.0 * context.frame.delta_time).exp();
        self.history_scroll = (self.history_scroll
            + context.frame.delta_time / platform::STATUS_SAMPLE_INTERVAL.as_secs_f32())
        .saturate();
        for processor in &mut self.processors {
            processor.temperature += (processor.target_temperature - processor.temperature) * temperature_blend;
            let center = cursor + GRAPH_WIDTH * 0.5;
            let graph_pill = Rect::new(
                vec2(x + cursor, PANEL_START + GAP),
                vec2(x + cursor + GRAPH_WIDTH, PANEL_START + height - GAP),
            );
            let mut label = ArrayString::<16>::new();
            let latest = processor.history[HISTORY_END].to_vec2() * 100.0;
            write!(label, "{:.0}% {:.0}% {:.0}\u{b0}C", latest.x, latest.y, processor.temperature).unwrap();
            let half_width = GRAPH_WIDTH * 0.5 - GAP * 0.5;

            let history = &processor.history;
            let history_scroll = self.history_scroll;
            let frame_color = {
                let heat = vec3(0.22, 0.62, 1.0)
                    .lerp(vec3(1.0, 0.38, 0.08), processor.temperature.smoothstep(60.0, 72.0))
                    .lerp(vec3(1.0, 0.08, 0.035), processor.temperature.smoothstep(72.0, 88.0));
                vec3(0.025, 0.09, 0.15)
                    .lerp(USAGE_COLOR, 0.18 + history[HISTORY_END].to_vec2().x * 0.24)
                    .lerp(heat, processor.temperature.smoothstep(60.0, 86.0) * 0.9)
            };
            shader!(
                context.frame,
                |frame,
                 panel: Rect,
                 history: &[Unorm16x2],
                 history_scroll: f32,
                 graph_pill: Rect,
                 frame_color: Vec3| lens(Shape::pill(panel), frame, Shape::pill(graph_pill)),
                |_, surface| {
                    let point = graph_pill.local(surface.refracted);
                    let half_width = graph_pill.size().x * 0.5;
                    let radius = graph_pill.size().y * 0.5;
                    let history_step = half_width * 2.0 / HISTORY_END as f32;
                    let graph_height = radius - 2.0;
                    let curve = |channel: Vec2, color: Vec3, fill_strength: f32| {
                        let distance = {
                            let sample = ((point.x + half_width) / history_step + history_scroll)
                                .clamp_num(0.0, HISTORY_END as f32);
                            let index = sample.floor() as usize;
                            let start = (1.0 - history[index].to_vec2().dot(channel) * 2.0) * graph_height;
                            let end = (1.0 - history[(index + 1).min(HISTORY_END)].to_vec2().dot(channel) * 2.0)
                                * graph_height;
                            let delta = end - start;
                            let t = sample.fract();
                            let graph_y = start + delta * t * t * (3.0 - 2.0 * t);
                            let slope = delta * 6.0 * t * (1.0 - t) / history_step;
                            (graph_y - point.y) / (1.0 + slope * slope).sqrt()
                        };
                        let sample = Sample::new(distance, 0.0);
                        color * (sample.fill() * fill_strength + sample.band(-CHART_LINE_WIDTH..CHART_LINE_WIDTH))
                    };
                    let graphs = curve(Vec2::X, USAGE_COLOR, 0.156) + curve(Vec2::Y, MEMORY_COLOR, 0.084);
                    let cell = (((point + vec2(half_width, radius)) / vec2(7.0, 6.1)).fract() - 0.5).abs();
                    let grid = cell.x.smoothstep(0.49, 0.46).max(cell.y.smoothstep(0.49, 0.45)) * 0.045;
                    let hardware = surface.sdf.band(-1.45..1.45);
                    let color = vec3(0.004, 0.012, 0.026).lerp(frame_color, hardware) + Vec3::splat(grid) + graphs;
                    glass(surface, color)
                },
            );
            let line = context
                .frame
                .resources
                .line(&label, TEXT_SMALL, 700.0)
                .fit(GAP + 5.0, center - half_width..center + half_width)
                .translated(vec2(x, PANEL_START));
            context.paint_text(panel, panel.size().y * 0.5, line, TEXT_COLOR.extend(1.0));
            cursor += GRAPH_WIDTH + GAP;
        }

        if let Some(battery_level) = self.battery_level {
            let center = cursor + DATA_WIDTH * 0.5;

            let rect = section(center, DATA_WIDTH);
            let charging = if battery_level < 0.0 { 1.0 } else { 0.0 };
            let level = battery_level.abs();
            let liquid_color = vec3(1.0, 0.18, 0.10)
                .lerp(vec3(1.0, 0.72, 0.12), level.smoothstep(0.08, 0.28))
                .lerp(vec3(0.22, 0.95, 0.55), level.smoothstep(0.18, 0.72));
            shader!(
                context.frame,
                |_, panel: Rect, rect: Rect, charging: f32, level: f32, liquid_color: Vec3| rect,
                |frame, surface| {
                    let surface = sample_deformation(Shape::pill(panel), frame, surface.pixel);
                    let point = rect.local(surface.refracted) / 0.8;
                    let body = Shape::rounded_rect(vec2(23.0, 30.0), 3.2).translated(vec2(0.0, 1.0));
                    let terminal = Shape::rounded_rect(vec2(8.0, 3.6), 0.8).translated(vec2(0.0, -15.6));
                    let shell = body.union(terminal).fill_at(point);
                    let inside = Shape::rounded_rect(vec2(17.0, 24.0), 1.7).translated(vec2(0.0, 1.0));
                    let liquid_y = 12.0 - level.saturate() * 24.0;
                    let wave = (point.x * 0.62 + frame.time * (1.4 + charging * 1.2)).sin() * 1.15
                        + (point.x * 0.27 - frame.time * 0.8).sin() * 0.45;
                    let liquid =
                        Sample::new(inside.distance_at(point).max(liquid_y + wave - (point.y - 1.0)), 0.0).fill();
                    let column = (point.x / 3.0).floor();
                    let seed = hash(vec2(column, 0.0));
                    let cycle = (frame.time * (0.35 + seed.y * 0.5) + seed.x * 7.0).fract();
                    let center = vec2((column + 0.2 + seed.x * 0.6) * 3.0, 13.0 - cycle * 24.0);
                    let bubble = Shape::circle(center, 0.4 + seed.y * 0.5).stroke(0.9).intersection(inside);
                    let fade = cycle.smoothstep(0.0, 0.25) * cycle.smoothstep(1.0, 0.7);
                    let bubble = bubble.fill_at(point) * fade * charging;
                    (TEXT_COLOR.lerp(liquid_color, liquid) + liquid_color.lerp(Vec3::ONE, 0.72) * bubble * 0.9)
                        .extend(shell)
                },
            );
            cursor += DATA_WIDTH + GAP;
        }

        // Each band spans a fixed range below its own recent peak, so quiet streams still reach the full width.
        let mut audio_spectrum = [0.0; AUDIO_SPECTRUM_BANDS];
        for (((damped, peak), scaled), level) in self
            .audio_spectrum
            .iter_mut()
            .zip(&mut self.audio_peak)
            .zip(&mut audio_spectrum)
            .zip(&self.audio_monitor.spectrum)
        {
            let target = f32::from_bits(level.load(Ordering::Relaxed));
            let response = if target > *damped { 20.0 } else { 6.0 };
            *damped += (target - *damped) * (1.0 - (-response * context.frame.delta_time).exp());
            *peak = (*peak - context.frame.delta_time * 0.1).max(*damped).max(0.1);
            *scaled = ((*damped - (*peak - AUDIO_SPAN)) / AUDIO_SPAN).saturate().powf(1.4);
        }
        let audio_center = cursor + AUDIO_WIDTH * 0.5;
        let audio_rect = section(audio_center, AUDIO_WIDTH);
        let scroll = context.interaction.interact("volume", Shape::rectangle(audio_rect)).scroll;
        if scroll != 0 {
            volume = (volume.abs() - scroll as f32 * 0.05).saturate() * volume.signum();
            self.audio_monitor.volume.store(volume.to_bits(), Ordering::Relaxed);
            platform::set_volume(volume.abs());
        }

        // The ribbon may use the whole section width, so its widest extent sits a gap from its neighbours. The
        // lens layer is padded so antialiasing and the shadow are never clipped.
        let visualizer_rect = Rect::from_center_size(audio_rect.center(), vec2(AUDIO_WIDTH, height - GAP * 2.0));
        let audio_rect = Rect::from_center_size(audio_rect.center(), audio_rect.size() + vec2(8.0, 0.0));
        shader!(
            context.frame,
            |frame,
             panel: Rect,
             audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS],
             volume: f32,
             audio_rect: Rect,
             visualizer_rect: Rect| lens(Shape::pill(panel), frame, Shape::rectangle(audio_rect)),
            |frame, surface| {
                let size = visualizer_rect.size();
                let half_height = size.y * 0.5;
                let local = visualizer_rect.local(surface.refracted);
                let time = frame.time;

                // Spectrum ribbon with fully rounded ends. Levels are stretched so typical music spans the whole
                // range, and loud bands make the ribbon snake sideways as well as swell.
                let level = spectrum_level(audio_spectrum, local.y, half_height);
                let ripple = (local.y * 0.72 + time * 1.8).sin() * 0.16 + (local.y * 1.13 - time * 1.2).sin() * 0.08;
                let max_half_width = size.x * 0.5 - 1.0;
                let sway = (local.y * 0.45 - time * 3.2).sin() * level * 2.2;
                let width = (2.0 + level * 6.5 + ripple * (1.0 + level * 5.0)).min(max_half_width - sway.abs());
                let ribbon = {
                    // Its tips always reach the full height, however thin the ribbon is.
                    let radius = width.max(1.0);
                    Shape::pill(Rect::from_center_size(Vec2::ZERO, vec2(radius * 2.0, size.y)))
                        .distance_at(vec2(local.x - sway, local.y))
                };
                let alpha = Sample::new(ribbon, 0.0).fill() * 0.92 + (1.0 - ribbon.abs() / 1.6).saturate() * 0.08;

                // Volume fills from the bottom; the boundary eases over a soft gradient.
                let from_top = 0.5 + local.y / size.y;
                let edge = (1.0 + VOLUME_FADE) * (1.0 - volume.abs().saturate()) - VOLUME_FADE * 0.5;
                let lit = from_top.smoothstep(edge - VOLUME_FADE * 0.5, edge + VOLUME_FADE * 0.5);
                // Brightest right at the level, easing back towards the base of the fill.
                let sheen = lit * (1.0 - (from_top - edge).max(0.0) * 0.8).saturate();
                let color = vec3(0.18, 0.52, 0.72)
                    .lerp(vec3(0.18, 0.88, 1.0), lit)
                    .lerp(vec3(0.7, 0.97, 1.0), sheen * 0.2 + level * 0.25 * lit);
                // A soft dark halo so the ribbon stands out against bright backgrounds.
                let shadow = (-ribbon.max(0.0) * 0.7).exp() * 0.28;
                let total = alpha + shadow * (1.0 - alpha);
                (color * alpha / total.max(0.0001)).extend(total)
            },
        );
        cursor += AUDIO_WIDTH + GAP;

        let icon_home = vec2(x + cursor + ACTION_WIDTH * 0.5, panel.center().y);
        let response = context.interaction.drag(POWER_DRAG_ID, Shape::circle(icon_home, 13.0));
        let drag = context.interaction.drag_motion().filter(|(id, ..)| *id == POWER_DRAG_ID);
        let pointer = context.interaction.mouse_pos();
        if response.held && self.action_origin.is_none() {
            self.action_origin = Some(drag.map_or(pointer, |(_, offset, _)| pointer - offset));
        }
        let pointer_y = match (drag, self.action_origin) {
            (Some((_, offset, _)), Some(origin)) => origin.y + offset.y,
            _ => pointer.y,
        };
        if drag.is_some_and(|(_, _, released)| released)
            && let Some(action) = power_action(pointer_y, panel)
        {
            platform::run_power_action(action);
        }
        let dragging = response.held || drag.is_some();
        if !response.held {
            self.action_origin = None;
        }
        let pull = if dragging { ((pointer_y - panel.center().y) / (height * 0.5)).clamp_num(-1.0, 1.0) } else { 0.0 };
        let icon_y = if dragging {
            pointer_y.clamp_num(panel.min.y + ICON_RADIUS, panel.max.y - ICON_RADIUS)
        } else {
            icon_home.y
        };
        let snap_progress = ((pull.abs() - ARMED_PULL) / (1.0 - ARMED_PULL)).smoothstep(0.0, 1.0);
        let icon_scale = 1.0 + snap_progress * 0.35;
        let action_rect = Rect::from_center_size(icon_home, vec2(ACTION_WIDTH, height) + Vec2::splat(GAP * 2.0));
        shader!(
            context.frame,
            |_, panel: Rect, action_rect: Rect, icon_home: Vec2, icon_y: f32, pull: f32, icon_scale: f32| action_rect,
            |frame, surface| {
                let surface = sample_deformation(Shape::pill(panel), frame, surface.pixel);
                let icon_center = panel.local(vec2(icon_home.x, icon_y));
                let point = panel.local(surface.content);
                let icon_point = (point - icon_center) / icon_scale;
                // One arc throughout: closed at rest, opening at the top (shutdown stem grows) when pulled up, or at
                // the side (reboot arrowhead grows) when pulled down.
                let up = (-pull).saturate();
                let down = pull.saturate();
                let half_gap = (up * 0.62 + down * TAU * 0.12).max(0.02);
                let gap_center = -TAU * 0.25 * (1.0 - down);
                let ring = Shape::arc(Vec2::ZERO, ICON_RADIUS, gap_center + half_gap, TAU - half_gap * 2.0);

                let stem_end = vec2(0.0, -1.0 - up * (ICON_RADIUS - 1.0));
                let stem = Shape::segment(vec2(0.0, -1.0), stem_end, ICON_STROKE);
                // Shrink the stem to nothing at rest rather than leaving a dot in the middle.
                let stem_distance = stem.distance_at(icon_point) + (1.0 - (up * 8.0).saturate()) * ICON_STROKE;

                let direction = Vec2::from_angle(gap_center - half_gap);
                let tip = direction * ICON_RADIUS;
                // The small floor keeps the segment from degenerating to a point at rest.
                let barb = |side: f32| {
                    Shape::segment(tip, tip - (direction.perp() - direction * side) * (0.05 + 3.0 * down), ICON_STROKE)
                };
                let distance = ring
                    .stroke(ICON_STROKE)
                    .distance_at(icon_point)
                    .min(stem_distance)
                    .min(barb(1.0).distance_at(icon_point))
                    .min(barb(-1.0).distance_at(icon_point));
                let icon = Sample::new(distance, 1.6);
                let armed = ((pull.abs() - ARMED_PULL) / 0.08).saturate();
                let color = TEXT_COLOR.lerp(vec3(1.0, 0.24, 0.2), armed);
                icon.paint(color.extend(1.0), TEXT_SHADOW.with_w(TEXT_SHADOW.w * 1.7))
            },
        );
        context.interaction.input_region(panel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_interpolates_smoothly_between_vertical_samples() {
        let levels = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        assert_eq!(spectrum_level(levels, -6.0, 6.0), 0.0);
        assert!((spectrum_level(levels, -5.0, 6.0) - 0.5).abs() < 0.001);
        assert_eq!(spectrum_level(levels, 0.0, 6.0), 1.0);
        assert_eq!(spectrum_level(levels, 6.0, 6.0), 0.0);
    }

    #[test]
    fn power_gesture_uses_the_status_pill_snap_zones() {
        let panel = Rect::new(vec2(0.0, 10.0), vec2(24.0, 110.0));
        assert!(matches!(power_action(10.0, panel), Some(platform::PowerAction::PowerOff)));
        assert!(matches!(power_action(29.9, panel), Some(platform::PowerAction::PowerOff)));
        assert!(power_action(30.1, panel).is_none());
        assert!(power_action(panel.center().y, panel).is_none());
        assert!(power_action(89.9, panel).is_none());
        assert!(matches!(power_action(90.1, panel), Some(platform::PowerAction::Reboot)));
        assert!(matches!(power_action(110.0, panel), Some(platform::PowerAction::Reboot)));
    }
}
