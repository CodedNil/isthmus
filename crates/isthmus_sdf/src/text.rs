//! Vector text geometry and rendering.

use crate::{Sample, Shape};
use core::ops::{Deref, Range};
#[cfg(target_arch = "spirv")]
use isthmus::Float as _;
use isthmus::{
    F16x2, Fragment, Primitive, Program, Quad, ResourceData, ShaderData, Vertex, VertexInput,
    glam::{Vec2, vec2},
    surface,
};

#[derive(Clone, Copy, ShaderData)]
#[doc(hidden)]
pub struct Curve {
    pub(super) points: [F16x2; 3],
}

impl Curve {
    pub(super) fn points(self) -> [Vec2; 3] {
        [self.points[0].to_vec2(), self.points[1].to_vec2(), self.points[2].to_vec2()]
    }
}

/// A placed text run; its draw stage supplies glyph data for explicit fragment sampling.
#[derive(Clone, Copy, Default, ShaderData)]
#[shader_data(view = Glyphs<'a>)]
#[must_use]
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
    pub(super) prepared_weight: Weight,
    /// Number of placed glyphs in the run.
    pub count: u32,
    /// First glyph in the current frame's placement buffer.
    pub first: u32,
    pub(super) outline: f32,
}

impl Text {
    /// Declares an exterior outline for raster bounds and fragment coverage queries.
    pub const fn outlined(mut self, width: f32) -> Self {
        self.outline = width.max(0.0);
        self
    }

    /// Returns raster bounds covering all supported font weights and the declared outline.
    pub fn bounds(self) -> Quad {
        if self.count == 0 {
            Quad::new(self.origin, Vec2::splat(-f32::MAX), Vec2::X)
        } else {
            Quad::from_min_max(self.min, self.max).expanded(self.outline)
        }
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

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct Glyphs<'a> {
    pub line: Text,
    pub placed_glyphs: &'a [u32],
    pub outlines: &'a [u32],
}

impl Deref for Glyphs<'_> {
    type Target = Text;

    fn deref(&self) -> &Text {
        &self.line
    }
}

impl<'a> Glyphs<'a> {
    /// Resolves a text handle against its frame resources on either CPU or GPU.
    pub const fn new(line: Text, resources: ResourceData<'a>) -> Self {
        Self { line, placed_glyphs: resources.transient, outlines: resources.persistent }
    }
}

#[derive(Clone, Copy, Default, ShaderData)]
pub(super) struct Weight {
    master: u32,
    blend: f32,
}

