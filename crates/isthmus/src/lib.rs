#![cfg_attr(target_arch = "spirv", no_std)]
#![feature(trait_alias)]

extern crate self as isthmus;

use spirv_std::num_traits;

#[cfg(not(target_arch = "spirv"))]
mod bindings;
mod data;
#[cfg(not(target_arch = "spirv"))]
mod frame;
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
pub use data::{ColorExt, ShaderData, Unorm8x4};
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
    pub use crate::{
        data::{PushBlock, load},
        geometry::DrawRecord,
        image::ShaderImage,
    };
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::{
        geometry::ShaderInput,
        program::{ShaderEntry, ShaderSpec, shader_index},
    };
    #[cfg(not(target_arch = "spirv"))]
    pub use bytemuck;
}
