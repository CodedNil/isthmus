#![cfg_attr(target_arch = "spirv", no_std)]
#![feature(trait_alias)]
#![cfg_attr(not(target_arch = "spirv"), feature(const_cmp, const_trait_impl))]

extern crate self as isthmus;

pub use data::{Buffer, F16x2, ShaderData, Unorm8x4, Unorm16x2};
pub use geometry::{Quad, Rect, Triangle};
pub use glam;
pub use image::{Image, Sampling};
pub use isthmus_macros::{ShaderData, program, shader};
pub use program::{Blend, Program};
pub use resources::ResourceData;
pub use shader::{
    Flat, Fragment, Paint, Primitive, ShaderFrame, Smooth, Surface, Vertex, VertexInput, Vertices, raster, source_over,
    surface, vertices,
};
pub use spirv_std;
use spirv_std::num_traits;
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
        Blend, Buffer, F16x2, Flat, Float, Fragment, Image, Paint, Primitive, Quad, Rect, Sampling, ShaderData,
        ShaderFrame, Smooth, Triangle, Unorm8x4, Unorm16x2, Vertex, VertexInput, program, raster, shader, source_over,
        surface, vertices,
    };
    pub use glam::{
        IVec2, IVec3, IVec4, Mat2, Mat3, Mat4, Quat, UVec2, UVec3, UVec4, Vec2, Vec3, Vec4, ivec2, ivec3, ivec4, uvec2,
        uvec3, uvec4, vec2, vec3, vec4,
    };
    pub use spirv_std::arch::Derivative;
}

#[doc(hidden)]
pub mod __private {
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::program::{ShaderEntry, shader_index};
    pub use crate::{data::FrameData, image::ShaderImage};
}
