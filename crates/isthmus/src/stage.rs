//! The contract between a shader's stages: what a primitive provides to each, and what each receives.
use crate::{
    Program, ShaderFrame,
    glam::{Vec2, Vec4, vec4},
};
use core::ops::Deref;

/// One vertex's index and shared frame data.
#[derive(Clone, Copy)]
pub struct VertexInput<P: Program> {
    /// Index within the submitted triangle strip.
    pub index: u32,
    /// Frame data supplied to the stage constructor.
    pub frame: ShaderFrame<P>,
}

impl<P: Program> VertexInput<P> {
    /// Projects a logical position and supplies its texture coordinates.
    pub fn project(self, pixel: Vec2, uv: Vec2) -> Vertex {
        let ndc = pixel / self.frame.screen_size * 2.0 - 1.0;
        Vertex { position: vec4(ndc.x, -ndc.y, 0.0, 1.0), uv }
    }
}

/// Value returned by a vertex stage.
#[derive(Clone, Copy)]
pub struct Vertex {
    /// Homogeneous clip-space position.
    pub position: Vec4,
    /// Texture coordinates interpolated across the primitive.
    pub uv: Vec2,
}

/// Raster geometry and fragment preparation for a triangle strip.
pub trait Primitive<P: Program>: Copy {
    /// Prepared input supplied to the fragment closure.
    type Sample;
    /// Number of vertices to submit; zero suppresses the draw.
    fn vertex_count(self) -> u32;
    /// Evaluates one vertex entirely on the GPU.
    fn vertex(self, input: VertexInput<P>) -> Vertex;
    /// Prepares shading data and antialiased coverage before user fragment evaluation.
    fn sample(self, fragment: Fragment) -> (Self::Sample, f32);
}

/// Rasterized fragment coordinates.
#[derive(Clone, Copy)]
pub struct Fragment<T = ()> {
    /// Actual screen position in logical pixels.
    pub pixel: Vec2,
    /// Interpolated coordinates supplied by the vertex stage.
    pub uv: Vec2,
    /// Primitive-specific data, also accessible through dereferencing this fragment.
    pub sample: T,
    /// Coverage applied by the renderer after fragment evaluation.
    pub coverage: f32,
}

impl<T> Deref for Fragment<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.sample
    }
}
