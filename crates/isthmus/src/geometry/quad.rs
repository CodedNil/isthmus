use crate::Program;
#[cfg(not(target_arch = "spirv"))]
use {crate::geometry::ShaderInput, core::iter::once};

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Quad {
    pub center: glam::Vec2,
    pub size: glam::Vec2,
    pub axis: glam::Vec2,
}

impl Quad {
    pub const fn data(self) -> [glam::Vec2; 3] {
        [self.center, self.size, self.axis]
    }

    pub const fn from_data([center, size, axis]: [glam::Vec2; 3]) -> Self {
        Self { center, size, axis }
    }

    pub const fn new(center: glam::Vec2, size: glam::Vec2, axis: glam::Vec2) -> Self {
        Self { center, size, axis }
    }

    /// Creates an oriented quad, falling back to the x-axis for a zero direction.
    pub fn oriented(center: glam::Vec2, size: glam::Vec2, direction: glam::Vec2) -> Self {
        Self::new(center, size, direction.normalize_or(glam::Vec2::X))
    }

    pub fn from_min_max(min: glam::Vec2, max: glam::Vec2) -> Self {
        Self::new(min.midpoint(max), max - min, glam::Vec2::X)
    }

    pub fn vertex(self, vertex: u32) -> glam::Vec2 {
        let local = (glam::vec2((vertex & 1) as f32, (vertex >> 1) as f32) - 0.5) * self.size;
        self.center + self.axis * local.x + self.axis.perp() * local.y
    }

    pub fn local(self, pixel: glam::Vec2) -> glam::Vec2 {
        let offset = pixel - self.center;
        glam::vec2(offset.dot(self.axis), offset.dot(self.axis.perp()))
    }

    #[must_use]
    pub fn expanded(mut self, amount: f32) -> Self {
        self.size += amount * 2.0;
        self
    }
}

#[derive(Clone, Copy)]
pub struct Fragment<P: Program> {
    pub pixel: glam::Vec2,
    pub local: glam::Vec2,
    pub uv: glam::Vec2,
    pub time: f32,
    pub globals: P::Globals,
}

impl<P: Program> Fragment<P> {
    pub fn new(pixel: glam::Vec2, quad: Quad, time: f32, globals: P::Globals) -> Self {
        let local = quad.local(pixel);
        Self { pixel, local, uv: local / quad.size + 0.5, time, globals }
    }
}

#[cfg(not(target_arch = "spirv"))]
impl<P: Program> ShaderInput for Fragment<P> {
    type Geometry = Quad;
    type Program = P;
}

#[cfg(not(target_arch = "spirv"))]
impl<T: Copy + Into<Quad>> super::Geometry for T {
    type Kind = Quad;

    fn primitives(self, _: &super::text::Text) -> impl Iterator<Item = [glam::Vec2; 3]> {
        once(self.into().data())
    }
}
