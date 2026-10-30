#![cfg_attr(target_arch = "wasm32", expect(clippy::future_not_send, reason = "browser futures run on one thread"))]
pub mod app;
pub mod camera;
pub mod home;
pub mod render;

#[cfg(not(target_arch = "wasm32"))]
pub mod bake;
pub mod presence;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(target_arch = "wasm32")]
pub mod web;
