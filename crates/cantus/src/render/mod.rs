use crate::{app::Background, config::Config, interaction::Interaction, music::Music, timer::Timer};
use isthmus::prelude::*;
use isthmus_sdf::{layout::TextCache, prelude::*};
use sdf::refract;

pub mod launcher;
pub mod lyrics;
pub mod music;
pub mod sdf;
pub mod status;
pub mod weathertime;

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
pub const HELD_PRESSURE: f32 = 2.0;

#[derive(Clone, Copy, Default, ShaderData)]
pub struct Globals {
    pub pointer: Vec2,
    pub pressure: f32,
    pub bar_height: f32,
    pub ripples: [RipplePulse; RIPPLE_COUNT],
}

isthmus::program!(Globals, TextCache);

#[derive(Clone, Copy, Default, ShaderData)]
pub struct RipplePulse {
    pub origin: Vec2,
    pub start_time: f32,
}

pub struct UiContext<'a> {
    pub frame: Frame<'a>,
    pub config: &'a Config,
    pub interaction: &'a mut Interaction,
}

#[derive(Clone, Copy)]
pub struct BarLayout {
    pub playhead_x: f32,
    pub px_per_ms: f32,
}

impl BarLayout {
    pub fn local_scale(self, duration: u32) -> f32 {
        if duration == 0 { self.px_per_ms } else { self.px_per_ms.max(240.0 / duration as f32) }
    }
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
        let reserved = HISTORY_WIDTH
            + GAP
            + f32::from(context.config.weathertime_enabled) * (weathertime::WIDTH + GAP)
            + status_width;
        let px_per_ms =
            (context.frame.screen_size.x - reserved).max(84.0) / (context.config.timeline_future_minutes * 60_000.0);
        self.music_view.animate(music, context.frame.delta_time, context.config.height);
        let layout = BarLayout {
            playhead_x: HISTORY_WIDTH + context.config.timeline_past_minutes * 60_000.0 * px_per_ms,
            px_per_ms,
        };
        let drag = context.interaction.drag_motion();
        let drag = drag.map(|(id, offset, released)| {
            let scale = music
                .local
                .as_ref()
                .filter(|local| local.track.interaction_id == id)
                .map_or(px_per_ms, |local| layout.local_scale(local.track.duration_ms));
            (id, offset.x / scale, released)
        });
        music.update_playback(drag, context.frame.delta_time);
        if context.config.lyrics_enabled {
            lyrics::show(context, music, layout);
        }
        let sky = self
            .weather
            .as_mut()
            .map_or_else(weathertime::StatusSky::default, |weather| weather.show(context, status_width, timer));
        if let Some(status) = self.status.as_mut() {
            status.show(context, sky);
        }
        self.music_view.show(context, music, layout);
    }
}

impl UiContext<'_> {
    fn paint_text(&mut self, rect: Rect, radius: f32, line: Text, color: Vec4) {
        shader!(
            self.frame
                .upload({
                    let rect: Rect;
                    let radius: f32;
                    let line: Text;
                    let color: Vec4;
                })
                .primitive(|frame| refract(Shape::rounded_rect(rect, radius), frame, line))
                .fragment(|_, _| color)
        );
    }
}
