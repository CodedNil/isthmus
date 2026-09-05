#![cfg_attr(target_arch = "spirv", no_std)]
#![feature(trait_alias)]

extern crate self as isthmus;

use spirv_std::num_traits;

#[cfg(not(target_arch = "spirv"))]
mod bindings;
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
pub use backend::{
    renderer::{Render, RenderError, Renderer},
    setup::SetupError,
};
#[cfg(not(target_arch = "spirv"))]
pub use contract::SurfaceHandle;
pub use contract::{Blend, ColorExt, Fragment, Program, Quad, Triangle, TriangleFragment};
pub use data::{ShaderData, Unorm8x4};
#[cfg(not(target_arch = "spirv"))]
pub use frame::{Frame, Geometry};
pub use glam;
pub use image::{Image, Sampling};
pub use isthmus_macros::{ShaderData, program, shader};
pub use sdf::{Sdf, SdfSample};
pub use spirv_std;
pub use text::TextFragment;

/// Floating-point math and interpolation available on both host and shader targets.
pub trait Float = glam::FloatExt + num_traits::Float;

/// Optional shader imports; `shader!` inherits its surrounding Rust scope without injecting this prelude.
pub mod prelude {
    pub use crate::{
        Blend, ColorExt, Float, Fragment, Image, Quad, Sdf, ShaderData, TextFragment, Triangle, TriangleFragment,
        Unorm8x4,
    };
    pub use core::f32::consts::*;
    pub use glam::{Mat2, Mat3, Mat4, UVec2, UVec3, UVec4, Vec2, Vec3, Vec4, vec2, vec3, vec4};
    pub use spirv_std::arch::{Derivative, kill};
}

#[doc(hidden)]
pub mod __private {
    pub use crate::contract::{DrawRecord, PushBlock, ShaderImage, load};
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::contract::{Primitive, ShaderEntry, ShaderInput, ShaderSpec, shader_index};
    #[cfg(not(target_arch = "spirv"))]
    pub use bytemuck;
}
