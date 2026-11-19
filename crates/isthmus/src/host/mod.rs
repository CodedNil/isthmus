//! The wgpu host: renderer, device state and presentation surfaces. Never compiled for shaders.
pub use arena::Arena;
pub use error::{RenderError, SetupError};
pub use frame::Frame;
pub use renderer::{Render, Renderer};
pub use shader::{Record, Shader, Shaped, Upload};
pub use surface::SurfaceHandle;

mod arena;
mod buffer;
mod error;
mod frame;
mod gpu;
mod images;
mod pipeline_cache;
mod pipelines;
mod renderer;
mod setup;
mod shader;
mod surface;
#[cfg(test)]
mod tests;
