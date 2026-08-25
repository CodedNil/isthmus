#![no_std]

#[cfg(not(target_arch = "spirv"))]
extern crate std;

extern crate self as isthmus;

mod contract;
mod data;
mod float;
#[cfg(not(target_arch = "spirv"))]
mod frame;
mod image;
pub mod text;

#[cfg(not(target_arch = "spirv"))]
mod backend;

pub use glam;
pub use spirv_std;

pub use isthmus_macros::{ShaderData, paint, program};

#[cfg(not(target_arch = "spirv"))]
pub use backend::{Render, RenderError, Renderer, SetupError};
#[cfg(not(target_arch = "spirv"))]
pub use contract::SurfaceHandle;
pub use contract::{Fragment, Quad};
pub use data::{ShaderData, Unorm8x4};
pub use float::FloatExt;
#[cfg(not(target_arch = "spirv"))]
pub use frame::Frame;
pub use image::Image;
pub use text::TextFragment;

#[doc(hidden)]
pub mod __private {
    pub use crate::contract::{DrawRecord, ImageHeap, PushBlock, ShaderImage, load};
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::contract::{PaintPipeline, Program, ShaderSpec, shader_module_name};
    pub use crate::data::ImageHandle;
    #[cfg(not(target_arch = "spirv"))]
    pub use bytemuck;
}
