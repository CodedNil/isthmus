mod quad;
pub mod sdf;
pub mod text;
mod triangle;

pub use quad::{Fragment, Quad};
pub use triangle::{Triangle, TriangleFragment};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    Quad,
    Triangle,
}

#[cfg(not(target_arch = "spirv"))]
/// Geometry accepted by paint, with a statically matched shader input kind.
pub trait Geometry: Copy {
    type Kind;
    fn primitives(self, text: &text::Text) -> impl Iterator<Item = [glam::Vec2; 3]>;
}

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
pub struct DrawRecord {
    pub geometry: [glam::Vec2; 3],
    pub payload: u32,
    pub(crate) _padding: u32,
}

#[cfg(not(target_arch = "spirv"))]
pub trait ShaderInput {
    type Program: crate::Program;
    type Geometry;
}
