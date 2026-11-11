//! Two-dimensional drawing on top of `isthmus`: rectangles, signed-distance shapes, text and painting.
#![cfg_attr(target_arch = "spirv", no_std)]

pub use geometry::{Quad, Rect};
pub use motion::Ramp;
pub use paint::{FragmentExt, Paint, source_over};
pub use raster::{Surface, raster, surface};
pub use shape::{Corner, Sample, Sdf, Shape};
pub use text::Text;

mod geometry;
mod motion;
mod paint;
pub mod prelude;
mod raster;
mod shape;
pub mod text;
