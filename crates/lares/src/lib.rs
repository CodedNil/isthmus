#![cfg_attr(target_arch = "wasm32", expect(clippy::future_not_send, reason = "browser futures run on one thread"))]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "scene dispatch names the few kinds and materials it treats specially"
)]

pub use app::camera;
#[cfg(not(target_arch = "wasm32"))]
pub use app::server;
#[cfg(not(target_arch = "wasm32"))]
pub use app::views;
#[cfg(not(target_arch = "wasm32"))]
pub use irradiance::bake;

pub mod app;
pub mod home;
pub(crate) mod irradiance;
pub mod render;
pub(crate) mod scene;

pub mod presence;

/// Logs at info level to stderr, for the server and the command line tools.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_logging() {
    tracing_subscriber::fmt().with_target(false).with_max_level(tracing::Level::INFO).init();
}
