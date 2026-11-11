//! Text: runs placed on screen, resolved against glyph data, and sampled as signed distance fields.
use crate::{Rect, Sample, Sdf, Shape, raster};
#[cfg(not(target_arch = "spirv"))]
pub use cache::{ShapedLine, TextCache};
use core::ops::{Deref, Range};
use glyph::{Glyph, PlacedGlyph, glyph_distance};
use isthmus::{
    Fragment, Primitive, Program, ResourceData, ShaderData, Vertex, VertexInput,
    glam::{Vec2, vec2},
};

#[cfg(not(target_arch = "spirv"))]
mod cache;
mod glyph;
#[cfg(not(target_arch = "spirv"))]
mod outline;

/// Text placed in the current frame; retain a `ShapedLine` to reuse it later.
#[derive(Clone, Copy, Default, ShaderData)]
#[shader_data(view = Glyphs<'view>)]
pub struct Text {
    /// Minimum rasterization corner in logical screen coordinates.
    pub min: Vec2,
    /// Maximum rasterization corner in logical screen coordinates.
    pub max: Vec2,
    /// Baseline origin in logical screen coordinates.
    pub origin: Vec2,
    /// Font ascent-to-descent span in logical pixels.
    pub size: f32,
    /// Total advance in logical pixels.
    pub width: f32,

    /// Number of placed glyphs in the run.
    pub count: u32,
    /// First glyph in the current frame's placement buffer.
    first: u32,
    outline: f32,
}

impl Text {
    /// Declares an exterior outline for raster bounds and fragment coverage queries.
    pub const fn outlined(mut self, width: f32) -> Self {
        self.outline = width.max(0.0);
        self
    }

    /// Moves the baseline and rasterization bounds by a logical pixel offset.
    pub fn translated(mut self, offset: Vec2) -> Self {
        self.min += offset;
        self.max += offset;
        self.origin += offset;
        self
    }

    /// Centers the advance and font metrics on a logical screen position.
    pub fn centered(self, center: Vec2) -> Self {
        self.translated(center - vec2(self.width * 0.5, 0.0))
    }

    /// Aligns the right advance edge and vertical center to a screen position.
    pub fn right(self, position: Vec2) -> Self {
        self.translated(position - vec2(self.width, 0.0))
    }

    /// Centers within horizontal bounds if the advance fits, otherwise aligns left.
    pub fn fit(self, y: f32, bounds: Range<f32>) -> Self {
        let x = if self.width <= bounds.end - bounds.start + 0.5 {
            (bounds.start + bounds.end - self.width) * 0.5
        } else {
            bounds.start
        };
        self.translated(vec2(x, y))
    }
}

/// A text run resolved against the glyph and outline buffers, ready for SDF sampling.
#[derive(Clone, Copy)]
pub struct Glyphs<'resources> {
    pub line: Text,
    pub resources: &'resources [u32],
}

impl Deref for Glyphs<'_> {
    type Target = Text;

    fn deref(&self) -> &Text {
        &self.line
    }
}

impl<'resources> Glyphs<'resources> {
    /// Chooses this run, or `other` when `second` is set. Both must come from the same frame. Only the plain
    /// run data is selected, since a shader cannot select between buffer references, so this is what lets
    /// a shader loop over several runs and keep one copy of the glyph code.
    pub const fn either(self, other: Self, second: bool) -> Self {
        Self { line: if second { other.line } else { self.line }, resources: self.resources }
    }

    /// Resolves a text handle against its frame resources on either CPU or GPU.
    pub const fn new(line: Text, resources: ResourceData<'resources>) -> Self {
        Self { line, resources }
    }
}

impl Sdf for Glyphs<'_> {
    fn geometry_bounds(self) -> Rect {
        if self.count == 0 { Rect::EMPTY } else { Rect::new(self.min, self.max) }
    }

    fn outline_width(self) -> f32 {
        self.line.outline
    }

    fn distance_at(self, point: Vec2) -> f32 {
        if self.line.count == 0 || self.line.size <= 0.0 {
            return f32::MAX;
        }

        let line_point = (point - self.line.origin) / self.line.size;
        let mut low = 0;
        let mut high = self.line.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let glyph = PlacedGlyph::read(self.resources, (self.line.first + middle) as usize * PlacedGlyph::WORDS);
            if glyph.left <= line_point.x {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let (mut left, mut right) = (low, low);
        let mut best = f32::MAX;
        while left > 0 || right < self.line.count {
            let before =
                PlacedGlyph::read(self.resources, (self.line.first + left.max(1) - 1) as usize * PlacedGlyph::WORDS);
            let after = PlacedGlyph::read(
                self.resources,
                (self.line.first + right.min(self.line.count - 1)) as usize * PlacedGlyph::WORDS,
            );
            let left_gap = if left > 0 { line_point.x - before.right } else { f32::MAX };
            let right_gap = if right < self.line.count { after.left - line_point.x } else { f32::MAX };
            if left_gap.min(right_gap) >= best {
                break;
            }
            let placed = if left_gap < right_gap {
                left -= 1;
                before
            } else {
                right += 1;
                after
            };
            let glyph = Glyph::read(self.resources, placed.glyph as usize);
            let glyph_point = vec2(line_point.x - placed.x, placed.y - line_point.y);
            if Shape::rectangle(Rect::new(glyph.min, glyph.max)).distance_at(glyph_point) < best {
                let distance = if glyph.field_size.x > 0 {
                    glyph.field_distance(self.resources, glyph_point)
                } else {
                    glyph_distance(self.resources, glyph.start, glyph.count, glyph_point)
                };
                best = best.min(distance);
            }
        }
        best * self.line.size
    }
}

impl<P: Program> Primitive<P> for Glyphs<'_> {
    type Sample = Sample;

    fn vertex_count(self) -> u32 {
        Primitive::<P>::vertex_count(raster(self.bounds(0.0)))
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        raster(self.bounds(0.0)).vertex(input)
    }

    fn sample(self, fragment: Fragment) -> (Sample, f32) {
        let sample = self.sample_at(fragment.pixel);
        (sample, sample.coverage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_selects_a_run_and_keeps_the_shared_resources() {
        let resources = [1, 2, 3];
        let (first, second) = (Text { count: 1, ..Text::default() }, Text { count: 2, ..Text::default() });
        let (first, second) = (Glyphs::new(first, &resources), Glyphs::new(second, &resources));
        assert_eq!(first.either(second, false).line.count, 1);
        assert_eq!(first.either(second, true).line.count, 2);
        assert_eq!(first.either(second, true).resources, &resources);
    }
}
