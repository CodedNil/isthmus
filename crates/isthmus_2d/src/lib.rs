#![cfg_attr(target_arch = "spirv", no_std)]

pub use motion::Ramp;
pub use shape::{Corner, Sample, Sdf, Shape};
pub use text::Text;

#[cfg(not(target_arch = "spirv"))]
pub mod layout;
mod motion;
pub mod prelude;
mod shape;
pub mod text;
