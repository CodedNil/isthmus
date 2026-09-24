pub mod app;
pub mod camera;
pub mod home;
pub mod render;

#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(target_arch = "wasm32")]
pub mod web;
