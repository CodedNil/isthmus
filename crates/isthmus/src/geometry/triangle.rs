use super::{GeometrySample, Raster, text::TextResources};
#[cfg(not(target_arch = "spirv"))]
use core::iter::once;
use glam::{Vec2, Vec3, vec2, vec3};

/// A triangle defined by three logical screen positions.
#[derive(Clone, Copy, crate::ShaderData)]
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
        let axis = direction.normalize_or(Vec2::X);
        let along = axis * size.x * 0.5;
        let across = axis.perp() * size.y * 0.5;
        Self::new(center + along, center - along + across, center - along - across)
    }

    /// Returns the vertices in rasterization order.
    pub const fn data(self) -> [Vec2; 3] {
        [self.a, self.b, self.c]
    }

    /// Returns vertex weights for a screen position; the triangle must have nonzero area.
    pub fn barycentric(self, pixel: Vec2) -> Vec3 {
        let ab = self.b - self.a;
        let ac = self.c - self.a;
        let ap = pixel - self.a;
        let area = ab.perp_dot(ac);
        let b = ap.perp_dot(ac) / area;
        let c = ab.perp_dot(ap) / area;
        vec3(1.0 - b - c, b, c)
    }
}

/// Interpolation coordinates within a triangle fragment.
#[derive(Clone, Copy)]
pub struct TriangleSample {
    /// Barycentric weights of the second and third vertices.
    pub uv: Vec2,
    /// Barycentric weights of all three vertices.
    pub barycentric: Vec3,
}

impl GeometrySample<'_> for TriangleSample {
    type Payload = ();
    type Raster = Triangle;

    fn sample(pixel: Vec2, raster: [Vec2; 3], (): (), _: TextResources<'_>) -> Self {
        let barycentric = Triangle::from_data(raster).barycentric(pixel);
        Self { uv: vec2(barycentric.y, barycentric.z), barycentric }
    }
}

impl Raster for Triangle {
    const VERTICES: u32 = 3;

    fn from_data([a, b, c]: [Vec2; 3]) -> Self {
        Self { a, b, c }
    }

    fn vertex(self, vertex: u32) -> Vec2 {
        match vertex {
            0 => self.a,
            1 => self.b,
            _ => self.c,
        }
    }
}

#[cfg(not(target_arch = "spirv"))]
impl super::Geometry for Triangle {
    type Context = ();
    type Sample = TriangleSample;

    fn payload(self) {}

    fn primitives(self, (): &()) -> impl Iterator<Item = [Vec2; 3]> {
        once(self.data())
    }
}
