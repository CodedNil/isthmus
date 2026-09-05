use crate::{
    glam::{FloatExt, Vec2, Vec4, vec2},
    spirv_std::arch::Derivative,
};
use core::ops::{Add, Neg, Sub};

/// Composable negative-inside signed-distance geometry.
#[derive(Clone, Copy)]
#[must_use]
pub struct Sdf {
    pub distance: f32,
}

impl Sdf {
    pub fn rounded_box(point: Vec2, half_size: Vec2, radius: f32) -> Self {
        let corner = point.abs() - half_size + radius;
        Self::new(corner.max(Vec2::ZERO).length() + corner.x.max(corner.y).min(0.0) - radius)
    }

    pub fn capsule(point: Vec2, half_span: f32, radius: f32) -> Self {
        Self::new((point - vec2(point.x.clamp(-half_span, half_span), 0.0)).length() - radius)
    }

    pub fn star(point: Vec2, radius: f32, indent: f32) -> Self {
        let k1 = vec2(0.809_017, -0.587_785_25);
        let k2 = vec2(-k1.x, k1.y);
        let mut point = vec2(point.x.abs(), -point.y);
        point -= 2.0 * k1.dot(point).max(0.0) * k1;
        point -= 2.0 * k2.dot(point).max(0.0) * k2;
        point.x = point.x.abs();
        point.y -= radius;
        let edge = indent * vec2(-k1.y, k1.x) - vec2(0.0, radius);
        let edge_t = (point.dot(edge) / edge.length_squared()).saturate();
        let cross = point.y * edge.x - point.x * edge.y;
        Self::new((point - edge * edge_t).length() * if cross < 0.0 { -1.0 } else { 1.0 })
    }

    pub fn rounded_triangle(point: Vec2, side_len: f32, radius: f32) -> Self {
        let k = 1.732_050_8;
        let mut point = vec2(point.x.abs(), point.y);
        let h = (point.x + k * point.y).max(0.0);
        point -= 0.5 * vec2(h, h * k);
        point -= vec2(
            point.x.clamp(-0.5 * (side_len - radius) * k, 0.5 * (side_len - radius) * k),
            -0.5 * (side_len - radius),
        );
        Self::new(point.length() * if point.y > 0.0 { -1.0 } else { 1.0 } - radius)
    }

    /// Shortest distance from `point` to the line segment between `start` and `end`.
    pub fn segment(point: Vec2, start: Vec2, end: Vec2) -> Self {
        let segment = end - start;
        let along = ((point - start).dot(segment) / segment.length_squared().max(0.001)).saturate();
        Self::new((point - start - segment * along).length())
    }

    /// "‹" chevron with its tip at the origin, spanning to `extent` and its mirror; negate `extent.x` for a "›".
    pub fn chevron(point: Vec2, extent: Vec2) -> Self {
        Self::segment(point, Vec2::ZERO, extent).union(Self::segment(point, Vec2::ZERO, vec2(extent.x, -extent.y)))
    }

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
            // Keep antialiasing local because derivatives spike at primitive boundaries.
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
