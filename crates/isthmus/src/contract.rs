use crate::ShaderData;
#[cfg(not(target_arch = "spirv"))]
use crate::{TextFragment, text::Line};
use spirv_std::{Sampler, image::Image2d};

/// Loads a generated shader value from its recorded byte-addressed buffer offset.
#[doc(hidden)]
/// # Safety
/// The offset must address a complete, correctly aligned value of T in the buffer.
pub unsafe fn load<T: ShaderData>(buffer: &[u32], byte_index: u32) -> T {
    // SAFETY: Generated shaders only request recorded, correctly aligned values of T.
    unsafe { spirv_std::ByteAddressableBuffer::from_slice(buffer).load(byte_index) }
}

#[doc(hidden)]
pub struct ShaderImage<'a> {
    image: &'a Image2d,
    sampler: Sampler,
}

impl<'a> ShaderImage<'a> {
    pub const fn new(image: &'a Image2d, sampler: Sampler) -> Self {
        Self { image, sampler }
    }

    pub fn sample(&self, uv: glam::Vec2) -> glam::Vec4 {
        self.image.sample(self.sampler, uv)
    }
}

#[doc(hidden)]
#[repr(C)]
#[derive(Clone, Copy, Default, crate::ShaderData)]
pub struct PushBlock {
    pub screen_size: glam::Vec2,
    pub time: f32,
    pub(crate) _padding: f32,
}

/// A nominal shader program whose interfaces are generated together.
///
/// # Safety
/// Metadata and code must come from the same validated program and match its globals layout.
pub unsafe trait Program: Copy + 'static {
    type Globals: ShaderData + Default;
    #[cfg(not(target_arch = "spirv"))]
    const CODE: &'static [u8];
    #[cfg(not(target_arch = "spirv"))]
    const SHADERS: &'static [ShaderEntry];
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    Over,
    Add,
    Replace,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_arch = "spirv"))]
pub enum Primitive {
    Quad,
    Triangle,
}

#[cfg(not(target_arch = "spirv"))]
pub struct ShaderEntry {
    pub name: &'static str,
    pub blend: Blend,
    pub primitive: Primitive,
}

#[cfg(not(target_arch = "spirv"))]
/// # Panics
/// Fails constant evaluation when a shader is absent from its generated program.
pub const fn shader_index(entries: &[ShaderEntry], name: &str) -> usize {
    let mut index = 0;
    while index < entries.len() {
        let a = entries[index].name.as_bytes();
        let b = name.as_bytes();
        let mut byte = 0;
        if a.len() == b.len() {
            while byte < a.len() && a[byte] == b[byte] {
                byte += 1;
            }
            if byte == a.len() {
                return index;
            }
        }
        index += 1;
    }
    panic!("shader was not extracted into this program");
}

/// Straight-alpha color operations; shader output conversion belongs to Isthmus.
pub trait ColorExt {
    #[must_use]
    fn opacity(self, opacity: f32) -> Self;
}

impl ColorExt for glam::Vec4 {
    fn opacity(self, opacity: f32) -> Self {
        self.truncate().extend(self.w * opacity)
    }
}

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Quad {
    pub center: glam::Vec2,
    pub size: glam::Vec2,
    pub axis: glam::Vec2,
}

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
pub struct DrawRecord {
    pub geometry: [glam::Vec2; 3],
    pub payload: u32,
    pub(crate) _padding: u32,
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

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Triangle {
    pub a: glam::Vec2,
    pub b: glam::Vec2,
    pub c: glam::Vec2,
}

impl Triangle {
    pub fn oriented(center: glam::Vec2, size: glam::Vec2, direction: glam::Vec2) -> Self {
        let axis = direction.normalize_or(glam::Vec2::X);
        let along = axis * size.x * 0.5;
        let across = axis.perp() * size.y * 0.5;
        Self::new(center + along, center - along + across, center - along - across)
    }

    pub const fn new(a: glam::Vec2, b: glam::Vec2, c: glam::Vec2) -> Self {
        Self { a, b, c }
    }

    pub const fn data(self) -> [glam::Vec2; 3] {
        [self.a, self.b, self.c]
    }

    pub const fn from_data([a, b, c]: [glam::Vec2; 3]) -> Self {
        Self { a, b, c }
    }

    pub fn barycentric(self, pixel: glam::Vec2) -> glam::Vec3 {
        let ab = self.b - self.a;
        let ac = self.c - self.a;
        let ap = pixel - self.a;
        let area = ab.perp_dot(ac);
        let b = ap.perp_dot(ac) / area;
        let c = ab.perp_dot(ap) / area;
        glam::vec3(1.0 - b - c, b, c)
    }
}

#[derive(Clone, Copy)]
pub struct TriangleFragment<P: Program> {
    pub pixel: glam::Vec2,
    pub uv: glam::Vec2,
    pub barycentric: glam::Vec3,
    pub time: f32,
    pub globals: P::Globals,
}

impl<P: Program> TriangleFragment<P> {
    pub fn new(pixel: glam::Vec2, triangle: Triangle, time: f32, globals: P::Globals) -> Self {
        let barycentric = triangle.barycentric(pixel);
        Self { pixel, uv: glam::vec2(barycentric.y, barycentric.z), barycentric, time, globals }
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
    pub fn new(pixel: glam::Vec2, local: glam::Vec2, size: glam::Vec2, time: f32, globals: P::Globals) -> Self {
        Self { pixel, local, uv: local / size + 0.5, time, globals }
    }
}

#[cfg(not(target_arch = "spirv"))]
slotmap::new_key_type! { pub struct SurfaceHandle; }

/// Describes the host-visible interface and fixed state of one shader.
///
/// # Safety
/// The entry must use this payload layout, globals type and geometry in the generated program.
#[cfg(not(target_arch = "spirv"))]
pub unsafe trait ShaderSpec: ShaderData {
    type Program: Program;
    type Geometry;
    const INDEX: usize;
}

#[cfg(not(target_arch = "spirv"))]
pub trait ShaderInput {
    type Program: Program;
    type Geometry;
}

#[cfg(not(target_arch = "spirv"))]
impl<P: Program> ShaderInput for Fragment<P> {
    type Geometry = Quad;
    type Program = P;
}

#[cfg(not(target_arch = "spirv"))]
impl<P: Program> ShaderInput for TextFragment<'_, P> {
    type Geometry = Line;
    type Program = P;
}

#[cfg(not(target_arch = "spirv"))]
impl<P: Program> ShaderInput for TriangleFragment<P> {
    type Geometry = Triangle;
    type Program = P;
}
