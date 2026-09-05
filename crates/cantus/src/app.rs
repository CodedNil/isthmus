use crate::{
    config::{self, Config},
    interaction::Interaction,
    music::{Enrichment, Music},
    platform::{Platform, Task},
    render::{Bar, Frame, Globals, UiContext, launcher::LauncherState},
};
use std::{
    io,
    sync::mpsc::{self, Sender},
    time::Duration,
};
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Handle;
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::{Layer, filter::Targets, fmt, layer::SubscriberExt, util::SubscriberInitExt};
use web_time::Instant;

pub type Update = Box<dyn FnOnce(&mut CantusApp) + Send>;
pub type AppUpdater = Sender<Update>;

#[derive(Clone)]
pub struct Background {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) runtime: Handle,
    pub(crate) updater: AppUpdater,
}

pub fn run() {
    let filter = Targets::new().with_default(LevelFilter::WARN).with_target("cantus", Level::INFO);
    tracing_subscriber::registry().with(fmt::layer().with_writer(io::stderr).with_filter(filter)).init();

    Platform::run();
}

impl Background {
    fn new(updater: &AppUpdater) -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            runtime: Handle::current(),
            updater: updater.clone(),
        }
    }

    pub(crate) fn spawn_update(&self, task: impl Task<Output = Option<Update>>) {
        let updater = self.updater.clone();
        self.spawn(async move {
            if let Some(event) = task.await {
                let _ = updater.send(event);
            }
        });
    }
}

pub struct CantusApp {
    pub(crate) music: Music,
    pub(crate) launcher: LauncherState,
    pub(crate) bar: Bar,
    pub(crate) app_updates: mpsc::Receiver<Update>,
    pub(crate) config: Config,
    pub(crate) enrichment: Enrichment,
    pub(crate) interaction: Interaction,
    next_enrichment: Instant,
}

impl Default for CantusApp {
    fn default() -> Self {
        let (updater, app_updates) = mpsc::channel();
        let background = Background::new(&updater);
        let enrichment = Enrichment::new(background.clone());
        let config = config::load();
        Platform::start_launcher_listener(&background, &updater);
        Self {
            launcher: LauncherState::new(&background, &enrichment.http, config.search_providers.iter().cloned()),
            bar: Bar::new(&config, &background, &enrichment),
            app_updates,
            enrichment,
            music: Music::spotify(&config, &updater, &background),
            interaction: Interaction::default(),
            next_enrichment: Instant::now(),
            config,
        }
    }
}

impl CantusApp {
    pub(crate) fn apply_pending_updates(&mut self) {
        while let Ok(update) = self.app_updates.try_recv() {
            update(self);
        }
        if Instant::now() >= self.next_enrichment {
            self.next_enrichment = Instant::now() + Duration::from_secs(1);
            self.refresh_enrichment();
        }
    }

    pub(crate) fn draw(&mut self, frame: Frame<'_>, bar: bool, launcher: bool) {
        let launcher_open = self.launcher.open;
        let owns_input = if launcher_open { launcher } else { bar };
        if owns_input {
            self.interaction.begin_frame(frame.delta_time, frame.time);
        }
        let mut context = UiContext { frame, config: &self.config, interaction: &mut self.interaction };
        context.interaction.enabled = !self.launcher.open;
        if bar {
            self.bar.show(&mut context, &mut self.music);
        }
        if launcher {
            context.interaction.enabled = true;
            self.launcher.show(&mut context);
        }
        *context.frame.globals = Globals {
            pointer: context.interaction.mouse_pos(),
            pressure: context.interaction.pressure(),
            bar_height: self.config.height,
            ripples: if owns_input { context.interaction.ripples } else { Default::default() },
        };
        context.interaction.enabled = true;
        if owns_input {
            context.interaction.end_frame();
        }
        if launcher_open != self.launcher.open {
            *context.interaction = Interaction::default();
        }
    }
}

pub fn update(work: impl FnOnce(&mut CantusApp) + Send + 'static) -> Update {
    Box::new(work)
}

pub fn send_update(sender: &AppUpdater, work: impl FnOnce(&mut CantusApp) + Send + 'static) -> bool {
    sender.send(update(work)).is_ok()
}
