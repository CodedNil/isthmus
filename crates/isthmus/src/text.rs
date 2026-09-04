#[cfg(not(target_arch = "spirv"))]
use {
    crate::backend::{BufferRange, Canvas, Context},
    smallvec::SmallVec,
    std::{ops::Range, sync::Arc, vec::Vec},
    ttf_parser::{Face, GlyphId, OutlineBuilder, Tag},
};

#[cfg(target_arch = "spirv")]
use crate::Float as _;
#[cfg(not(target_arch = "spirv"))]
use crate::glam::Vec3;
use crate::{
    Fragment, Quad, Sdf, SdfSample, Unorm8x4,
    glam::{Vec2, Vec4, vec2},
};

const EFFECT_PADDING: f32 = 3.5;
#[cfg(not(target_arch = "spirv"))]
const GLYPH_CAPACITY: usize = 16_384;

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
struct F16x2 {
    value: u32,
}

impl F16x2 {
    #[cfg(not(target_arch = "spirv"))]
    fn new(value: Vec2) -> Self {
        Self {
            value: u32::from(half::f16::from_f32(value.x).to_bits())
                | (u32::from(half::f16::from_f32(value.y).to_bits()) << 16),
        }
    }

    fn get(self) -> Vec2 {
        #[cfg(target_arch = "spirv")]
        {
            spirv_std::float::f16x2_to_vec2(self.value)
        }
        #[cfg(not(target_arch = "spirv"))]
        {
            vec2(
                half::f16::from_bits(self.value as u16).to_f32(),
                half::f16::from_bits((self.value >> 16) as u16).to_f32(),
            )
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
#[doc(hidden)]
pub struct Curve {
    start: F16x2,
    control: F16x2,
    end: F16x2,
    start_delta: F16x2,
    control_delta: F16x2,
    end_delta: F16x2,
    min: Vec2,
    max: Vec2,
}

#[repr(C)]
#[derive(Clone, Copy, Default, crate::ShaderData)]
pub struct Line {
    pub min: Vec2,
    pub max: Vec2,
    pub origin: Vec2,
    pub size: f32,
    pub weight: f32,
    pub count: u32,
    pub first: u32,
    pub color: Unorm8x4,
    padding: f32,
}

impl Line {
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    #[doc(hidden)]
    pub fn quad(self) -> Quad {
        Quad::from_min_max(self.min, self.max)
    }

    /// Extends rasterization beyond the font's normal effect padding.
    #[must_use]
    pub fn expanded(mut self, padding: f32) -> Self {
        self.min -= padding;
        self.max += padding;
        self.padding += padding;
        self
    }

    #[must_use]
    pub fn translated(mut self, offset: Vec2) -> Self {
        self.min += offset;
        self.max += offset;
        self.origin += offset;
        self
    }

    #[must_use]
    pub fn with_color(mut self, color: Vec4) -> Self {
        self.color = Unorm8x4::from_vec4(color);
        self
    }

    fn distance(self, placed_glyphs: &[PlacedGlyph], glyphs: &[Glyph], curves: &[Curve], point: Vec2) -> f32 {
        self.distance_scaled(placed_glyphs, glyphs, curves, point, 1.0)
    }

    fn distance_scaled(
        self,
        placed_glyphs: &[PlacedGlyph],
        glyphs: &[Glyph],
        curves: &[Curve],
        point: Vec2,
        scale: f32,
    ) -> f32 {
        line_distance_scaled(self, placed_glyphs, glyphs, curves, point, scale)
    }
}

/// Shader-side text geometry plus the fragment coordinates of its raster quad.
pub struct TextFragment<'a, Globals = ()> {
    pub pixel: Vec2,
    pub local: Vec2,
    pub uv: Vec2,
    pub time: f32,
    pub globals: Globals,
    pub line: Line,
    placed_glyphs: &'a [PlacedGlyph],
    glyphs: &'a [Glyph],
    curves: &'a [Curve],
}

pub type TextSample = SdfSample;

impl<'a, Globals: Copy> TextFragment<'a, Globals> {
    #[doc(hidden)]
    pub const fn new(
        fragment: Fragment<Globals>,
        line: Line,
        placed_glyphs: &'a [PlacedGlyph],
        glyphs: &'a [Glyph],
        curves: &'a [Curve],
    ) -> Self {
        Self {
            pixel: fragment.pixel,
            local: fragment.local,
            uv: fragment.uv,
            time: fragment.time,
            globals: fragment.globals,
            line,
            placed_glyphs,
            glyphs,
            curves,
        }
    }

    pub fn distance(&self, point: Vec2) -> f32 {
        self.line.distance(self.placed_glyphs, self.glyphs, self.curves, point)
    }

    pub fn distance_scaled(&self, point: Vec2, scale: f32) -> f32 {
        self.line
            .distance_scaled(self.placed_glyphs, self.glyphs, self.curves, point, scale)
    }

    pub fn distance_scaled_with_weight(&self, point: Vec2, scale: f32, weight: f32) -> f32 {
        let mut line = self.line;
        line.weight = weight;
        line.distance_scaled(self.placed_glyphs, self.glyphs, self.curves, point, scale)
    }

    pub fn sample_with_weight(&self, point: Vec2, weight: f32) -> TextSample {
        Sdf::new(self.distance_scaled_with_weight(point, 1.0, weight)).sample()
    }

    pub fn alpha(&self) -> f32 {
        self.alpha_at(self.pixel)
    }

    pub fn alpha_at(&self, point: Vec2) -> f32 {
        Sdf::new(self.distance(point)).fill()
    }

    pub fn color(&self, coverage: f32) -> Vec4 {
        let alpha = coverage * self.line.color.to_vec4().w;
        self.line.color.to_vec3().extend(alpha)
    }
}

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
#[doc(hidden)]
pub struct Glyph {
    min: Vec2,
    max: Vec2,
    start: u32,
    count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, crate::ShaderData)]
#[doc(hidden)]
pub struct PlacedGlyph {
    pub x: f32,
    pub y: f32,
    pub glyph: u32,
}

#[cfg(not(target_arch = "spirv"))]
fn normalized_weight(weight: f32) -> f32 {
    ((weight - 600.0) / 300.0).clamp(0.0, 1.0)
}

fn quadratic(start: Vec2, control: Vec2, end: Vec2, t: f32) -> Vec2 {
    let inverse = 1.0 - t;
    start * inverse * inverse + control * 2.0 * inverse * t + end * t * t
}

#[expect(
    clippy::manual_range_contains,
    reason = "Range::contains does not lower cleanly through Rust-GPU"
)]
fn curve_winding(start: Vec2, control: Vec2, end: Vec2, point: Vec2) -> i32 {
    let a = start.y - 2.0 * control.y + end.y;
    let b = 2.0 * (control.y - start.y);
    let c = start.y - point.y;
    let discriminant = b * b - 4.0 * a * c;
    let mut winding = 0;
    let mut test = |t: f32| {
        if t >= 0.0 && t < 1.0 {
            let position = quadratic(start, control, end, t);
            let direction = 2.0 * a * t + b;
            if position.x > point.x {
                winding += if direction > 0.0 {
                    1
                } else if direction < 0.0 {
                    -1
                } else {
                    0
                };
            }
        }
    };
    if a.abs() < 1e-6 {
        if b.abs() >= 1e-6 {
            test(-c / b);
        }
    } else if discriminant >= 0.0 {
        let root = discriminant.sqrt();
        test((-b - root) / (2.0 * a));
        test((-b + root) / (2.0 * a));
    }
    winding
}

