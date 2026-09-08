use crate::{Fragment, Primitive, Program, ShaderData, Vertex, VertexInput};
use glam::{Vec2, vec2};

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

    /// Creates an axis-aligned rectangle from its minimum and maximum corners.
    pub fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self::new(min.midpoint(max), max - min, Vec2::X)
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

    /// Returns the minimum and maximum corners of the enclosing axis-aligned rectangle.
    pub fn extents(self) -> (Vec2, Vec2) {
        let half_size = (self.axis.abs() * self.size.x + self.axis.perp().abs() * self.size.y) * 0.5;
        (self.center - half_size, self.center + half_size)
    }

    #[must_use]
    /// Moves each edge outward by `amount` logical pixels.
    pub fn expanded(mut self, amount: f32) -> Self {
        self.size += amount * 2.0;
        self
    }
}

/// Creates an axis-aligned rectangle centered at the origin from its full size.
impl From<Vec2> for Quad {
    fn from(size: Vec2) -> Self {
        Self::new(Vec2::ZERO, size, Vec2::X)
    }
}

impl<P: Program> Primitive<P> for Quad {
    type Outputs = ();
    type Sample = ();

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }

    fn vertex_count(self, _: f32) -> u32 {
        4
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let uv = vec2((input.index & 1) as f32, (input.index >> 1) as f32);
        input.project(self.point(self.size * (uv - 0.5)), uv)
    }
}
