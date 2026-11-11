//! Antialiased evaluation of a distance field at a fragment.
use crate::Paint;
use core::ops::Range;
use isthmus::{
    Float,
    glam::{Vec2, Vec4, vec2},
    spirv_std::arch::Derivative,
};

/// A distance evaluation with cached derivatives and primitive coverage.
#[derive(Clone, Copy)]
pub struct Sample {
    /// Signed distance after any local deformation.
    pub distance: f32,
    /// Distance derivatives along framebuffer x and y.
    pub gradient: Vec2,
    /// Antialiased coverage of the primitive's selected distance region.
    pub coverage: f32,
}

impl Paint for Sample {
    fn paint(self, fill: Vec4, outline: Vec4) -> Vec4 {
        let foreground = fill.w * self.fill();
        let background = outline.w * (self.coverage - foreground).max(0.0);
        let alpha = foreground + background;
        ((fill.truncate() * foreground + outline.truncate() * background) / alpha.max(f32::MIN_POSITIVE)).extend(alpha)
    }
}

impl Sample {
    /// Resolves a distance and its declared outline with antialiased coverage.
    pub fn new(distance: f32, outline: f32) -> Self {
        let mut sample = Self { distance, gradient: vec2(distance.dfdx(), distance.dfdy()), coverage: 0.0 };
        sample.coverage = sample.below(outline.max(0.0));
        sample
    }

    fn below(self, edge: f32) -> f32 {
        (0.5 + (edge - self.distance) / self.gradient.abs().element_sum().max(f32::MIN_POSITIVE)).saturate()
    }

    /// Antialiased coverage inside the original contour.
    pub fn fill(self) -> f32 {
        self.below(0.0)
    }

    /// Antialiased coverage of an interval in the original distance field.
    pub fn band(self, range: Range<f32>) -> f32 {
        (self.below(range.end) - self.below(range.start)).max(0.0)
    }
}
