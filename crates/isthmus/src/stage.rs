//! The contract between a shader's primitive and its fragment closure.
use crate::glam::Vec2;
use core::ops::Deref;

/// Geometry rasterized by a shader, and how each pixel it covers is sampled.
pub trait Primitive: Copy {
    /// Prepared input supplied to the fragment closure.
    type Sample;
    /// Corners of the screen region to rasterize in triangle-strip order (min, +x, +y, max), or `None` to draw
    /// nothing. `pixel_size` is the logical size of a framebuffer pixel, for antialiasing margins.
    /// Evaluated on the host; the GPU only ever samples.
    fn region(self, pixel_size: f32) -> Option<[Vec2; 4]>;
    /// Prepares shading data and antialiased coverage before the fragment closure runs.
    fn sample(self, fragment: Fragment) -> (Self::Sample, f32);
}

/// Rasterized fragment coordinates.
#[derive(Clone, Copy)]
pub struct Fragment<T = ()> {
    /// Actual screen position in logical pixels.
    pub pixel: Vec2,
    /// Coordinates across the region, from 0 at its min corner to 1 at its max corner.
    pub uv: Vec2,
    /// Primitive-specific data, also accessible through dereferencing this fragment.
    pub sample: T,
    /// Coverage applied by the renderer after fragment evaluation.
    pub coverage: f32,
}

impl<T> Fragment<T> {
    #[doc(hidden)]
    pub const fn sampled<U>(&self, sample: U, coverage: f32) -> Fragment<U> {
        Fragment { pixel: self.pixel, uv: self.uv, sample, coverage }
    }
}

impl<T> Deref for Fragment<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.sample
    }
}
