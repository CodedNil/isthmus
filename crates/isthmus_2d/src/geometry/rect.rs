//! Axis-aligned rectangles.
use super::{Quad, corner};
use isthmus::{Fragment, Primitive, Program, ShaderData, Vertex, VertexInput, glam::Vec2};

/// Axis-aligned bounds in logical pixels; inverted corners represent an empty region.
#[derive(Clone, Copy, Default, ShaderData)]
pub struct Rect {
    /// Minimum corner.
    pub min: Vec2,
    /// Maximum corner.
    pub max: Vec2,
}

impl Rect {
    /// Empty bounds with finite coordinates for shader arithmetic.
    pub const EMPTY: Self = Self::new(Vec2::splat(f32::MAX * 0.25), Vec2::splat(-f32::MAX * 0.25));

    /// Reserves the region between two corners without reordering them.
    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// Creates bounds from their center and full size.
    pub fn from_center_size(center: Vec2, size: Vec2) -> Self {
        Self::new(center - size * 0.5, center + size * 0.5)
    }

    /// Center in logical screen coordinates.
    pub fn center(self) -> Vec2 {
        self.min.midpoint(self.max)
    }

    /// Coordinates relative to the center.
    pub fn local(self, point: Vec2) -> Vec2 {
        point - self.center()
    }

    /// Normalized coordinates without clamping.
    pub fn uv(self, point: Vec2) -> Vec2 {
        (point - self.min) / self.size()
    }

    /// Maps normalized coordinates to a logical screen position.
    pub fn point(self, uv: Vec2) -> Vec2 {
        self.min + self.size() * uv
    }

    /// Width and height, negative for inverted bounds.
    pub fn size(self) -> Vec2 {
        self.max - self.min
    }

    /// Whether either dimension is inverted.
    pub fn is_empty(self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    /// Reserves an additional margin around every edge.
    pub fn expanded(self, amount: f32) -> Self {
        Self::new(self.min - amount, self.max + amount)
    }

    /// Encloses both regions.
    pub fn union(self, other: Self) -> Self {
        Self::new(self.min.min(other.min), self.max.max(other.max))
    }

    /// Keeps only the shared region.
    pub fn intersection(self, other: Self) -> Self {
        Self::new(self.min.max(other.min), self.max.min(other.max))
    }
}

impl From<Quad> for Rect {
    fn from(quad: Quad) -> Self {
        let size = quad.axis.abs() * quad.size.x + quad.axis.perp().abs() * quad.size.y;
        Self::from_center_size(quad.center, size)
    }
}

impl From<Rect> for Quad {
    fn from(rect: Rect) -> Self {
        Self::new(rect.center(), rect.size(), Vec2::X)
    }
}

impl From<Vec2> for Rect {
    fn from(size: Vec2) -> Self {
        Self::from_center_size(Vec2::ZERO, size)
    }
}

impl<P: Program> Primitive<P> for Rect {
    type Sample = ();

    fn vertex_count(self) -> u32 {
        if self.is_empty() { 0 } else { 4 }
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let uv = corner(input.index);
        input.project(self.point(uv), uv)
    }

    fn sample(self, _: Fragment) -> ((), f32) {
        ((), 1.0)
    }
}
