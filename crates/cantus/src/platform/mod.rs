pub use catalog::{DesktopApp, decode_icon, load_desktop_icons, start_app_catalog};
pub use desktop::{
    PowerAction, current_location, open_url, run_power_action, spawn, start_launcher_listener, trigger_launcher,
    try_open_url,
};
use std::thread;
pub use system::{STATUS_SAMPLE_INTERVAL, monitor_audio, set_volume, start_status_monitor};
pub use wayland::run;

mod catalog;
mod desktop;
pub(crate) mod mpris;
mod system;
mod wayland;

fn spawn_thread(name: &'static str, job: impl FnOnce() + Send + 'static) {
    thread::Builder::new().name(name.into()).spawn(job).expect("failed to spawn background thread");
}