fn curve_distance(curve: Curve, weight: f32, point: Vec2, best: f32) -> (f32, i32) {
    let start = curve.start.get() + curve.start_delta.get() * weight;
    let control = curve.control.get() + curve.control_delta.get() * weight;
    let end = curve.end.get() + curve.end_delta.get() * weight;
    let winding = if point.y >= curve.min.y && point.y <= curve.max.y {
        curve_winding(start, control, end, point)
    } else {
        0
    };
    if (point - point.clamp(curve.min, curve.max)).length_squared() >= best {
        return (best, winding);
    }

    let acceleration = start - control * 2.0 + end;
    let velocity = (control - start) * 2.0;
    let mut distance = best;
    for seed in 0..3 {
        let mut t = seed as f32 * 0.5;
        for _ in 0..3 {
            let position = quadratic(start, control, end, t);
            let tangent = acceleration * (2.0 * t) + velocity;
            let derivative = tangent.length_squared() + (position - point).dot(acceleration * 2.0);
            if derivative.abs() > 1e-8 {
                t = (t - (position - point).dot(tangent) / derivative).clamp(0.0, 1.0);
            }
        }
        distance = distance.min((quadratic(start, control, end, t) - point).length_squared());
    }
    (distance, winding)
}

fn glyph_distance(curves: &[Curve], start: u32, count: u32, weight: f32, point: Vec2, size: f32) -> f32 {
    let mut distance_squared = f32::MAX;
    let mut winding = 0;
    // Rust-GPU cannot lower this runtime slice iterator without a pointer-to-integer conversion.
    for index in 0..count {
        let (distance, edge_winding) =
            curve_distance(curves[(start + index) as usize], weight, point, distance_squared);
        distance_squared = distance;
        winding += edge_winding;
    }
    let scaled_squared = distance_squared * size * size;
    let distance = if scaled_squared >= EFFECT_PADDING * EFFECT_PADDING {
        EFFECT_PADDING
    } else {
        scaled_squared.sqrt()
    };
    distance * if winding == 0 { 1.0 } else { -1.0 }
}

