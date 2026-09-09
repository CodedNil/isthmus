pub mod quad;
pub mod triangle;

use crate::{Fragment, Primitive, Program, Vertex, VertexInput};
use glam::Vec2;

impl<P: Program, const N: usize> Primitive<P> for [Vec2; N] {
    type Outputs = ();
    type Sample = ();

    fn sample(self, _: Fragment, (): ()) -> ((), f32) {
        ((), 1.0)
    }

    fn vertex_count(self, _: f32) -> u32 {
        const {
            assert!(N <= u32::MAX as usize, "geometry vertex count exceeds u32");
        }
        N as u32
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        let pixel = if (input.index as usize) < N { self[input.index as usize] } else { Vec2::ZERO };
        input.project(pixel, pixel)
    }
}
