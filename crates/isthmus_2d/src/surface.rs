//! Bounded fragment samplers, rasterized with an antialiasing margin.
use crate::Rect;
use isthmus::{Fragment, Primitive, glam::Vec2};

/// A bounded fragment sampler rasterized with an antialiasing margin.
#[derive(Clone, Copy)]
pub struct Surface<F> {
    bounds: Rect,
    sample: F,
}

/// Pairs conservative bounds with a shader returning prepared data and coverage.
pub const fn surface<F>(bounds: Rect, sample: F) -> Surface<F> {
    Surface { bounds, sample }
}

impl<T, F: FnOnce(Vec2) -> (T, f32) + Copy> Primitive for Surface<F> {
    type Sample = T;

    fn region(self, pixel_size: f32) -> Option<[Vec2; 4]> {
        self.bounds.padded_region(pixel_size)
    }

    fn sample(self, fragment: Fragment) -> (T, f32) {
        (self.sample)(fragment.pixel)
    }
}
