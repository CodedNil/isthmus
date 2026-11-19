use crate::{app::Background, config::Config, interaction::Interaction, music::Music, timer::Timer};
use arrayvec::ArrayString;
use isthmus::prelude::*;
use isthmus_2d::prelude::*;
use sdf::refract;
use std::sync::Once;

pub mod launcher;
pub mod lyrics;
pub mod music;
pub mod sdf;
pub mod status;
pub mod weathertime;

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;

/// Space for playlist buttons and compact history tracks, in logical pixels.
pub const HISTORY_WIDTH: f32 = 200.0;

/// Fraction of a pill's height at which its bottom content row is centred.
pub const SUPPORT_ROW: f32 = 0.975;

/// Centre of a pill's bottom content row, which a support shape hangs beneath.
pub fn support_row(pill: Rect) -> Vec2 {
    vec2(pill.center().x, pill.min.y + pill.size().y * SUPPORT_ROW - 1.0)
}

pub const TEXT_COLOR: Vec3 = Vec3::splat(0.94);
/// Straight-alpha shadow painted behind text for readability.
pub const TEXT_SHADOW: Vec4 = Vec4::new(0.2, 0.2, 0.2, 0.18);
/// Corner radius of the bar's expanded glass panels.
pub const PANEL_RADIUS: f32 = 16.0;
/// Blend radius where a support shape merges into its pill.
pub const SUPPORT_BLEND: f32 = UNIT * 2.0;
/// Distance a support shape's top sits above the pill's lower edge.
pub const SUPPORT_INSET: f32 = 8.0;
/// Text sizes, from the smallest caption to the largest display label.
pub const TEXT_SMALL: f32 = 12.0;
pub const TEXT_BODY: f32 = 14.0;
pub const TEXT_TITLE: f32 = 16.0;
pub const TEXT_HEADING: f32 = 20.0;
pub const TEXT_DISPLAY: f32 = 24.0;
/// Gap between the top of the surface and the top of the bar.
pub const PANEL_START: f32 = 6.0;
/// Base spacing unit. Sizes and gaps should be whole multiples of it.
pub const UNIT: f32 = 4.0;
/// The standard small gap between adjacent elements.
pub const GAP: f32 = UNIT * 2.0;
/// The standard inset between a container edge and its contents.
pub const PADDING: f32 = UNIT * 3.0;
pub const RIPPLE_COUNT: usize = 4;

#[derive(Clone, Copy, Default, ShaderData)]
pub struct Globals {
    pub pointer: Vec2,
    pub pressure: f32,
    pub bar_height: f32,
    pub ripples: [RipplePulse; RIPPLE_COUNT],
}

isthmus::program!(Globals);

/// Installs the fonts every line of text is shaped with, once.
pub fn install_fonts() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        isthmus_2d::install_fonts([
            include_bytes!("../../assets/NotoSans-Variable.ttf").as_slice(),
            include_bytes!("../../assets/NotoSansSymbols-Music.ttf").as_slice(),
        ]);
    });
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct RipplePulse {
    pub origin: Vec2,
    pub start_time: f32,
}

pub struct UiContext<'frame> {
    pub frame: Frame<'frame>,
    pub config: &'frame Config,
    pub interaction: &'frame mut Interaction,
}

#[derive(Clone, Copy, ShaderData)]
pub struct BarLayout {
    pub playhead_x: f32,
    pub px_per_ms: f32,
}

struct WeatherReadout {
    label: ArrayString<64>,
    hour: f32,
    pill_width: f32,
}

pub struct Bar {
    pub(crate) weather: Option<weathertime::WeatherPanel>,
    pub(crate) status: Option<status::StatusPanel>,
    music_view: music::MusicView,
}

impl Bar {
    pub fn new(config: &Config, background: &Background) -> Self {
        Self {
            weather: config.weathertime_enabled.then(|| weathertime::WeatherPanel::new(&config.timezones, background)),
            status: config.status_enabled.then(|| status::StatusPanel::new(background)),
            music_view: music::MusicView::default(),
        }
    }

    pub fn show(&mut self, context: &mut UiContext, music: &mut Music, timer: &Timer) {
        let status_width = self.status.as_ref().map_or(0.0, |status| status.width() + GAP);
        let weather_readout = self.weather.as_ref().map(weathertime::WeatherPanel::collapsed_readout);
        let reserved = HISTORY_WIDTH
            + GAP
            + weather_readout.as_ref().map_or(0.0, |readout| readout.pill_width + GAP)
            + status_width;
        let px_per_ms =
            (context.frame.screen_size.x - reserved).max(84.0) / (context.config.timeline_future_minutes * 60_000.0);
        let layout = BarLayout {
            playhead_x: HISTORY_WIDTH + context.config.timeline_past_minutes * 60_000.0 * px_per_ms,
            px_per_ms,
        };
        let drag = music
            .queue
            .tracks
            .iter()
            .find_map(|track| context.interaction.drag_of(track.interaction_id))
            .map(|drag| (drag.offset.x / px_per_ms, drag.scroll / px_per_ms, drag.released));
        music.update_playback(drag);
        if context.config.lyrics_enabled {
            lyrics::show(context, music, layout);
        }
        let sky = match (self.weather.as_mut(), weather_readout.as_ref()) {
            (Some(weather), Some(readout)) => weather.show(context, status_width, timer, readout),
            _ => weathertime::StatusSky::default(),
        };
        if let Some(status) = self.status.as_mut() {
            status.show(context, sky);
        }
        self.music_view.show(context, music, layout);
    }
}

impl UiContext<'_> {
    fn paint_text(&mut self, rect: Rect, radius: f32, line: Text, color: Vec4) {
        self.frame
            .shader("glass text")
            .with((rect, radius, line, color))
            .shape(|frame| refract(Shape::rounded_rect(rect, radius), frame, line))
            .paint(|_, _| color);
    }
}