fn glyph_after(placed_glyphs: &[PlacedGlyph], first: u32, count: u32, x: f32) -> u32 {
    let mut low = 0;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        if placed_glyphs[(first + middle) as usize].x <= x {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

fn line_distance_scaled(
    line: Line,
    placed_glyphs: &[PlacedGlyph],
    glyphs: &[Glyph],
    curves: &[Curve],
    local: Vec2,
    scale: f32,
) -> f32 {
    let inverse_size = 1.0 / line.size;
    let line_point = (local - line.origin) * inverse_size;
    let after = glyph_after(placed_glyphs, line.first, line.count, line_point.x);
    let mut best: f32 = 1e6;
    let padding = EFFECT_PADDING * inverse_size / scale;
    // Include the next origin too: italic/curved glyphs and the effect padding can overhang left.
    let mut glyph_index = (after + 1).min(line.count);
    while glyph_index > 0 {
        glyph_index -= 1;
        let placed = placed_glyphs[(line.first + glyph_index) as usize];
        let glyph = glyphs[placed.glyph as usize];
        let glyph_point = vec2(line_point.x - placed.x, placed.y - line_point.y) / scale;
        if glyph_point.x > glyph.max.x + padding {
            break;
        }
        if glyph_point.x >= glyph.min.x - padding
            && glyph_point.y >= glyph.min.y - padding
            && glyph_point.x <= glyph.max.x + padding
            && glyph_point.y <= glyph.max.y + padding
        {
            best = best.min(glyph_distance(
                curves,
                glyph.start,
                glyph.count,
                line.weight,
                glyph_point,
                line.size * scale,
            ));
        }
    }
    best
}

#[cfg(not(target_arch = "spirv"))]
const WGHT: Tag = Tag::from_bytes(b"wght");
#[cfg(not(target_arch = "spirv"))]
const RANGES: &[(u32, u32)] = &[
    (0x20, 0x7e),
    (0xa0, 0xff),
    (0x100, 0x17f),
    (0x300, 0x36f),
    (0x370, 0x3ff),
    (0x400, 0x4ff),
    (0x2000, 0x206f),
    (0x20ac, 0x20ac),
    (0x266a, 0x266b),
];

#[cfg(not(target_arch = "spirv"))]
#[derive(Clone, Copy)]
struct Meta {
    glyph: u32,
    data: Glyph,
    advance: [f32; 2],
}

#[cfg(not(target_arch = "spirv"))]
#[derive(Default)]
struct Outline {
    edges: Vec<[Vec2; 3]>,
    first: Vec2,
    current: Vec2,
}

#[cfg(not(target_arch = "spirv"))]
impl Outline {
    fn segment(&mut self, point: Vec2) {
        self.edges.push([self.current, self.current.midpoint(point), point]);
        self.current = point;
    }
}

#[cfg(not(target_arch = "spirv"))]
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.first = vec2(x, y);
        self.current = self.first;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.segment(vec2(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let end = vec2(x, y);
        self.edges.push([self.current, vec2(x1, y1), end]);
        self.current = end;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (start, control_a, control_b, end) = (self.current, vec2(x1, y1), vec2(x2, y2), vec2(x, y));
        let point = |t: f32| {
            let inverse = 1.0 - t;
            start * inverse * inverse * inverse
                + control_a * 3.0 * inverse * inverse * t
                + control_b * 3.0 * inverse * t * t
                + end * t * t * t
        };
        let tangent = |t: f32| {
            let inverse = 1.0 - t;
            (control_a - start) * 3.0 * inverse * inverse
                + (control_b - control_a) * 6.0 * inverse * t
                + (end - control_b) * 3.0 * t * t
        };
        for step in 0..4 {
            let t0 = step as f32 * 0.25;
            let t1 = (step + 1) as f32 * 0.25;
            let a = point(t0);
            let c = point(t1);
            let control = (a + tangent(t0) * 0.125 + c - tangent(t1) * 0.125) * 0.5;
            self.edges.push([a, control, c]);
        }
        self.current = end;
    }

    fn close(&mut self) {
        if self.current != self.first {
            self.segment(self.first);
        }
    }
}

#[cfg(not(target_arch = "spirv"))]
#[derive(Default)]
pub struct ShapedLine {
    glyphs: SmallVec<[PlacedGlyph; 32]>,
    min: Vec2,
    max: Vec2,
    pub width: f32,
    baseline: f32,
    size: f32,
    weight: f32,
}

#[cfg(not(target_arch = "spirv"))]
#[derive(Clone)]
pub struct Shaper {
    characters: Arc<[(char, Meta)]>,
    baseline: f32,
}

#[cfg(not(target_arch = "spirv"))]
pub(crate) struct Text {
    shaper: Shaper,
    glyphs: Arc<[Glyph]>,
    context: Context,
    placed: Vec<PlacedGlyph>,
    color: Unorm8x4,
}

#[cfg(not(target_arch = "spirv"))]
pub struct TextLayout<'a> {
    text: &'a mut Text,
    shaped: ShapedLine,
}

#[cfg(not(target_arch = "spirv"))]
pub struct TextScope<'a> {
    text: &'a mut Text,
}

