//! Oriented rectangles.
use super::Rect;
use isthmus::{
    Fragment, Primitive, ShaderData,
    glam::{Vec2, vec2},
};

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

impl Primitive for Quad {
    type Sample = ();

    fn region(self, _: f32) -> Option<[Vec2; 4]> {
        let at = |x: f32, y: f32| self.point(self.size * vec2(x, y));
        Some([at(-0.5, -0.5), at(0.5, -0.5), at(-0.5, 0.5), at(0.5, 0.5)])
    }

    fn sample(self, _: Fragment) -> ((), f32) {
        ((), 1.0)
    }
}
