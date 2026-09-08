use crate::{Fragment, Primitive, Program, Quad, ShaderData, Vertex, VertexInput};
use glam::{Vec2, vec2};

/// A triangle defined by three logical screen positions.
#[derive(Clone, Copy, ShaderData)]
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
        let axis = Quad::oriented(center, size, direction).axis;
        let along = axis * size.x * 0.5;
        let across = axis.perp() * size.y * 0.5;
        Self::new(center + along, center - along + across, center - along - across)
    }
}

impl<P: Program> Primitive<P> for Triangle {
    type Outputs = ();
    type Sample = ();

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }

    fn vertex_count(self, _: f32) -> u32 {
        3
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let (pixel, uv) = match input.index {
            0 => (self.a, Vec2::ZERO),
            1 => (self.b, vec2(1.0, 0.0)),
            _ => (self.c, vec2(0.0, 1.0)),
        };
        input.project(pixel, uv)
    }
}
