#![cfg_attr(target_arch = "spirv", no_std)]

extern crate self as isthmus;

mod contract;
mod data;
#[cfg(not(target_arch = "spirv"))]
mod frame;
mod image;
mod sdf;
pub mod text;

#[cfg(not(target_arch = "spirv"))]
mod backend;

#[cfg(not(target_arch = "spirv"))]
pub use backend::{Render, RenderError, Renderer, SetupError};
#[cfg(not(target_arch = "spirv"))]
pub use contract::SurfaceHandle;
pub use contract::{Fragment, Quad};
pub use data::{ShaderData, Unorm8x4};
#[cfg(not(target_arch = "spirv"))]
pub use frame::Frame;
pub use glam;
pub use image::Image;
pub use isthmus_macros::{ShaderData, paint, program};
pub use sdf::{Sdf, SdfSample};
pub use spirv_std::{self, num_traits::Float};
pub use text::TextFragment;

#[doc(hidden)]
pub mod __private {
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::contract::{PaintPipeline, Program, ShaderSpec};
    pub use crate::{
        contract::{DrawRecord, PushBlock, ShaderImage, load},
        data::ImageHandle,
    };
    #[cfg(not(target_arch = "spirv"))]
    pub use bytemuck;
}