#[cfg(not(target_arch = "spirv"))]
impl<'a> TextScope<'a> {
    pub(crate) const fn new(text: &'a mut Text) -> Self {
        Self { text }
    }

    pub fn line(&mut self, content: &str, size: f32, weight: f32) -> TextLayout<'_> {
        TextLayout::new(self.text, content, size, weight)
    }

    pub fn width(&self, content: &str, size: f32, weight: f32) -> f32 {
        self.text.shaper.width(content, size, weight)
    }

    pub fn fit(&mut self, shaped: &ShapedLine, y: f32, bounds: Range<f32>) -> Line {
        self.text.fit_shaped(shaped, y, bounds.start, bounds.end)
    }

    pub fn visible(&mut self, shaped: &ShapedLine, left: Vec2, clip: Range<f32>) -> Line {
        self.text.place_visible(shaped, left, clip)
    }

    pub fn shaper(&self) -> Shaper {
        self.text.shaper.clone()
    }
}

#[cfg(not(target_arch = "spirv"))]
impl<'a> TextLayout<'a> {
    pub(crate) fn new(text: &'a mut Text, content: &str, size: f32, weight: f32) -> Self {
        let shaped = text.shaper.shape(content, size, weight);
        Self { text, shaped }
    }

    pub fn shape(self) -> ShapedLine {
        self.shaped
    }

    pub fn centered(self, center: Vec2) -> Line {
        let origin = vec2(center.x - self.shaped.width * 0.5, center.y + self.shaped.baseline);
        self.text.place(&self.shaped, origin)
    }

    pub fn left(self, position: Vec2) -> Line {
        let baseline = self.shaped.baseline;
        self.text.place(&self.shaped, vec2(position.x, position.y + baseline))
    }

    pub fn right(self, position: Vec2) -> Line {
        let baseline = self.shaped.baseline;
        self.text.place(
            &self.shaped,
            vec2(position.x - self.shaped.width, position.y + baseline),
        )
    }

    pub fn fit(self, y: f32, bounds: Range<f32>) -> Line {
        self.text.fit_shaped(&self.shaped, y, bounds.start, bounds.end)
    }

    pub fn visible(self, left: Vec2, clip: Range<f32>) -> Line {
        self.text.place_visible(&self.shaped, left, clip)
    }
}

