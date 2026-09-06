use super::{GeometrySample, Raster, text::TextResources};
#[cfg(not(target_arch = "spirv"))]
use core::iter::once;

/// An oriented rectangle in logical screen coordinates.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Quad {
    /// Center in logical pixels.
    pub center: glam::Vec2,
    /// Full width and height along the local axes.
    pub size: glam::Vec2,
    /// Unit vector pointing along the local x-axis.
    pub axis: glam::Vec2,
}

impl Quad {
    /// Encodes the center, size, and axis for rasterization.
    pub const fn data(self) -> [glam::Vec2; 3] {
        [self.center, self.size, self.axis]
    }

    /// Creates a rectangle with the given unit x-axis.
    pub const fn new(center: glam::Vec2, size: glam::Vec2, axis: glam::Vec2) -> Self {
        Self { center, size, axis }
    }

    /// Creates an oriented quad, falling back to the x-axis for a zero direction.
    pub fn oriented(center: glam::Vec2, size: glam::Vec2, direction: glam::Vec2) -> Self {
        Self::new(center, size, direction.normalize_or(glam::Vec2::X))
    }

    /// Creates an axis-aligned rectangle from its minimum and maximum corners.
    pub fn from_min_max(min: glam::Vec2, max: glam::Vec2) -> Self {
        Self::new(min.midpoint(max), max - min, glam::Vec2::X)
    }

    /// Converts a screen position to coordinates relative to the rectangle's center and axes.
    pub fn local(self, pixel: glam::Vec2) -> glam::Vec2 {
        let offset = pixel - self.center;
        glam::vec2(offset.dot(self.axis), offset.dot(self.axis.perp()))
    }

    #[must_use]
    /// Moves each edge outward by `amount` logical pixels.
    pub fn expanded(mut self, amount: f32) -> Self {
        self.size += amount * 2.0;
        self
    }
}

/// Local and normalized coordinates within a quad fragment.
#[derive(Clone, Copy)]
pub struct QuadSample {
    /// Position relative to the quad's center and axes, in logical pixels.
    pub local: glam::Vec2,
    /// Coordinates ranging from zero to one across the quad.
    pub uv: glam::Vec2,
}

impl GeometrySample<'_> for QuadSample {
    type Payload = ();
    type Raster = Quad;

    fn sample(pixel: glam::Vec2, raster: [glam::Vec2; 3], (): (), _: TextResources<'_>) -> Self {
        let quad = Quad::from_data(raster);
        let local = quad.local(pixel);
        Self { local, uv: local / quad.size + 0.5 }
    }
}

impl Raster for Quad {
    const VERTICES: u32 = 4;

    fn from_data([center, size, axis]: [glam::Vec2; 3]) -> Self {
        Self { center, size, axis }
    }

    #[inline(never)]
    fn vertex(self, vertex: u32) -> glam::Vec2 {
        let local = (glam::vec2((vertex & 1) as f32, (vertex >> 1) as f32) - 0.5) * self.size;
        self.center + self.axis * local.x + self.axis.perp() * local.y
    }
}

#[cfg(not(target_arch = "spirv"))]
impl<T: Copy + Into<Quad>> super::Geometry for T {
    type Context = ();
    type Sample = QuadSample;

    fn payload(self) {}

    fn primitives(self, (): &()) -> impl Iterator<Item = [glam::Vec2; 3]> {
        once(self.into().data())
    }
}
