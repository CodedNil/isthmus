//! Images: pixel data on the host, a bound texture and sampler inside shaders.
use crate::glam::Vec4;
#[cfg(not(target_arch = "spirv"))]
pub use host::Image;
#[cfg(target_arch = "spirv")]
pub use sampled::Image;

#[cfg(not(target_arch = "spirv"))]
mod host;
#[cfg(target_arch = "spirv")]
mod sampled;

/// Undoes alpha premultiplication.
fn straight(color: Vec4) -> Vec4 {
    (color.truncate() / color.w.max(f32::MIN_POSITIVE)).extend(color.w)
}
