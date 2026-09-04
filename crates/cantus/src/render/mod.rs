pub mod launcher;
pub mod lyrics;
pub mod music;
pub mod sdf;
pub mod status;
pub mod weathertime;

#[cfg(not(target_arch = "spirv"))]
use crate::{
    app::Background,
    config::Config,
    interaction::Interaction,
    music::{Enrichment, Music},
};
#[cfg(not(target_arch = "spirv"))]
use isthmus::Frame;
use isthmus::{
    ShaderData,
    glam::{Vec2, Vec3},
};

pub const TEXT_COLOR: Vec3 = Vec3::splat(0.94);
/// Gap between the top of the surface and the top of the bar.
pub const PANEL_START: f32 = 6.0;
/// Base spacing unit. Sizes and gaps should be whole multiples of it.
pub const UNIT: f32 = 4.0;
/// The standard small gap between adjacent elements.
pub const GAP: f32 = UNIT * 2.0;
/// The standard inset between a container edge and its contents.
pub const PADDING: f32 = UNIT * 3.0;

#[repr(C)]
#[derive(Clone, Copy, Default, ShaderData)]
pub struct Globals {
    pub pointer: Vec2,
    pub pressure: f32,
    pub bar_height: f32,
    pub ripples: [RipplePulse; 4],
}

pub type Fragment = isthmus::Fragment<Globals>;
pub type TextFragment<'a> = isthmus::TextFragment<'a, Globals>;

isthmus::program!();

#[repr(C)]
#[derive(Clone, Copy, Default, ShaderData)]
pub struct RipplePulse {
    pub origin: Vec2,
    pub start_time: f32,
}

#[cfg(not(target_arch = "spirv"))]
pub struct UiContext<'a> {
    pub frame: Frame<'a>,
    pub config: &'a Config,
    pub interaction: &'a mut Interaction,
}

#[cfg(not(target_arch = "spirv"))]
#[derive(Clone, Copy)]
pub struct BarLayout {
    pub playhead_x: f32,
    pub px_per_ms: f32,
}

#[cfg(not(target_arch = "spirv"))]
impl<'a> UiContext<'a> {
    pub fn new(frame: Frame<'a>, config: &'a Config, interaction: &'a mut Interaction) -> Self {
        interaction.begin_frame(frame.delta_time, frame.time);
        Self { frame, config, interaction }
    }

    pub fn finish(mut self) {
        self.frame.set_globals(Globals {
            pointer: self.interaction.mouse_pos(),
            pressure: self.interaction.mouse_pressure(),
            bar_height: self.config.height,
            ripples: self.interaction.mouse_ripples(),
        });
        self.interaction.end_frame();
    }
}

#[cfg(not(target_arch = "spirv"))]
pub struct Bar {
    pub(crate) weather: Option<weathertime::WeatherPanel>,
    pub(crate) status: Option<status::StatusPanel>,
    music_view: music::MusicView,
}

#[cfg(not(target_arch = "spirv"))]
impl Bar {
    pub fn new(config: &Config, background: &Background, enrichment: &Enrichment) -> Self {
        Self {
            weather: config
                .weathertime_enabled
                .then(|| weathertime::WeatherPanel::new(&config.timezones, background, enrichment.http.clone())),
            status: config.status_enabled.then(|| status::StatusPanel::new(background)),
            music_view: music::MusicView::default(),
        }
    }

    pub fn show(&mut self, context: &mut UiContext, music: &mut Music) {
        let status_width = self.status.as_ref().map_or(0.0, |status| status.width() + GAP);
        let reserved = context.config.history_width
            + GAP
            + f32::from(context.config.weathertime_enabled) * (weathertime::WIDTH + GAP)
            + status_width;
        let px_per_ms =
            (context.frame.screen_size.x - reserved).max(84.0) / (context.config.timeline_future_minutes * 60_000.0);
        let layout = BarLayout {
            playhead_x: context.config.history_width + context.config.timeline_past_minutes * 60_000.0 * px_per_ms,
            px_per_ms,
        };
        if context.config.lyrics_enabled {
            lyrics::show(context, music, layout);
        }
        let sky = self
            .weather
            .as_mut()
            .map_or_else(weathertime::StatusSky::default, |weather| weather.show(context, status_width));
        if let Some(status) = self.status.as_mut() {
            status.show(context, sky);
        }
        self.music_view.show(context, music, layout);
    }
}
