//! Screen-space rectangles, in logical pixels.
use isthmus::glam::{Vec2, vec2};
pub use quad::Quad;
pub use rect::Rect;

mod quad;
mod rect;

/// Corner of the unit square for a triangle-strip vertex index.
const fn corner(index: u32) -> Vec2 {
    vec2((index & 1) as f32, (index >> 1) as f32)
}
