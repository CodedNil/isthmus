//! Immediate-mode drawing with Rust shaders: write shader code inline, run it on the GPU.
//!
//! This is the bridge between host and GPU, with no drawing vocabulary of its own: the word codec, the
//! stage contract, the build-time program/shader macros and the Vulkan renderer. Shapes, text and painting
//! live in `isthmus_2d`. Shader-side modules (`codec`, `stage`, `frame`, `gpu`, `image`) compile for SPIR-V as well
//! as the host; the `host` that records and presents draws is host-only.
#![cfg_attr(target_arch = "spirv", no_std, feature(asm_experimental_arch))]
#![cfg_attr(not(target_arch = "spirv"), feature(const_trait_impl))]
#![feature(trait_alias)]

extern crate self as isthmus;

#[cfg(not(target_arch = "spirv"))]
pub use ash::vk;
pub use codec::{F16x2, InlineVec, Sent, ShaderData, SharedWords, Unorm8x4, Unorm16x2};
pub use frame::{FrameState, ShaderFrame};
#[cfg(all(not(target_arch = "spirv"), feature = "wayland"))]
pub use host::WaylandWindow;
#[cfg(not(target_arch = "spirv"))]
pub use host::{
    Arena, Frame, Record, Render, RenderError, Renderer, SetupError, Shader, Shaped, SurfaceHandle, TextureFormat,
    Upload, Window,
};
pub use image::Image;
pub use isthmus_macros::{ShaderData, program};
pub use math::MinMax;
pub use program::{Blend, Program};
#[cfg(not(target_arch = "spirv"))]
pub use raw_window_handle as rwh;
use spirv_std::num_traits;
pub use spirv_std::{self, glam};
pub use stage::{Fragment, Primitive};

mod codec;
mod frame;
#[cfg(target_arch = "spirv")]
mod gpu;
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
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::TextureFormat;
    pub use crate::{
        Blend, F16x2, Float, Fragment, Image, InlineVec, MinMax, Primitive, ShaderData, ShaderFrame, Unorm8x4,
        Unorm16x2, glam::prelude::*, program,
    };
    pub use spirv_std::arch::Derivative;
}

#[doc(hidden)]
pub mod __private {
    #[cfg(target_arch = "spirv")]
    pub use crate::gpu::{ShaderSlice, fragment, fragment_input, vertex};
    #[cfg(not(target_arch = "spirv"))]
    pub use crate::program::ShaderEntry;
}
