use crate::{
    app::Background,
    platform::{
        self,
        mpris::{MediaNotice, PlaybackStatus},
    },
    render::{
        GAP, PANEL_START, SUPPORT_INSET, TEXT_COLOR, TEXT_SHADOW, TEXT_SMALL, UiContext,
        sdf::{deform, glass, hash, lens, refract, sample_deformation},
        weathertime::{StatusSky, WeatherCondition, scene, sky_phase},
    },
};
use arrayvec::ArrayString;
use core::f32::consts::{FRAC_1_SQRT_2, TAU};
use isthmus::prelude::*;
use isthmus_2d::prelude::*;
use std::{
    fmt::Write,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

pub const OSD_EXTENSION: f32 = 18.0;
const OSD_HOLD: Duration = Duration::from_millis(2600);
const OSD_BLEND: f32 = 12.0;
const OSD_PADDING: f32 = 12.0;
const OSD_ICON_SLOT: f32 = 24.0;
const STATUS_HISTORY_SAMPLES: usize = 32;
pub const AUDIO_SPECTRUM_BANDS: usize = 7;
const DATA_WIDTH: f32 = 32.0;
const AUDIO_WIDTH: f32 = 20.0;
const ACTION_WIDTH: f32 = 24.0;
/// Displayed range below each band's peak (about 21 dB).
const AUDIO_SPAN: f32 = 0.22;
/// Lowest scaling window, so silence stays flat.
const AUDIO_SILENCE_MARGIN: f32 = 0.08;
/// Share of the visualiser height where the volume level fades.
const VOLUME_FADE: f32 = 0.24;
/// Pull at which releasing the power icon runs its action.
const ARMED_PULL: f32 = 0.6;
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
/// Padding around the contents.
const STATUS_INSET: f32 = GAP;
const BASE_WIDTH: f32 = GRAPH_WIDTH * 2.0 + AUDIO_WIDTH + ACTION_WIDTH + GAP * 3.0 + STATUS_INSET * 2.0;

fn spectrum_level(spectrum: [f32; AUDIO_SPECTRUM_BANDS], y: f32, half_height: f32) -> f32 {
    let position = ((y / half_height * 0.5 + 0.5).saturate()) * (AUDIO_SPECTRUM_BANDS - 1) as f32;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(AUDIO_SPECTRUM_BANDS - 1);
    spectrum[lower].lerp(spectrum[upper], position.fract().smoothstep(0.0, 1.0)).saturate()
}

/// Straight-alpha color over a dark halo for contrast.
fn with_halo(color: Vec3, alpha: f32, distance: f32, opacity: f32) -> Vec4 {
    let shadow = (-distance.max(0.0) * 0.7).exp() * opacity;
    let total = alpha + shadow * (1.0 - alpha);
    (color * alpha / total.max(0.0001)).extend(total)
}

/// Action on release; `pull` runs from -1 (up) to 1 (down).
fn power_action(pull: f32) -> Option<platform::PowerAction> {
    if pull >= ARMED_PULL {
        Some(platform::PowerAction::PowerOff)
    } else if pull <= -ARMED_PULL {
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

pub struct AudioMonitor {
    pub volume: AtomicU32,
    pub spectrum: [AtomicU32; AUDIO_SPECTRUM_BANDS],
}

impl Default for AudioMonitor {
    fn default() -> Self {
        Self { volume: AtomicU32::new(f32::NAN.to_bits()), spectrum: Default::default() }
    }
}

fn osd_bubble(panel: Rect, width: f32, presence: f32) -> Rect {
    let top = panel.max.y - SUPPORT_INSET;
    let spread = presence.smoothstep(0.12, 0.9);
    let half_width = 12.0f32.lerp(width * 0.5, spread);
    let depth = OSD_EXTENSION * presence;
    Rect::new(vec2(panel.center().x - half_width, top), vec2(panel.center().x + half_width, panel.max.y + depth))
}

fn status_shape(panel: Rect, bubble: Rect, presence: f32) -> impl Sdf {
    Shape::pill(panel).smooth_union(Shape::rounded_rect(bubble, 10.0), OSD_BLEND, presence.smoothstep(0.0, 0.6))
}

#[derive(Clone, Copy)]
enum OsdIcon {
    Play,
    Pause,
    Stop,
    Previous,
    Next,
    Brightness,
}

struct StatusOsd {
    label: String,
    icon: OsdIcon,
    shown: Instant,
}

#[derive(Default)]
pub struct StatusPanel {
    /// Battery charge magnitude, negated while charging.
    battery_level: Option<f32>,
    /// Frequency-band levels from the system audio monitor.
    audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS],
    /// Slowly decaying per-band peak, for auto-scaling.
    audio_peak: [f32; AUDIO_SPECTRUM_BANDS],
    last_volume: Option<f32>,
    volume_level: Ramp,
    volume_muted: Ramp,
    volume_pulse: Ramp,
    osd: Option<StatusOsd>,
    osd_presence: Ramp,
    osd_width: Ramp,
    /// Progress from one history sample to the next.
    history_scroll: Ramp,
    processors: [ProcessorStatus; 2],
    pub(crate) audio_monitor: Arc<AudioMonitor>,
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
        platform::mpris::watch_osd(background.updater.clone());
        panel
    }

    pub(crate) fn show_media(&mut self, notice: MediaNotice) {
        let icon = match (notice.status, notice.skip_forward) {
            (PlaybackStatus::Stopped, _) => OsdIcon::Stop,
            (_, Some(true)) => OsdIcon::Next,
            (_, Some(false)) => OsdIcon::Previous,
            (PlaybackStatus::Playing, None) => OsdIcon::Play,
            (PlaybackStatus::Paused, None) => OsdIcon::Pause,
        };
        let label = if notice.title.is_empty() {
            String::from("Stopped")
        } else if notice.artist.is_empty() {
            notice.title
        } else {
            format!("{}  ·  {}", notice.title, notice.artist)
        };
        self.osd = Some(StatusOsd { label, icon, shown: Instant::now() });
    }

    pub(crate) fn show_brightness(&mut self, level: f32) {
        let label = format!("{:.0}%", level * 100.0);
        self.osd = Some(StatusOsd { label, icon: OsdIcon::Brightness, shown: Instant::now() });
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
        self.history_scroll = Ramp::default();
    }

    pub fn show(&mut self, context: &mut UiContext, sky: StatusSky) {
        let mut volume: f32 = f32::from_bits(self.audio_monitor.volume.load(Ordering::Relaxed));
        let open = self.osd.as_ref().is_some_and(|media| media.shown.elapsed() < OSD_HOLD);
        let presence: f32 = self
            .osd_presence
            .approach_over(context.frame.time, f32::from(open), if open { 0.45 } else { 0.42 })
            .smoothstep(0.0, 1.0);
        let width = self.width();
        let x = context.frame.screen_size.x - width - GAP;
        let panel: Rect = Rect::new(vec2(x, PANEL_START), vec2(x + width, PANEL_START + context.config.height));
        let osd = self.osd.as_ref().filter(|_| open || presence > 0.0);
        let label = osd.map(|osd| Line::new(&osd.label, TEXT_SMALL, 650.0));
        let desired_width = label
            .as_ref()
            .map_or(0.0, |line| (line.text.width + OSD_ICON_SLOT + 2.0 * OSD_PADDING).min(width - 2.0 * GAP));
        let osd_width = self.osd_width.approach(context.frame.time, desired_width, 900.0);
        let bubble: Rect = osd_bubble(panel, osd_width, presence);
        let mut cursor = STATUS_INSET;
        let section = |center: f32, section_width: f32| {
            Rect::from_center_size(
                vec2(x + center, PANEL_START + context.config.height * 0.5),
                vec2(section_width, context.config.height),
            )
        };

        let phase: Vec3 = sky_phase(sky.sun_height);
        let conditions: WeatherCondition = sky.conditions;
        context
            .frame
            .shader("status sky")
            .with((conditions, panel, phase, bubble, presence))
            .shape(|frame| deform(status_shape(panel, bubble, presence), frame))
            .paint(|frame, surface| {
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
            });
        if let Some((osd, label)) = osd.zip(label)
            && presence > 0.0
        {
            Self::paint_osd(context, panel, bubble, presence, osd, &label);
        }

        let temperature_blend = 1.0 - (-5.0 * context.frame.delta_time).exp();
        self.history_scroll.approach_over(context.frame.time, 1.0, platform::STATUS_SAMPLE_INTERVAL.as_secs_f32());
        for processor in &mut self.processors {
            processor.temperature += (processor.target_temperature - processor.temperature) * temperature_blend;
            let center = cursor + GRAPH_WIDTH * 0.5;
            let graph_pill: Rect = Rect::new(
                vec2(x + cursor, PANEL_START + GAP),
                vec2(x + cursor + GRAPH_WIDTH, PANEL_START + context.config.height - GAP),
            );
            let mut label = ArrayString::<16>::new();
            let latest = processor.history[HISTORY_END].to_vec2() * 100.0;
            write!(label, "{:.0}% {:.0}% {:.0}\u{b0}C", latest.x, latest.y, processor.temperature).unwrap();
            let half_width = GRAPH_WIDTH * 0.5 - GAP * 0.5;

            let history: &[Unorm16x2] = &processor.history;
            let frame_color: Vec3 = {
                let heat = vec3(0.22, 0.62, 1.0)
                    .lerp(vec3(1.0, 0.38, 0.08), processor.temperature.smoothstep(60.0, 72.0))
                    .lerp(vec3(1.0, 0.08, 0.035), processor.temperature.smoothstep(72.0, 88.0));
                vec3(0.025, 0.09, 0.15)
                    .lerp(USAGE_COLOR, 0.18 + history[HISTORY_END].to_vec2().x * 0.24)
                    .lerp(heat, processor.temperature.smoothstep(60.0, 86.0) * 0.9)
            };
            let history_scroll: f32 = self.history_scroll.at(context.frame.time);
            context
                .frame
                .shader("status graph")
                .with((panel, history, history_scroll, graph_pill, frame_color))
                .shape(|frame| lens(Shape::pill(panel), frame, Shape::pill(graph_pill)))
                .paint(|_, surface| {
                    let point = graph_pill.local(surface.refracted);
                    let half_width = graph_pill.size().x * 0.5;
                    let radius = graph_pill.size().y * 0.5;
                    let history_step = half_width * 2.0 / HISTORY_END as f32;
                    let graph_height = radius - 2.0;

                    // Sample the history curve and convert its slope to a screen-space distance.
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

                    // Layer both curves over the grid and hardware frame.
                    let graphs = curve(Vec2::X, USAGE_COLOR, 0.156) + curve(Vec2::Y, MEMORY_COLOR, 0.084);
                    let cell = (((point + vec2(half_width, radius)) / vec2(7.0, 6.1)).fract() - 0.5).abs();
                    let grid = cell.x.smoothstep(0.49, 0.46).max(cell.y.smoothstep(0.49, 0.45)) * 0.045;
                    let hardware = surface.sdf.band(-1.45..1.45);
                    let color = vec3(0.004, 0.012, 0.026).lerp(frame_color, hardware) + Vec3::splat(grid) + graphs;
                    glass(surface, color)
                });
            let line = Line::new(&label, TEXT_SMALL, 700.0);
            let text = line
                .at(Vec2::ZERO)
                .fit(GAP + 5.0, center - half_width..center + half_width)
                .translated(vec2(x, PANEL_START));
            context.paint_text(panel, panel.size().y * 0.5, text, TEXT_COLOR.extend(1.0));
            cursor += GRAPH_WIDTH + GAP;
        }

        if let Some(battery_level) = self.battery_level {
            let center = cursor + DATA_WIDTH * 0.5;

            let level: f32 = battery_level.abs();
            let liquid_color: Vec3 = vec3(1.0, 0.18, 0.10)
                .lerp(vec3(1.0, 0.72, 0.12), level.smoothstep(0.08, 0.28))
                .lerp(vec3(0.22, 0.95, 0.55), level.smoothstep(0.18, 0.72));
            let rect: Rect = section(center, DATA_WIDTH);
            let charging: f32 = if battery_level < 0.0 { 1.0 } else { 0.0 };
            context
                .frame
                .shader("status battery")
                .with((panel, rect, charging, level, liquid_color))
                .bounds(rect)
                .paint(|frame, surface| {
                    let surface = sample_deformation(Shape::pill(panel), frame, surface.pixel);
                    let point = rect.local(surface.refracted) / 0.8;
                    let body = Shape::rounded_rect(vec2(23.0, 30.0), 3.2).translated(vec2(0.0, 1.0));
                    let terminal = Shape::rounded_rect(vec2(8.0, 3.6), 0.8).translated(vec2(0.0, -15.6));
                    let shell = body.union(terminal).fill_at(point);

                    // Fill the shell to the charge level with a moving surface.
                    let inside = Shape::rounded_rect(vec2(17.0, 24.0), 1.7).translated(vec2(0.0, 1.0));
                    let liquid_y = 12.0 - level.saturate() * 24.0;
                    let wave = (point.x * 0.62 + frame.time * (1.4 + charging * 1.2)).sin() * 1.15
                        + (point.x * 0.27 - frame.time * 0.8).sin() * 0.45;
                    let liquid =
                        Sample::new(inside.distance_at(point).max(liquid_y + wave - (point.y - 1.0)), 0.0).fill();

                    // Two bubbles per 3px column, each with its own speed and phase so they trickle steadily.
                    // Neighbouring columns are checked so none is cut at a column edge.
                    let mut bubble = 0.0f32;
                    for neighbour in 0..3 {
                        for layer in 0..2 {
                            let column = (point.x / 3.0).floor() + neighbour as f32 - 1.0;
                            let seed = hash(vec2(column, layer as f32));
                            let phase = hash(vec2(column, layer as f32 + 2.0)).x;
                            let cycle = (frame.time * (0.3 + seed.y * 0.5) + phase).fract();
                            let center = vec2((column + 0.2 + seed.x * 0.6) * 3.0, 13.0 - cycle * 24.0);
                            // Exact-distance antialiasing; screen derivatives break where the column changes.
                            let ring = ((point - center).length() - (0.4 + seed.y * 0.5)).abs() - 0.45;
                            let fade = cycle.smoothstep(0.0, 0.25) * cycle.smoothstep(1.0, 0.7);
                            bubble = bubble.max((0.5 - ring * 0.8).saturate() * fade);
                        }
                    }
                    let bubble = bubble * inside.fill_at(point) * charging;
                    // Bubbles are a darker shade of the liquid, so they show against it and the white shell.
                    TEXT_COLOR.lerp(liquid_color, liquid).lerp(liquid_color * 0.55, bubble).extend(shell)
                });
            cursor += DATA_WIDTH + GAP;
        }

        // Each band spans a fixed range below its own peak, so quiet audio still fills the width.
        let mut audio_spectrum: [f32; AUDIO_SPECTRUM_BANDS] = [0.0; AUDIO_SPECTRUM_BANDS];
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
            *peak = (*peak - context.frame.delta_time * 0.1).max(*damped).max(AUDIO_SPAN + AUDIO_SILENCE_MARGIN);
            *scaled = ((*damped - (*peak - AUDIO_SPAN)) / AUDIO_SPAN).saturate().powf(1.4);
        }
        let audio_center = cursor + AUDIO_WIDTH * 0.5;
        let audio_rect = section(audio_center, AUDIO_WIDTH);
        let scroll = context.interaction.interact("volume", Shape::rectangle(audio_rect)).scroll;
        if scroll != 0 {
            let current = if volume.is_finite() { volume } else { 0.0 };
            volume = (current.abs() - scroll as f32 * 0.05).saturate().copysign(current);
            self.audio_monitor.volume.store(volume.to_bits(), Ordering::Relaxed);
            platform::set_volume(volume.abs());
            self.volume_pulse.set(1.0);
        }
        if volume.is_finite() {
            let muted = volume.is_sign_negative();
            if let Some(previous) = self.last_volume {
                if previous.is_sign_negative() != muted || (previous.abs() - volume.abs()).abs() >= 0.005 {
                    self.volume_pulse.set(1.0);
                }
            } else {
                self.volume_level.set(volume.abs().saturate());
                self.volume_muted.set(f32::from(muted));
            }
            self.last_volume = Some(volume);
        }
        let level_target = self.last_volume.map_or(0.0, |sample| sample.abs().saturate());
        let mute_target = self.last_volume.is_some_and(f32::is_sign_negative);
        let volume: f32 = self.volume_level.approach(context.frame.time, level_target, 3.5);
        let muted: f32 = self.volume_muted.approach_over(context.frame.time, f32::from(mute_target), 0.2);
        let pulse: f32 = self.volume_pulse.approach_over(context.frame.time, 0.0, 0.55).smoothstep(0.0, 1.0);

        // The ribbon fills the section width; the lens layer is padded so nothing is clipped.
        let visualizer_rect: Rect =
            Rect::from_center_size(audio_rect.center(), vec2(AUDIO_WIDTH, context.config.height - GAP * 2.0));
        let audio_rect: Rect = Rect::from_center_size(audio_rect.center(), audio_rect.size() + vec2(8.0, 0.0));
        context
            .frame
            .shader("status audio")
            .with((panel, audio_spectrum, volume, muted, pulse, audio_rect, visualizer_rect))
            .shape(|frame| lens(Shape::pill(panel), frame, Shape::rectangle(audio_rect)))
            .paint(|frame, surface| {
                let size = visualizer_rect.size();
                let half_height = size.y * 0.5;
                let local = visualizer_rect.local(surface.refracted);

                // Spectrum ribbon: loud bands swell and sway it.
                let level = spectrum_level(audio_spectrum, local.y, half_height) * (1.0 - muted);
                let ripple =
                    (local.y * 0.72 + frame.time * 1.8).sin() * 0.16 + (local.y * 1.13 - frame.time * 1.2).sin() * 0.08;
                let max_half_width = size.x * 0.5 - 1.0;
                let sway = (local.y * 0.45 - frame.time * 3.2).sin() * level * 2.2;
                let width =
                    (2.0 + level * 6.5 + ripple * (1.0 + level * 5.0) + pulse * 3.0).min(max_half_width - sway.abs());
                let ribbon = {
                    // Tips always reach the full height.
                    let radius = width.max(1.0);
                    Shape::pill(Rect::from_center_size(Vec2::ZERO, vec2(radius * 2.0, size.y)))
                        .distance_at(vec2(local.x - sway, local.y))
                };
                let halo = (-ribbon.max(0.0) * 0.38).exp() * pulse * 0.28;
                let alpha =
                    (Sample::new(ribbon, 0.0).fill() * 0.92 + (1.0 - ribbon.abs() / 1.6).saturate() * 0.08 + halo)
                        .saturate();

                // Volume fills from the bottom; the boundary eases over a soft gradient.
                let from_top = 0.5 + local.y / size.y;
                let edge = (1.0 + VOLUME_FADE) * (1.0 - volume) - VOLUME_FADE * 0.5;
                let lit = from_top.smoothstep(edge - VOLUME_FADE * 0.5, edge + VOLUME_FADE * 0.5);
                // Brightest at the level, fading toward the base.
                let sheen = lit * (1.0 - (from_top - edge).max(0.0) * 0.8).saturate();
                let color = vec3(0.18, 0.52, 0.72)
                    .lerp(vec3(0.18, 0.88, 1.0), lit)
                    .lerp(vec3(0.7, 0.97, 1.0), sheen * 0.2 + level * 0.25 * lit)
                    .lerp(vec3(0.76, 0.23, 0.27).lerp(vec3(1.0, 0.42, 0.43), lit), muted)
                    .lerp(TEXT_COLOR, pulse * 0.32);
                with_halo(color, alpha, ribbon, 0.28)
            });
        cursor += AUDIO_WIDTH + GAP;

        let icon_home = vec2(x + cursor + ACTION_WIDTH * 0.5, panel.center().y);
        context.interaction.interact(POWER_DRAG_ID, Shape::circle(icon_home, 13.0));
        let drag = context.interaction.drag_of(POWER_DRAG_ID);
        let pointer_y = context.interaction.mouse_pos().y;
        let pull: f32 =
            drag.map_or(0.0, |_| ((pointer_y - icon_home.y) / (context.config.height * 0.5)).clamp_num(-1.0, 1.0));
        if let Some(action) = power_action(pull).filter(|_| drag.is_some_and(|drag| drag.released)) {
            platform::run_power_action(action);
        }
        let icon_y =
            drag.map_or(icon_home.y, |_| pointer_y.clamp_num(panel.min.y + ICON_RADIUS, panel.max.y - ICON_RADIUS));
        let snap_progress = ((pull.abs() - ARMED_PULL) / (1.0 - ARMED_PULL)).smoothstep(0.0, 1.0);
        let action_rect: Rect =
            Rect::from_center_size(icon_home, vec2(ACTION_WIDTH, context.config.height) + Vec2::splat(GAP * 2.0));
        let icon_center: Vec2 = vec2(icon_home.x, icon_y);
        let icon_scale: f32 = 1.0 + snap_progress * 0.35;
        context.frame.shader("status power").with((icon_center, pull, icon_scale)).bounds(action_rect).paint(
            |_, surface| {
                let (shutdown, reboot) = (pull.saturate(), (-pull).saturate());
                // No pointer lens, which would warp the thin strokes.
                let point = (surface.pixel - icon_center) / icon_scale;
                // One arc; pulling down opens the top and grows a stem, pulling up adds an arrowhead.
                let half_gap = (shutdown * 0.62 + reboot * TAU * 0.12).max(0.02);
                let gap_center = -TAU * 0.25 * (1.0 - reboot);
                let ring = Shape::arc(Vec2::ZERO, ICON_RADIUS, gap_center + half_gap, TAU - half_gap * 2.0);
                let stem = Shape::segment(vec2(0.0, -1.0), vec2(0.0, -1.0 - shutdown * (ICON_RADIUS - 1.0)), 0.0);
                let direction = Vec2::from_angle(gap_center - half_gap);
                let tip = direction * ICON_RADIUS;
                let barb = |side: f32| {
                    Shape::segment(tip, tip - (direction.perp() - direction * side) * 3.0 * reboot, 0.0)
                        .distance_at(point)
                };

                // Each part is a centerline; the stem and arrowhead fade in.
                let (stem_presence, barb_presence) = ((shutdown * 8.0).saturate(), (reboot * 8.0).saturate());
                // Screen-space distance to a part's stroke, and how much of it is drawn.
                let stroke = |centerline: f32, presence: f32| {
                    let distance = (centerline - ICON_STROKE * 0.5) * icon_scale;
                    (distance + (1.0 - presence) * ICON_RADIUS, (0.5 - distance).saturate() * presence)
                };
                let ring = stroke(ring.distance_at(point), 1.0);
                let stem = stroke(stem.distance_at(point), stem_presence);
                let barb = stroke(barb(1.0).min(barb(-1.0)), barb_presence);
                let nearest = ring.0.min(stem.0).min(barb.0);
                let alpha = ring.1.max(stem.1).max(barb.1);

                let armed = ((pull.abs() - ARMED_PULL) / 0.08).saturate();
                let color = TEXT_COLOR.lerp(vec3(1.0, 0.24, 0.2), armed);
                with_halo(color, alpha, nearest, 0.3)
            },
        );
        context.interaction.input_region(panel);
    }

    fn paint_osd(context: &mut UiContext, panel: Rect, bubble: Rect, presence: f32, osd: &StatusOsd, label: &Line) {
        let age = osd.shown.elapsed().as_secs_f32();
        let reveal = presence.smoothstep(0.34, 0.86);
        let opacity: f32 = reveal * age.smoothstep(0.0, 0.14);
        let direction: f32 = match osd.icon {
            OsdIcon::Previous => -1.0,
            OsdIcon::Next => 1.0,
            OsdIcon::Play | OsdIcon::Pause | OsdIcon::Stop | OsdIcon::Brightness => 0.0,
        };
        let settling = (age - 0.14).max(0.0);
        let spring = (settling * 23.0).cos() * (-settling * 12.0).exp();
        let center_y = f32::midpoint(panel.max.y, bubble.max.y);
        let text_width = label.text.width.min((bubble.size().x - 2.0 * OSD_PADDING - OSD_ICON_SLOT).max(0.0));
        let start = bubble.center().x - f32::midpoint(OSD_ICON_SLOT, text_width);
        let icon: Vec2 = vec2(start + 7.0 - 6.0 * direction * spring, center_y - 2.0 * (1.0 - reveal));
        let scale: f32 = 0.85 - 0.2 * spring;
        let brightness: bool = matches!(osd.icon, OsdIcon::Brightness);
        let stop: bool = matches!(osd.icon, OsdIcon::Stop);
        let play: bool = matches!(osd.icon, OsdIcon::Play);
        context
            .frame
            .shader("status OSD icon")
            .with((panel, bubble, presence, icon, opacity, direction, scale, brightness, stop, play))
            .bounds(Rect::from_center_size(icon, Vec2::splat(28.0)))
            .paint(|frame, surface| {
                let sample = sample_deformation(status_shape(panel, bubble, presence), frame, surface.pixel);
                let p = (sample.content - icon) / scale;
                let distance = if direction.abs() > 0.5 {
                    let x = p.x * direction;
                    let arrow = Shape::rounded_triangle(6.0, 0.8).distance_at(vec2(p.y, x + 2.2));
                    let bar = Shape::rounded_rect(vec2(2.0, 12.0), 0.8).distance_at(vec2(x - 5.5, p.y));
                    arrow.min(bar)
                } else if brightness {
                    let ring = (p.length() - 3.0).abs() - 0.7;
                    let ray = Shape::rounded_rect(vec2(1.4, 2.8), 0.7);
                    let axes = p.abs();
                    let diagonals = (vec2(p.x + p.y, p.x - p.y) * FRAC_1_SQRT_2).abs();
                    let straight = ray.distance_at(vec2(axes.min_element(), axes.max_element() - 6.6));
                    let diagonal = ray.distance_at(vec2(diagonals.min_element(), diagonals.max_element() - 6.6));
                    ring.min(straight).min(diagonal)
                } else if stop {
                    Shape::rounded_rect(Vec2::splat(11.5), 1.2).distance_at(p)
                } else if play {
                    Shape::rounded_triangle(7.0, 0.9).distance_at(vec2(p.y, p.x + 1.2))
                } else {
                    Shape::rounded_rect(vec2(2.8, 12.0), 1.1).distance_at(vec2(p.x.abs() - 3.1, p.y))
                };
                let fill = Sample::new(distance * scale, 0.0).fill();
                let clip = surface.pixel.y.smoothstep(panel.max.y, panel.max.y + 2.0) * sample.parent_fill;
                let tint = if brightness {
                    vec3(1.0, 0.88, 0.60)
                } else if direction.abs() > 0.5 {
                    vec3(0.76, 0.94, 1.0)
                } else {
                    TEXT_COLOR
                };
                let ink = with_halo(tint, fill, distance, 0.44);
                ink.with_w(ink.w * opacity * clip)
            });
        let left: f32 = start + OSD_ICON_SLOT;
        let overflow = (label.text.width - text_width).max(0.0);
        let right: f32 = left + text_width + if overflow > 0.0 { 0.0 } else { 10.0 };
        let alpha: f32 = reveal * age.smoothstep(0.06, 0.28);
        let slide = overflow * age.smoothstep(0.75, 2.2) * reveal;
        let text: Text = label.at(vec2(left - slide, center_y - 2.0 * (1.0 - alpha)));
        context
            .frame
            .shader("status OSD text")
            .with((panel, bubble, presence, text, alpha, left, right))
            .shape(|frame| refract(status_shape(panel, bubble, presence), frame, text.outlined(1.5)))
            .paint(|_, surface| {
                let clip = surface.pixel.x.smoothstep(left - 4.0, left)
                    * surface.pixel.x.smoothstep(right, right - 10.0)
                    * surface.pixel.y.smoothstep(panel.max.y, panel.max.y + 2.0);
                surface.paint(TEXT_COLOR.extend(1.0), TEXT_SHADOW.with_w(0.62))
                    * vec4(1.0, 1.0, 1.0, clip * surface.parent_fill * alpha)
            });
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
    fn dragging_the_power_icon_down_shuts_down_and_up_reboots() {
        assert!(matches!(power_action(0.61), Some(platform::PowerAction::PowerOff)));
        assert!(matches!(power_action(1.0), Some(platform::PowerAction::PowerOff)));
        assert!(matches!(power_action(-0.61), Some(platform::PowerAction::Reboot)));
        assert!(matches!(power_action(-1.0), Some(platform::PowerAction::Reboot)));
        for pull in [0.59, 0.0, -0.59] {
            assert!(power_action(pull).is_none());
        }
    }
}
