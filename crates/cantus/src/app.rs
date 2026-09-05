use crate::{
    config::{self, Config},
    interaction::Interaction,
    music::{Enrichment, Music},
    platform::{Platform, Task},
    render::{Bar, Frame, UiContext, launcher::LauncherState},
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

pub type Update<T> = Box<dyn FnOnce(&mut T) + Send>;
pub type AppUpdater = Sender<Update<CantusApp>>;

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

    pub(crate) fn spawn_update(&self, task: impl Task<Output = Option<Update<CantusApp>>>) {
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
    pub(crate) app_updates: mpsc::Receiver<Update<Self>>,
    pub(crate) config: Config,
    pub(crate) enrichment: Enrichment,
    pub(crate) interaction: Interaction,
    occluded_interaction: Interaction,
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
            occluded_interaction: Interaction::default(),
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
            self.refresh_enrichment(true);
        }
    }

    pub(crate) fn draw(&mut self, frame: Frame<'_>, bar: bool, launcher: bool) {
        let interaction =
            if self.launcher.open && !launcher { &mut self.occluded_interaction } else { &mut self.interaction };
        let mut context = UiContext::new(frame, &self.config, interaction);
        if bar {
            self.bar.show(&mut context, &mut self.music);
        }
        if launcher {
            self.launcher.show(&mut context, !bar);
        }
        context.finish();
    }
}

pub fn update(work: impl FnOnce(&mut CantusApp) + Send + 'static) -> Update<CantusApp> {
    Box::new(work)
}

pub fn send_update(sender: &AppUpdater, work: impl FnOnce(&mut CantusApp) + Send + 'static) -> bool {
    sender.send(update(work)).is_ok()
}
