#![feature(default_field_values, trait_alias)]
#![cfg_attr(target_arch = "wasm32", expect(clippy::future_not_send, reason = "browser futures run on one thread"))]

pub use app::run;
#[cfg(target_os = "linux")]
pub use config::generate_nix_options;

mod app;
mod catalog;
pub(crate) mod config;
pub(crate) mod interaction;
pub(crate) mod music;
pub mod platform;
pub(crate) mod render;
pub(crate) mod timer;
