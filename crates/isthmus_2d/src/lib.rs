//! Two-dimensional drawing on top of `isthmus`: rectangles, signed-distance shapes, text and painting.
#![cfg_attr(target_arch = "spirv", no_std)]

pub use geometry::{Quad, Rect};
pub use motion::Ramp;
pub use paint::{FragmentExt, Paint, source_over};
pub use shape::{Corner, Sample, Sdf, Shape};
pub use surface::{Surface, surface};
#[cfg(not(target_arch = "spirv"))]
pub use text::{Line, install_fonts};
pub use text::{Text, TextRecord};

mod geometry;
mod motion;
mod paint;
pub mod prelude;
mod shape;
mod surface;
pub mod text;
