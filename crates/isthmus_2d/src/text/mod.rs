//! Text: shaped lines, placed on screen and sampled as signed distance fields on the GPU.
use crate::{Rect, Sample, Sdf, Shape};
use core::ops::{Deref, Range};
use glyph::{Glyph, PlacedGlyph, glyph_distance};
use isthmus::{
    Fragment, Primitive, Sent, ShaderData, SharedWords,
    glam::{Vec2, vec2},
};
#[cfg(not(target_arch = "spirv"))]
pub use line::{Line, install_fonts};

mod glyph;
#[cfg(not(target_arch = "spirv"))]
mod line;
#[cfg(not(target_arch = "spirv"))]
mod outline;

/// The placement of text in the current frame, which is what travels in a draw's record.
#[derive(Clone, Copy, Default, ShaderData)]
pub struct TextRecord {
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
}

/// Text ready to send to a shader and sample there as a signed distance field: a [`Line`] placed on screen.
///
/// Make one with [`Line::at`]. On the CPU it carries only its placement, and its glyphs are written to the frame
/// when it is sent with `.with(..)`. On the GPU it is read from the frame's words.
#[derive(Clone, Copy, Default)]
pub struct Text<'frame> {
    pub line: TextRecord,
    words: &'frame [u32],
    #[cfg(not(target_arch = "spirv"))]
    shaped: Option<&'frame Line>,
}

impl Deref for Text<'_> {
    type Target = TextRecord;

    fn deref(&self) -> &TextRecord {
        &self.line
    }
}

impl<'frame> Text<'frame> {
    /// Text read from words that hold its glyphs, as the GPU reads it, for example to sample it on the CPU.
    pub const fn on(line: TextRecord, words: SharedWords<'frame>) -> Self {
        Self {
            line,
            words,
            #[cfg(not(target_arch = "spirv"))]
            shaped: None,
        }
    }

    /// Text to write to the frame from a line, once it is sent.
    #[cfg(not(target_arch = "spirv"))]
    pub(crate) const fn of(shaped: &'frame Line, line: TextRecord) -> Self {
        Self { line, words: &[], shaped: Some(shaped) }
    }

    /// Chooses this run, or `other` when `second` is set. Both must come from the same frame. Only the plain
    /// run data is selected, since a shader cannot select between buffer references, so this is what lets
    /// a shader loop over several runs and keep one copy of the glyph code.
    pub const fn either(self, other: Self, second: bool) -> Self {
        Self { line: if second { other.line } else { self.line }, ..self }
    }

    /// Moves the baseline and rasterization bounds by a logical pixel offset.
    pub fn translated(mut self, offset: Vec2) -> Self {
        self.line.min += offset;
        self.line.max += offset;
        self.line.origin += offset;
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

impl Sdf for Text<'_> {
    fn geometry_bounds(self) -> Rect {
        if self.count == 0 { Rect::EMPTY } else { Rect::new(self.min, self.max) }
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
            let glyph = PlacedGlyph::read(self.words, (self.line.first + middle) as usize * PlacedGlyph::WORDS);
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
                PlacedGlyph::read(self.words, (self.line.first + left.max(1) - 1) as usize * PlacedGlyph::WORDS);
            let after = PlacedGlyph::read(
                self.words,
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
            let (at, glyph_point) = (placed.glyph as usize, vec2(line_point.x - placed.x, placed.y - line_point.y));
            let glyph = Glyph::read(self.words, at);
            let body = (at + Glyph::WORDS) as u32;
            if Shape::rectangle(Rect::new(glyph.min, glyph.max)).distance_at(glyph_point) < best {
                let distance = if glyph.field_size.x > 0 {
                    glyph.field_distance(self.words, body, glyph_point)
                } else {
                    glyph_distance(self.words, body, glyph.count, glyph_point)
                };
                best = best.min(distance);
            }
        }
        best * self.line.size
    }
}

/// Text is sent as its placement, and restored on the GPU against the frame's words.
impl Sent for Text<'_> {
    type Record = TextRecord;
    type Restored<'words> = Text<'words>;

    fn restore(record: TextRecord, words: SharedWords<'_>) -> Text<'_> {
        Text::on(record, words)
    }
}

impl Primitive for Text<'_> {
    type Sample = Sample;

    fn region(self, pixel_size: f32) -> Option<[Vec2; 4]> {
        self.bounds(0.0).padded_region(pixel_size)
    }

    fn sample(self, fragment: Fragment) -> (Sample, f32) {
        let sample = self.sample_at(fragment.pixel);
        (sample, sample.coverage)
    }
}
