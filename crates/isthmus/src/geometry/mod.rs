mod fragment;
mod quad;
/// Bounded shape expressions and antialiased distance-field coverage.
pub mod sdf;
pub mod text;
mod triangle;

pub use fragment::{Fragment, GeometrySample, ShaderInput};
pub use quad::{Quad, QuadSample};
pub use triangle::{Triangle, TriangleSample};
/// Fragment context with triangle coordinates.
pub type TriangleFragment<P> = Fragment<P, TriangleSample>;

/// Triangle-strip rasterization shared by geometry preparation and generated vertex shaders.
pub trait Raster: Copy {
    /// Number of vertices in the primitive's triangle strip.
    const VERTICES: u32;
    /// Reconstructs a primitive from its three encoded vectors.
    fn from_data(data: [glam::Vec2; 3]) -> Self;
    /// Returns the screen position of a triangle-strip vertex.
    fn vertex(self, vertex: u32) -> glam::Vec2;
}

#[cfg(not(target_arch = "spirv"))]
/// Geometry accepted by paint, with a statically matched fragment sample.
pub trait Geometry: Copy {
    /// Fragment queries supported by this geometry.
    type Sample: GeometrySample<'static>;
    /// Host resources required to prepare this geometry.
    type Context: ?Sized;
    /// Encodes the geometry-specific data shared by its primitives.
    fn payload(self) -> <Self::Sample as GeometrySample<'static>>::Payload;
    /// Produces raster primitives using the supplied host resources.
    fn primitives(self, context: &Self::Context) -> impl Iterator<Item = [glam::Vec2; 3]>;
}

/// Raster geometry and its associated payload address for one draw.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct DrawRecord {
    /// Three vectors interpreted by the geometry's raster type.
    pub geometry: [glam::Vec2; 3],
    /// Word offset of the draw's captured payload.
    pub payload: u32,
}