#[cfg(not(target_arch = "spirv"))]
impl Text {
    /// Creates the shared vector-font renderer and GPU storage.
    ///
    /// # Panics
    ///
    /// Panics if the supplied variable font cannot be parsed or its outlines cannot be read.
    pub(crate) fn new(context: &Context, canvas: &mut Canvas, font: &[u8], color: Vec3) -> Self {
        let mut face = Face::parse(font, 0).expect("parse variable font");
        let characters = RANGES
            .iter()
            .flat_map(|&(a, b)| a..=b)
            .filter_map(char::from_u32)
            .filter_map(|character| face.glyph_index(character).map(|id| (character, id.0)))
            .collect::<Vec<_>>();
        let mut ids = characters.iter().map(|&(_, id)| id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        let span = f32::from(face.ascender() - face.descender());
        let baseline = f32::from(face.ascender() + face.descender()) * 0.5 / span;
        let mut outlines = Vec::new();
        for weight in [600.0, 900.0] {
            face.set_variation(WGHT, weight).expect("font must have a weight axis");
            outlines.push(
                ids.iter()
                    .map(|&id| {
                        let mut outline = Outline::default();
                        let bounds = face.outline_glyph(GlyphId(id), &mut outline);
                        for edge in &mut outline.edges {
                            *edge = edge.map(|point| point / span);
                        }
                        let (min, max) = bounds.map_or((Vec2::ZERO, Vec2::ZERO), |bounds| {
                            (
                                vec2(f32::from(bounds.x_min), f32::from(bounds.y_min)) / span,
                                vec2(f32::from(bounds.x_max), f32::from(bounds.y_max)) / span,
                            )
                        });
                        (
                            outline.edges,
                            f32::from(face.glyph_hor_advance(GlyphId(id)).unwrap_or(0)) / span,
                            (min, max),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let (mut curves, mut metadata) = (Vec::new(), Vec::new());
        for (index, id) in ids.iter().copied().enumerate() {
            let (low, low_advance, low_bounds) = &outlines[0][index];
            let (high, high_advance, high_bounds) = &outlines[1][index];
            assert_eq!(low.len(), high.len(), "variable outline topology changed");
            let start = curves.len() as u32;
            for (&[a, b, c], &[d, e, f]) in low.iter().zip(high) {
                curves.push(Curve {
                    start: F16x2::new(a),
                    control: F16x2::new(b),
                    end: F16x2::new(c),
                    start_delta: F16x2::new(d - a),
                    control_delta: F16x2::new(e - b),
                    end_delta: F16x2::new(f - c),
                    min: a.min(b).min(c).min(d).min(e).min(f),
                    max: a.max(b).max(c).max(d).max(e).max(f),
                });
            }
            let count = curves.len() - start as usize;
            metadata.push((
                id,
                Meta {
                    glyph: index as u32,
                    data: Glyph {
                        start,
                        count: count as u32,
                        min: low_bounds.0.min(high_bounds.0),
                        max: low_bounds.1.max(high_bounds.1),
                    },
                    advance: [*low_advance, *high_advance],
                },
            ));
        }
        let characters = characters
            .into_iter()
            .filter_map(|(character, id)| {
                metadata
                    .binary_search_by_key(&id, |&(id, _)| id)
                    .ok()
                    .map(|index| (character, metadata[index].1))
            })
            .collect::<Vec<_>>();
        let curves = context.upload(&curves);
        let glyphs = metadata.iter().map(|(_, meta)| meta.data).collect::<Arc<[_]>>();
        canvas.register_text(context.upload(&glyphs), curves);
        Self {
            shaper: Shaper {
                characters: characters.into(),
                baseline,
            },
            glyphs,
            context: context.clone(),
            placed: Vec::with_capacity(GLYPH_CAPACITY),
            color: Unorm8x4::from_vec3(color),
        }
    }

    pub(crate) fn begin_frame(&mut self) {
        self.placed.clear();
    }

    pub(crate) fn finish_frame(&self) -> BufferRange {
        self.context.upload(&self.placed)
    }

    pub(crate) fn quads(&self, line: Line) -> SmallVec<[Quad; 32]> {
        let placed = &self.placed[line.first as usize..(line.first + line.count) as usize];
        let bounds = |placed: PlacedGlyph| {
            let glyph = self.glyphs[placed.glyph as usize];
            vec2(
                line.origin.x + (placed.x + glyph.min.x) * line.size - line.padding,
                line.origin.x + (placed.x + glyph.max.x) * line.size + line.padding,
            )
        };
        placed
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, glyph)| {
                let own = bounds(glyph);
                let left = index.checked_sub(1).map_or(own.x, |previous| {
                    let previous = bounds(placed[previous]);
                    if previous.y < own.x {
                        own.x
                    } else {
                        (previous.y + own.x) * 0.5
                    }
                });
                let right = placed.get(index + 1).map_or(own.y, |&next| {
                    let next = bounds(next);
                    if own.y < next.x { own.y } else { (own.y + next.x) * 0.5 }
                });
                let min = vec2(left.max(line.min.x), line.min.y);
                let max = vec2(right.min(line.max.x), line.max.y);
                (min.x < max.x).then(|| Quad::from_min_max(min, max))
            })
            .collect()
    }

    pub(crate) fn place_visible(&mut self, shaped: &ShapedLine, left: Vec2, clip: Range<f32>) -> Line {
        let origin = vec2(left.x, left.y + shaped.baseline);
        let local = |x| (x - origin.x) / shaped.size;
        let start = shaped
            .glyphs
            .partition_point(|glyph| glyph.x < local(clip.start - EFFECT_PADDING))
            .saturating_sub(1);
        let end = (shaped
            .glyphs
            .partition_point(|glyph| glyph.x <= local(clip.end + EFFECT_PADDING))
            + 1)
        .min(shaped.glyphs.len());
        let mut line = self.place_range(shaped, origin, start..end);
        line.min.x = line.min.x.max(clip.start);
        line.max.x = line.max.x.min(clip.end);
        line
    }

    pub(crate) fn fit_shaped(&mut self, shaped: &ShapedLine, y: f32, left: f32, right: f32) -> Line {
        let x = if shaped.width <= right - left + 0.5 {
            (left + right - shaped.width) * 0.5
        } else {
            left
        };
        self.place(shaped, vec2(x, y + shaped.baseline))
    }

    fn place(&mut self, shaped: &ShapedLine, origin: Vec2) -> Line {
        self.place_range(shaped, origin, 0..shaped.glyphs.len())
    }

    fn place_range(&mut self, shaped: &ShapedLine, origin: Vec2, range: Range<usize>) -> Line {
        let first = self.placed.len();
        let count = range.len();
        let (min, max) = if count == 0 {
            (Vec2::ZERO, Vec2::ZERO)
        } else {
            (
                origin + shaped.min * shaped.size - EFFECT_PADDING,
                origin + shaped.max * shaped.size + EFFECT_PADDING,
            )
        };
        self.placed.extend_from_slice(&shaped.glyphs[range]);
        Line {
            min,
            max,
            origin,
            size: shaped.size,
            weight: shaped.weight,
            count: count as u32,
            first: first as u32,
            color: self.color,
            padding: EFFECT_PADDING,
        }
    }
}

#[cfg(not(target_arch = "spirv"))]
impl Shaper {
    fn glyph(&self, character: char) -> Option<Meta> {
        self.characters
            .binary_search_by_key(&character, |glyph| glyph.0)
            .ok()
            .map(|index| self.characters[index].1)
    }

    pub fn shape(&self, text: &str, size: f32, weight: f32) -> ShapedLine {
        self.shape_positioned([(text, Vec2::ZERO)], size, weight, usize::MAX)
    }

    pub fn width(&self, text: &str, size: f32, weight: f32) -> f32 {
        let weight = normalized_weight(weight);
        text.chars()
            .filter_map(|character| self.glyph(character))
            .map(|meta| meta.advance[0] + (meta.advance[1] - meta.advance[0]) * weight)
            .sum::<f32>()
            * size
    }

    pub fn shape_positioned<'a>(
        &self,
        parts: impl IntoIterator<Item = (&'a str, Vec2)>,
        size: f32,
        font_weight: f32,
        max_glyphs: usize,
    ) -> ShapedLine {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        let weight = normalized_weight(font_weight);
        let mut width: f32 = 0.0;
        let mut glyphs = SmallVec::new();
        for (text, position) in parts {
            let mut x = position.x / size;
            let y = position.y / size;
            for meta in text.chars().filter_map(|character| self.glyph(character)) {
                if glyphs.len() == max_glyphs {
                    break;
                }
                if meta.data.count > 0 {
                    min = min.min(vec2(x + meta.data.min.x, y - meta.data.max.y));
                    max = max.max(vec2(x + meta.data.max.x, y - meta.data.min.y));
                    glyphs.push(PlacedGlyph {
                        x,
                        y,
                        glyph: meta.glyph,
                    });
                }
                x += meta.advance[0] + (meta.advance[1] - meta.advance[0]) * weight;
            }
            width = width.max(x * size);
        }
        ShapedLine {
            glyphs,
            min,
            max,
            width,
            baseline: self.baseline * size,
            size,
            weight,
        }
    }
}
