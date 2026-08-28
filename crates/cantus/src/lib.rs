mod app;
pub(crate) mod config;
pub(crate) mod interaction;
pub(crate) mod music;
pub(crate) mod platform;
pub(crate) mod render;
mod time;

pub use app::run;
pub use platform::Platform;
