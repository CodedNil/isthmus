#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "scene dispatch names the few kinds and materials it treats specially"
)]

pub use app::{camera, run, views};
pub use irradiance::bake;

pub mod app;
pub mod home;
pub(crate) mod irradiance;
pub mod render;
pub(crate) mod scene;

pub mod presence;

/// Logs at info level to stderr for the app and command line tools.
pub fn init_logging() {
    tracing_subscriber::fmt().with_target(false).with_max_level(tracing::Level::INFO).init();
}
