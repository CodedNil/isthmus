//! The 2D vocabulary, intended for `use isthmus_2d::prelude::*` next to `isthmus::prelude::*`.
#[cfg(not(target_arch = "spirv"))]
pub use crate::Line;
pub use crate::{Corner, FragmentExt, Paint, Quad, Ramp, Rect, Sample, Sdf, Shape, Text, raster, source_over, surface};
