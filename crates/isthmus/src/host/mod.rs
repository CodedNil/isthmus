//! The wgpu host: renderer, device state and presentation surfaces. Never compiled for shaders.
pub use error::{RenderError, SetupError};
pub use frame::Frame;
pub use renderer::{Render, Renderer};
pub use resources::Resources;
pub use surface::SurfaceHandle;

mod buffer;
mod error;
mod frame;
mod gpu;
mod images;
mod pipelines;
mod renderer;
mod resources;
mod setup;
mod surface;
#[cfg(test)]
mod tests;
