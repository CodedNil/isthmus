use crate::Program;
use glam::{Vec2, Vec3, vec2, vec3};
#[cfg(not(target_arch = "spirv"))]
use {crate::geometry::ShaderInput, core::iter::once};

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Triangle {
    pub a: Vec2,
    pub b: Vec2,
    pub c: Vec2,
}

impl Triangle {
    pub const fn new(a: Vec2, b: Vec2, c: Vec2) -> Self {
        Self { a, b, c }
    }

    pub fn oriented(center: Vec2, size: Vec2, direction: Vec2) -> Self {
        let axis = direction.normalize_or(Vec2::X);
        let along = axis * size.x * 0.5;
        let across = axis.perp() * size.y * 0.5;
        Self::new(center + along, center - along + across, center - along - across)
    }

    pub const fn data(self) -> [Vec2; 3] {
        [self.a, self.b, self.c]
    }

    pub const fn from_data([a, b, c]: [Vec2; 3]) -> Self {
        Self { a, b, c }
    }

    pub const fn vertex(self, vertex: u32) -> Vec2 {
        match vertex {
            0 => self.a,
            1 => self.b,
            _ => self.c,
        }
    }

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

#[derive(Clone, Copy)]
pub struct TriangleFragment<P: Program> {
    pub pixel: Vec2,
    pub uv: Vec2,
    pub barycentric: Vec3,
    pub time: f32,
    pub globals: P::Globals,
}

impl<P: Program> TriangleFragment<P> {
    pub fn new(pixel: Vec2, triangle: Triangle, time: f32, globals: P::Globals) -> Self {
        let barycentric = triangle.barycentric(pixel);
        Self { pixel, uv: vec2(barycentric.y, barycentric.z), barycentric, time, globals }
    }
}

#[cfg(not(target_arch = "spirv"))]
impl<P: Program> ShaderInput for TriangleFragment<P> {
    type Geometry = Triangle;
    type Program = P;
}

#[cfg(not(target_arch = "spirv"))]
impl super::Geometry for Triangle {
    type Kind = Self;

    fn primitives(self, _: &super::text::Text) -> impl Iterator<Item = [Vec2; 3]> {
        once(self.data())
    }
}
