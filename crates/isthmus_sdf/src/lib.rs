//! Bounded distance fields and analytic Bézier text primitives for Isthmus.
#![cfg_attr(target_arch = "spirv", no_std)]
#![warn(missing_docs)]

#[cfg(not(target_arch = "spirv"))]
pub mod layout;
mod shape;
pub mod text;

pub use shape::{Outlined, Sample, Shape};
pub use text::Text;
