#![cfg_attr(target_arch = "spirv", no_std, feature(asm_experimental_arch))]
#![cfg_attr(not(target_arch = "spirv"), feature(const_cmp, const_trait_impl))]
#![feature(trait_alias)]

extern crate self as isthmus;

pub use data::{F16x2, InlineVec, ShaderData, Unorm8x4, Unorm16x2};
pub use geometry::{Quad, Rect};
pub use image::Image;
pub use isthmus_macros::{ShaderData, program, shader};
pub use math::MinMax;
pub use program::{Blend, Program};
pub use resources::ResourceData;
pub use shader::{Fragment, Paint, Primitive, ShaderFrame, Surface, Vertex, VertexInput, raster, source_over, surface};
use spirv_std::num_traits;
pub use spirv_std::{self, glam};
#[cfg(not(target_arch = "spirv"))]
pub use {
    backend::{
        renderer::{Render, RenderError, Renderer},
        setup::SetupError,
        surface::SurfaceHandle,
    },
    frame::Frame,
    resources::Resources,
    wgpu,
};

#[cfg(not(target_arch = "spirv"))]
mod bindings;
mod data;
#[cfg(not(target_arch = "spirv"))]
mod frame;
mod geometry;
mod image;
mod math;
mod program;
mod resources;
mod shader;

#[cfg(not(target_arch = "spirv"))]
mod backend;

/// Floating-point math and interpolation available on both host and shader targets.
pub trait Float = glam::FloatExt + num_traits::Float;

/// Common shader types and operations, intended for `use isthmus::prelude::*`.
pub mod prelude {
    pub use crate::{
        Blend, F16x2, Float, Fragment, Image, InlineVec, MinMax, Paint, Primitive, Quad, Rect, ShaderData, ShaderFrame,
        Unorm8x4, Unorm16x2, Vertex, VertexInput, glam::prelude::*, program, raster, shader, source_over, surface,
    };
    pub use spirv_std::arch::Derivative;
}

#[doc(hidden)]
pub mod __private {
    pub use crate::data::FrameData;
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::program::{ShaderEntry, shader_index};
}
