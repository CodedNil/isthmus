use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_arch = "wasm32")]
pub mod web;

/// One launchable application entry exposed to the launcher.
pub struct DesktopApp {
    pub name: String,
    pub exec: String,
    pub comment: String,
    pub icon_path: Option<PathBuf>,
    pub action: Option<(String, String)>,
    pub icon: Option<isthmus::Image>,
}

/// Platform services used by the shared Cantus UI.
///
/// The selected platform module adds the inherent methods. A concrete service
/// namespace keeps call sites as simple as `Platform::run()` without exposing
/// a trait or requiring fully-qualified calls for associated functions.
pub struct Platform;