impl Weight {
    pub(super) fn resolve(weights: &[u32], weight: f32) -> Self {
        let mut low = 0;
        let last = u32::read(weights, 0).max(1) - 1;
        let mut high = last;
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            if f32::read(weights, middle as usize + 1) <= weight {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let next = (low + 1).min(last);
        let a = f32::read(weights, low as usize + 1);
        let b = f32::read(weights, next as usize + 1);
        let blend = if b > a { ((weight - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        Self { master: low, blend }
    }
}

impl Glyphs<'_> {
    /// Evaluates the selected weight once; fill and outline masks supply the shader's final alpha.
    pub fn sample_at(self, point: Vec2) -> Sample {
        Sample::new(self.distance_at(point), self.line.outline)
    }

    /// Selects a font weight for analytic distance queries.
    #[must_use]
    pub fn with_weight(self, weight: f32) -> Self {
        let line = Text { prepared_weight: Weight::resolve(self.outlines, weight), ..self.line };
        Self { line, ..self }
    }

    /// Evaluates the analytic glyph distance at a screen position.
    pub fn distance_at(self, point: Vec2) -> f32 {
        let Self { line, placed_glyphs, outlines } = self;
        if line.count == 0 || line.size <= 0.0 {
            return f32::MAX;
        }
        let weight = line.prepared_weight;
        let line_point = (point - line.origin) / line.size;
        let mut low = 0;
        let mut high = line.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let glyph = PlacedGlyph::read(placed_glyphs, (line.first + middle) as usize * PlacedGlyph::WORDS);
            if glyph.left <= line_point.x {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let (mut left, mut right) = (low, low);
        let mut best = f32::MAX;
        while left > 0 || right < line.count {
            let before = PlacedGlyph::read(placed_glyphs, (line.first + left.max(1) - 1) as usize * PlacedGlyph::WORDS);
            let after = PlacedGlyph::read(
                placed_glyphs,
                (line.first + right.min(line.count - 1)) as usize * PlacedGlyph::WORDS,
            );
            let left_gap = if left > 0 { line_point.x - before.right } else { f32::MAX };
            let right_gap = if right < line.count { after.left - line_point.x } else { f32::MAX };
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
            let glyph = Glyph::read(outlines, placed.glyph as usize);
            let glyph_point = vec2(line_point.x - placed.x, placed.y - line_point.y);
            if Shape::rectangle(Quad::from_min_max(glyph.min, glyph.max)).distance_at(glyph_point) < best {
                best = best.min(glyph_distance(outlines, glyph.start, glyph.count, weight, glyph_point));
            }
        }
        best * line.size
    }
}

impl<P: Program> Primitive<P> for Glyphs<'_> {
    type Outputs = ();
    type Sample = Self;

    fn vertex_count(self, pixel_size: f32) -> u32 {
        Primitive::<P>::vertex_count(surface(Some(self.bounds()), |_| ((), 1.0)), pixel_size)
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        surface(Some(self.bounds()), |_| ((), 1.0)).vertex(input)
    }

    fn sample(self, _: Fragment, (): ()) -> (Self, f32) {
        (self, 1.0)
    }
}

#[derive(Clone, Copy, ShaderData)]
#[doc(hidden)]
pub struct Glyph {
    pub(super) min: Vec2,
    pub(super) max: Vec2,
    pub(super) start: u32,
    pub(super) count: u32,
}

#[derive(Clone, Copy, PartialEq, ShaderData)]
#[doc(hidden)]
pub struct PlacedGlyph {
    pub x: f32,
    pub y: f32,
    pub glyph: u32,
    pub left: f32,
    pub right: f32,
}

fn quadratic(start: Vec2, control: Vec2, end: Vec2, t: f32) -> Vec2 {
    let inverse = 1.0 - t;
    start * inverse * inverse + control * 2.0 * inverse * t + end * t * t
}

fn quadratic_roots(a: f32, b: f32, c: f32) -> [f32; 2] {
    if a == 0.0 {
        return [if b == 0.0 { -1.0 } else { -c / b }, -1.0];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return [-1.0; 2];
    }
    if discriminant == 0.0 {
        return [-0.5 * b / a, -1.0];
    }
    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
    [q / a, c / q]
}

#[expect(clippy::manual_range_contains, reason = "Range::contains does not lower through Rust-GPU")]
#[inline(never)]
fn curve_winding(start: Vec2, control: Vec2, end: Vec2, point: Vec2) -> i32 {
    let a = start.y - 2.0 * control.y + end.y;
    let b = 2.0 * (control.y - start.y);
    let c = start.y - point.y;
    if a != 0.0 && b * b - 4.0 * a * c <= 0.0 {
        return 0;
    }
    let crossing = |t: f32| {
        let direction = 2.0 * a * t + b;
        let crosses = if direction > 0.0 { t >= 0.0 && t < 1.0 } else { t > 0.0 && t <= 1.0 };
        if crosses && quadratic(start, control, end, t).x > point.x {
            if direction > 0.0 {
                1
            } else if direction < 0.0 {
                -1
            } else {
                0
            }
        } else {
            0
        }
    };
    let roots = quadratic_roots(a, b, c);
    crossing(roots[0]) + crossing(roots[1])
}

fn curve_distance([start, control, end]: [Vec2; 3], point: Vec2, best: f32) -> (f32, i32) {
    let min = start.min(control).min(end);
    let max = start.max(control).max(end);
    let winding = if point.y >= min.y && point.y <= max.y { curve_winding(start, control, end, point) } else { 0 };
    if (point - point.clamp(min, max)).length_squared() >= best {
        return (best, winding);
    }
    let a = start - control * 2.0 + end;
    let b = (control - start) * 2.0;
    let c = start - point;
    let mut distance = best.min(c.length_squared()).min((end - point).length_squared());
    if a.length_squared() == 0.0 {
        let t = (-c.dot(b) / b.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        return (distance.min((c + b * t).length_squared()), winding);
    }
    let coefficients = [2.0 * a.dot(a), 3.0 * a.dot(b), b.dot(b) + 2.0 * a.dot(c), b.dot(c)];
    let evaluate = |t: f32| ((coefficients[0] * t + coefficients[1]) * t + coefficients[2]) * t + coefficients[3];
    let turns = quadratic_roots(3.0 * coefficients[0], 2.0 * coefficients[1], coefficients[2]);
    let intervals = [0.0, turns[0].min(turns[1]).clamp(0.0, 1.0), turns[0].max(turns[1]).clamp(0.0, 1.0), 1.0];
    for index in 0..3 {
        let mut lo = intervals[index];
        let mut hi = intervals[index + 1];
        distance = distance.min((quadratic(start, control, end, hi) - point).length_squared());
        let negative = evaluate(lo) < 0.0;
        if lo >= hi || negative == (evaluate(hi) < 0.0) {
            continue;
        }
        let mut t = (lo + hi) * 0.5;
        // Newton steps stay bracketed within a monotone interval of the stationary cubic.
        for _ in 0..24 {
            let value = evaluate(t);
            if value == 0.0 {
                break;
            }
            if negative == (value < 0.0) {
                lo = t;
            } else {
                hi = t;
            }
            let slope = (3.0 * coefficients[0] * t + 2.0 * coefficients[1]) * t + coefficients[2];
            let next = t - value / slope;
            if (next - t).abs() <= f32::EPSILON * 2.0 {
                break;
            }
            t = if next > lo && next < hi { next } else { (lo + hi) * 0.5 };
        }
        distance = distance.min((quadratic(start, control, end, t) - point).length_squared());
    }
    (distance, winding)
}

fn glyph_distance(curves: &[u32], start: u32, count: u32, weight: Weight, point: Vec2) -> f32 {
    let mut distance_squared = f32::MAX;
    let mut winding = 0;
    // Rust-GPU cannot lower this runtime slice iterator without a pointer-to-integer conversion.
    for index in 0..count {
        let a = Curve::read(curves, start as usize + (weight.master * count + index) as usize * Curve::WORDS).points();
        let points = if weight.blend == 0.0 {
            a
        } else {
            let b = Curve::read(curves, start as usize + ((weight.master + 1) * count + index) as usize * Curve::WORDS)
                .points();
            [a[0].lerp(b[0], weight.blend), a[1].lerp(b[1], weight.blend), a[2].lerp(b[2], weight.blend)]
        };
        let (distance, edge_winding) = curve_distance(points, point, distance_squared);
        distance_squared = distance;
        winding += edge_winding;
    }
    distance_squared.sqrt() * if winding == 0 { 1.0 } else { -1.0 }
}
