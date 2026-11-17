use crate::{
    config::{self, Config},
    interaction::Interaction,
    music::Music,
    platform::{self, Task},
    render::{Bar, Globals, Program, RIPPLE_COUNT, RipplePulse, UiContext, launcher::LauncherState},
    timer::Timer,
};
#[cfg(target_os = "linux")]
use calloop::channel::Sender;
use isthmus::{Render, SurfaceHandle, glam::Vec2};
use isthmus_2d::Ramp;
use jiff::ZonedDateTime;
use reqwest::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
#[cfg(target_arch = "wasm32")]
use std::sync::mpsc::Sender;
use std::{env, fmt::Result as FmtResult, io, time::Duration};
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::{
    Layer as _,
    filter::Targets,
    fmt::{self, format::Writer},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};
use web_time::Instant;

#[derive(Clone, Copy)]
pub struct View {
    pub bar: bool,
    pub launcher: bool,
}

pub type Update = Box<dyn FnOnce(&mut CantusApp) + Send>;
pub type AppUpdater = Sender<Update>;

pub async fn fetch_json<T: DeserializeOwned>(request: RequestBuilder) -> reqwest::Result<T> {
    request.send().await?.error_for_status()?.json().await
}

#[derive(Clone)]
pub struct Background {
    pub(crate) http: Client,
    pub(crate) updater: AppUpdater,
}

fn log_time(writer: &mut Writer<'_>) -> FmtResult {
    write!(writer, "{}", ZonedDateTime::now().strftime("%H:%M:%S%.3f"))
}

pub fn run() {
    // `RUST_LOG=isthmus=debug` shows renderer diagnostics.
    let filter = env::var("RUST_LOG")
        .ok()
        .and_then(|spec| spec.parse::<Targets>().ok())
        .unwrap_or_else(|| Targets::new().with_default(LevelFilter::WARN).with_target("cantus", Level::INFO));
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_target(false)
                .with_timer(log_time as fn(&mut Writer<'_>) -> FmtResult)
                .with_writer(io::stderr)
                .with_filter(filter),
        )
        .init();

    platform::run();
}

impl Background {
    pub(crate) fn spawn_update<F: FnOnce(&mut CantusApp) + Send + 'static>(&self, task: impl Task<Output = Option<F>>) {
        let updater = self.updater.clone();
        platform::spawn_task(async move {
            if let Some(event) = task.await {
                drop(updater.send(Box::new(event)));
            }
        });
    }
}

pub struct CantusApp {
    started: Instant,
    pub(crate) music: Music,
    pub(crate) launcher: LauncherState,
    pub(crate) bar: Bar,
    pub(crate) config: Config,
    pub(crate) background: Background,
    pub(crate) interaction: Interaction,
    /// How hard the pointer presses the glass, eased over time.
    pressure: Ramp,
    ripples: [RipplePulse; RIPPLE_COUNT],
    /// Where the current press started a ripple.
    pressing: Option<Vec2>,
    pub(crate) next_enrichment: Instant,
    pub(crate) timer: Timer,
}

impl CantusApp {
    pub(crate) fn new(updater: AppUpdater) -> Self {
        let http = Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let http = http.timeout(Duration::from_secs(15));
        let background = Background { updater, http: http.build().expect("failed to construct HTTP client") };
        let config = config::load();
        platform::start_launcher_listener(&background.updater);
        Self {
            started: Instant::now(),
            launcher: LauncherState::new(&background, config.search_providers.iter().cloned()),
            bar: Bar::new(&config, &background),
            music: Music::new(&config, &background.updater),
            background,
            interaction: Interaction::default(),
            pressure: Ramp::default(),
            ripples: Default::default(),
            pressing: None,
            next_enrichment: Instant::now(),
            timer: Timer::default(),
            config,
        }
    }

    pub(crate) fn refresh(&mut self) {
        if let Some(request) = self.launcher.pending_timer.take() {
            self.timer.apply(request);
        }
        if Instant::now() >= self.next_enrichment {
            self.next_enrichment = Instant::now() + Duration::from_secs(1);
            self.refresh_enrichment();
        }
    }

    pub(crate) fn draw(
        &mut self,
        render: &mut Render<'_, Program>,
        surface: SurfaceHandle,
        screen_size: Vec2,
        view: View,
    ) {
        // The app owns input animation time, so it survives GPU restarts.
        render.time = self.started.elapsed().as_secs_f32();
        let owns_input = if self.launcher.open { view.launcher } else { view.bar };
        self.interaction.enabled = owns_input;
        let (pressure, ripples) = if owns_input {
            self.interaction.begin_frame();
            // The glass dents under the pointer, more when pressed; each press sends a ripple.
            let target = if self.interaction.pressed() { 2.0 } else { f32::from(self.interaction.hovering()) };
            self.pressure.approach(render.time, target, 5.0);
            let pressing = self.interaction.press_origin();
            if let Some(origin) = pressing.filter(|_| pressing != self.pressing)
                && let Some(ripple) = self.ripples.iter_mut().min_by(|a, b| a.start_time.total_cmp(&b.start_time))
            {
                *ripple = RipplePulse { origin, start_time: render.time };
            }
            self.pressing = pressing;
            (self.pressure.at(render.time), self.ripples)
        } else {
            (0.0, Default::default())
        };
        let globals =
            Globals { pointer: self.interaction.mouse_pos(), pressure, bar_height: self.config.height, ripples };
        render.surface(surface, screen_size, globals, |frame| {
            let mut context = UiContext { frame, config: &self.config, interaction: &mut self.interaction };
            if view.bar {
                self.bar.show(&mut context, &mut self.music, &self.timer);
            }
            if view.launcher {
                self.launcher.show(&mut context);
            }
        });
        if owns_input {
            self.interaction.end_frame();
        }
    }
}

pub fn send_update(sender: &AppUpdater, work: impl FnOnce(&mut CantusApp) + Send + 'static) -> bool {
    sender.send(Box::new(work)).is_ok()
}
