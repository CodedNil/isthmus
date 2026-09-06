//! Typed Rust shaders, geometry, and rendering shared between CPU and GPU.
#![cfg_attr(target_arch = "spirv", no_std)]
#![warn(missing_docs)]
#![feature(trait_alias)]

extern crate self as isthmus;

use spirv_std::num_traits;

#[cfg(not(target_arch = "spirv"))]
mod bindings;
mod data;
#[cfg(not(target_arch = "spirv"))]
mod frame;
/// Raster geometry, composable distance fields, and vector text.
pub mod geometry;
mod image;
mod program;

#[cfg(not(target_arch = "spirv"))]
mod backend;

#[cfg(not(target_arch = "spirv"))]
pub use backend::{
    renderer::{Render, RenderError, Renderer},
    setup::SetupError,
    surface::SurfaceHandle,
};
pub use data::{Buffer, ColorExt, F16x2, ShaderData, Unorm8x4, Unorm16x2};
#[cfg(not(target_arch = "spirv"))]
pub use frame::Frame;
#[cfg(not(target_arch = "spirv"))]
pub use geometry::Geometry;
pub use geometry::{
    Fragment, Quad, Triangle, TriangleFragment,
    sdf::{Sdf, SdfSample},
    text::TextFragment,
};
pub use glam;
pub use image::{Image, Sampling};
pub use isthmus_macros::{ShaderData, program, shader};
pub use program::{Blend, Program};
pub use spirv_std;

/// Floating-point math and interpolation available on both host and shader targets.
pub trait Float = glam::FloatExt + num_traits::Float;

#[doc(hidden)]
pub mod __private {
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::backend::gpu::Gpu;
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::program::{ShaderEntry, ShaderSpec, shader_index};
    pub use crate::{
        data::{FrameData, load_unchecked},
        geometry::{DrawRecord, ShaderInput},
        image::ShaderImage,
    };
}
