#![cfg_attr(target_arch = "spirv", no_std)]

#[cfg(not(target_arch = "spirv"))]
pub mod layout;
pub mod prelude;
mod shape;
pub mod text;

pub use shape::{Sample, Sdf, Shape};
pub use text::Text;
