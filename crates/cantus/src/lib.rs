#![feature(default_field_values)]

pub use app::run;
pub use config::generate_nix_options;

mod app;
mod catalog;
pub(crate) mod config;
pub(crate) mod interaction;
pub(crate) mod music;
pub mod platform;
pub(crate) mod render;
pub(crate) mod timer;
