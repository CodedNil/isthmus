//! The Vulkan host: renderer, device state and presentation surfaces. Never compiled for shaders.
pub use arena::Arena;
pub use error::{RenderError, SetupError};
pub use frame::Frame;
pub use images::TextureFormat;
pub use renderer::{Render, Renderer};
pub use setup::Window;
pub use shader::{Record, Shader, Shaped, Upload};
pub use surface::SurfaceHandle;
#[cfg(feature = "wayland")]
pub use wayland::WaylandWindow;

mod arena;
mod buffer;
mod error;
mod frame;
mod gpu;
mod heap;
mod images;
mod pipelines;
mod renderer;
mod setup;
mod shader;
mod surface;
#[cfg(test)]
mod tests;
mod timings;

#[cfg(feature = "wayland")]
mod wayland;
