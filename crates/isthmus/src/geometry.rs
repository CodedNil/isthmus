use crate::{Fragment, Primitive, Program, ShaderData, Vertex, VertexInput};
use glam::{Vec2, vec2};

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

    /// Tests membership, including the boundary.
    pub fn contains(self, point: Vec2) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
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
    type Outputs = ();
    type Sample = ();

    fn vertex_count(self) -> u32 {
        if self.is_empty() { 0 } else { 4 }
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let uv = corner(input.index);
        input.project(self.point(uv), uv)
    }

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }
}

/// An oriented rectangle in logical screen coordinates.
#[derive(Clone, Copy, ShaderData)]
pub struct Quad {
    /// Center in logical pixels.
    pub center: Vec2,
    /// Full width and height along the local axes.
    pub size: Vec2,
    /// Unit vector pointing along the local x-axis.
    pub axis: Vec2,
}

impl Quad {
    /// Creates a rectangle with the given unit x-axis.
    pub const fn new(center: Vec2, size: Vec2, axis: Vec2) -> Self {
        Self { center, size, axis }
    }

    /// Creates an oriented quad, falling back to the x-axis for a zero direction.
    pub fn oriented(center: Vec2, size: Vec2, direction: Vec2) -> Self {
        let length = direction.length();
        let axis = if length > 0.0 && length <= f32::MAX { direction / length } else { Vec2::X };
        Self::new(center, size, axis)
    }

    /// Converts a screen position to coordinates relative to the rectangle's center and axes.
    pub fn local(self, pixel: Vec2) -> Vec2 {
        let offset = pixel - self.center;
        vec2(offset.dot(self.axis), offset.dot(self.axis.perp()))
    }

    /// Maps a screen position into this quad's normalized coordinates, without clamping.
    pub fn uv(self, pixel: Vec2) -> Vec2 {
        self.local(pixel) / self.size + 0.5
    }

    /// Converts centered local coordinates to a logical screen position.
    pub fn point(self, local: Vec2) -> Vec2 {
        self.center + self.axis * local.x + self.axis.perp() * local.y
    }

    /// Tests membership, including points on the boundary.
    pub fn contains(self, point: Vec2) -> bool {
        self.local(point).abs().cmple(self.size * 0.5).all()
    }

    /// Moves each edge outward by `amount` logical pixels.
    pub fn expanded(mut self, amount: f32) -> Self {
        self.size += amount * 2.0;
        self
    }
}

/// Creates an axis-aligned rectangle centered at the origin from its full size.
impl From<Vec2> for Quad {
    fn from(size: Vec2) -> Self {
        Rect::from(size).into()
    }
}

impl<P: Program> Primitive<P> for Quad {
    type Outputs = ();
    type Sample = ();

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }

    fn vertex_count(self) -> u32 {
        4
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let uv = corner(input.index);
        input.project(self.point(self.size * (uv - 0.5)), uv)
    }
}

/// A triangle defined by three logical screen positions.
#[derive(Clone, Copy, ShaderData)]
pub struct Triangle {
    /// First vertex.
    pub a: Vec2,
    /// Second vertex.
    pub b: Vec2,
    /// Third vertex.
    pub c: Vec2,
}

impl Triangle {
    /// Creates a triangle from its three vertices.
    pub const fn new(a: Vec2, b: Vec2, c: Vec2) -> Self {
        Self { a, b, c }
    }

    /// Creates an isosceles triangle pointing along `direction`, or the x-axis if zero.
    pub fn oriented(center: Vec2, size: Vec2, direction: Vec2) -> Self {
        let axis = Quad::oriented(center, size, direction).axis;
        let along = axis * size.x * 0.5;
        let across = axis.perp() * size.y * 0.5;
        Self::new(center + along, center - along + across, center - along - across)
    }
}

impl<P: Program> Primitive<P> for Triangle {
    type Outputs = ();
    type Sample = ();

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }

    fn vertex_count(self) -> u32 {
        3
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let (pixel, uv) = match input.index {
            0 => (self.a, Vec2::ZERO),
            1 => (self.b, vec2(1.0, 0.0)),
            _ => (self.c, vec2(0.0, 1.0)),
        };
        input.project(pixel, uv)
    }
}

fn corner(index: u32) -> Vec2 {
    vec2((index & 1) as f32, (index >> 1) as f32)
}

impl<P: Program, const N: usize> Primitive<P> for [Vec2; N] {
    type Outputs = ();
    type Sample = ();

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }

    fn vertex_count(self) -> u32 {
        const {
            assert!(N <= u32::MAX as usize, "geometry vertex count exceeds u32");
        }
        N as u32
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let pixel = if (input.index as usize) < N { self[input.index as usize] } else { Vec2::ZERO };
        input.project(pixel, pixel)
    }
}
