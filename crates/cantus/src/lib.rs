#![feature(default_field_values, trait_alias)]

pub use app::run;
#[cfg(target_os = "linux")]
pub use config::generate_nix_options;

mod app;
pub(crate) mod config;
pub(crate) mod interaction;
pub(crate) mod music;
pub mod platform;
pub(crate) mod render;
