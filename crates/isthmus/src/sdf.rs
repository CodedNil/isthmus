use core::ops::{Add, Neg, Sub};

use crate::{
    glam::{FloatExt, Vec4},
    spirv_std::arch::Derivative,
};

/// Composable negative-inside signed-distance geometry.
#[derive(Clone, Copy)]
#[must_use]
pub struct Sdf {
    pub distance: f32,
}

impl Sdf {
    pub const fn new(distance: f32) -> Self {
        Self { distance }
    }

    pub fn sample(self) -> SdfSample {
        SdfSample::new(self.distance)
    }

    pub fn fill(self) -> f32 {
        self.sample().fill()
    }

    pub fn stroke(self, half_width: f32) -> f32 {
        self.sample().stroke(half_width)
    }

    pub const fn union(self, other: Self) -> Self {
        Self::new(self.distance.min(other.distance))
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self::new(self.distance.max(other.distance))
    }

    pub fn difference(self, other: Self) -> Self {
        self.intersection(-other)
    }

    pub fn lerp(self, other: Self, amount: f32) -> Self {
        Self::new(self.distance.lerp(other.distance, amount))
    }

    pub fn smooth_union(self, other: Self, radius: f32, amount: f32) -> Self {
        let blend = (0.5 + 0.5 * (other.distance - self.distance) / radius).clamp(0.0, 1.0);
        let union = other.distance.lerp(self.distance, blend) - radius * blend * (1.0 - blend);
        Self::new(self.distance.lerp(union, amount))
    }
}

impl Add<f32> for Sdf {
    type Output = Self;
    fn add(self, rhs: f32) -> Self {
        Self::new(self.distance + rhs)
    }
}

impl Sub<f32> for Sdf {
    type Output = Self;
    fn sub(self, rhs: f32) -> Self {
        Self::new(self.distance - rhs)
    }
}

impl Neg for Sdf {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.distance)
    }
}

/// A negative-inside signed distance with derivative-aware antialiasing.
#[must_use]
pub struct SdfSample {
    pub distance: f32,
    pub half_width: f32,
}

impl SdfSample {
    pub fn new(distance: f32) -> Self {
        Self {
            distance,
            // Derivatives can spike at primitive boundaries, so AA must remain
            // local to the actual shape rather than its raster geometry.
            half_width: (distance.fwidth() * 0.5).clamp(0.35, 1.0),
        }
    }

    pub fn fill(&self) -> f32 {
        self.coverage(self.distance)
    }

    pub fn expanded(&self, pixels: f32) -> f32 {
        self.coverage(self.distance - pixels)
    }

    pub fn outline(&self, pixels: f32) -> f32 {
        (self.expanded(pixels) - self.fill()).max(0.0)
    }

    pub fn stroke(&self, half_width: f32) -> f32 {
        self.coverage(self.distance.abs() - half_width)
    }

    /// Composites straight-alpha fill and outline colors.
    pub fn color(&self, fill: Vec4, outline: Vec4, pixels: f32) -> Vec4 {
        let fill_alpha = self.fill() * fill.w;
        let outline_alpha = self.outline(pixels) * outline.w;
        let alpha = fill_alpha + outline_alpha;
        ((fill.truncate() * fill_alpha + outline.truncate() * outline_alpha) / alpha.max(0.0001)).extend(alpha)
    }

    fn coverage(&self, distance: f32) -> f32 {
        distance.smoothstep(self.half_width, -self.half_width)
    }
}
