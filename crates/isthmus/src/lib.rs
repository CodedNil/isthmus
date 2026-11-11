//! Immediate-mode drawing with Rust shaders: write shader code inline, run it on the GPU.
//!
//! This is the bridge between host and GPU, with no drawing vocabulary of its own: the word codec, the
//! stage contract, the build-time program/shader macros and the wgpu renderer. Shapes, text and painting
//! live in `isthmus_2d`. Shader-side modules (`codec`, `stage`, `frame`, `image`) compile for SPIR-V as well
//! as the host; the `host` that records and presents draws is host-only.
#![cfg_attr(target_arch = "spirv", no_std, feature(asm_experimental_arch))]
#![cfg_attr(not(target_arch = "spirv"), feature(const_cmp, const_trait_impl))]
#![feature(trait_alias)]
#![cfg_attr(target_arch = "wasm32", expect(clippy::future_not_send, reason = "browser futures run on one thread"))]

extern crate self as isthmus;

pub use codec::{F16x2, InlineVec, ResourceData, ShaderData, Unorm8x4, Unorm16x2};
pub use frame::{FrameState, ShaderFrame};
#[cfg(not(target_arch = "spirv"))]
pub use host::{Frame, Render, RenderError, Renderer, Resources, SetupError, SurfaceHandle};
pub use image::Image;
pub use isthmus_macros::{ShaderData, program, shader};
pub use math::MinMax;
pub use program::{Blend, Program};
use spirv_std::num_traits;
pub use spirv_std::{self, glam};
pub use stage::{Fragment, Primitive, Vertex, VertexInput};
#[cfg(not(target_arch = "spirv"))]
pub use wgpu;

mod codec;
mod frame;
#[cfg(not(target_arch = "spirv"))]
mod host;
mod image;
mod math;
mod program;
mod stage;

/// Floating-point math and interpolation available on both host and shader targets.
pub trait Float = glam::FloatExt + num_traits::Float;

/// Common shader types and operations, intended for `use isthmus::prelude::*`.
pub mod prelude {
    pub use crate::{
        Blend, F16x2, Float, Fragment, Image, InlineVec, MinMax, Primitive, ShaderData, ShaderFrame, Unorm8x4,
        Unorm16x2, Vertex, VertexInput, glam::prelude::*, program, shader,
    };
    pub use spirv_std::arch::Derivative;
}

#[doc(hidden)]
pub mod __private {
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::program::{ShaderEntry, shader_index};
}
