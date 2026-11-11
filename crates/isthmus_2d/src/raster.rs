//! Bounded fragment samplers, rasterized with an antialiasing margin.
use crate::Rect;
use isthmus::{Fragment, Primitive, Program, Vertex, VertexInput, glam::Vec2};

/// A bounded fragment sampler rasterized with an antialiasing margin.
#[derive(Clone, Copy)]
pub struct Surface<F> {
    bounds: Rect,
    sample: F,
}

/// Pairs conservative bounds with a shader returning prepared data and coverage.
pub const fn surface<F>(bounds: Rect, sample: F) -> Surface<F> {
    Surface { bounds, sample }
}

/// Rasterizes conservative bounds with automatic antialiasing padding for custom fragment sampling.
pub fn raster(bounds: Rect) -> Surface<impl Fn(Vec2) -> ((), f32) + Copy> {
    surface(bounds, |_| ((), 1.0))
}

impl<P: Program, T, F: FnOnce(Vec2) -> (T, f32) + Copy> Primitive<P> for Surface<F> {
    type Sample = T;

    fn vertex_count(self) -> u32 {
        Primitive::<P>::vertex_count(self.bounds)
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        self.bounds.expanded(input.frame.pixel_size()).vertex(input)
    }

    fn sample(self, fragment: Fragment) -> (T, f32) {
        (self.sample)(fragment.pixel)
    }
}
