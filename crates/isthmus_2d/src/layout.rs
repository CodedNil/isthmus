use crate::text::{Curve, FIELD_PADDING, FIELD_RESOLUTION, Glyph, PlacedGlyph, Text, glyph_distance};
use hashbrown::HashMap;
use isthmus::{
    F16x2, ResourceData, Resources, ShaderData,
    glam::{UVec2, Vec2, vec2},
};
use kurbo::{PathEl, PathSeg, Point, QuadBez, Rect, Shape, cubics_to_quadratic_splines, segments};
use skrifa::{
    FontRef, GlyphId, MetadataProvider, Tag,
    instance::{LocationRef, Size},
    outline::{
        DrawSettings,
        pen::{PathElement, PathStyle},
    },
};
use std::{
    borrow::Cow,
    mem,
    sync::{Arc, OnceLock, Weak},
};

impl Curve {
    fn new(quad: QuadBez) -> Self {
        Self { points: [quad.p0, quad.p1, quad.p2].map(|p| F16x2::from_vec2(vec2(p.x as f32, p.y as f32))) }
    }

    fn bounds(self) -> Rect {
        let [a, b, c] = self.points().map(|p| (f64::from(p.x), f64::from(p.y)));
        let bounds = QuadBez::new(a, b, c).bounding_box();
        // Every interpolated curve lies between its endpoint curves; allow arithmetic rounding too.
        let scale = [bounds.x0, bounds.x1, bounds.y0, bounds.y1].into_iter().map(f64::abs).fold(1.0, f64::max);
        bounds.inflate(scale * f64::from(8.0 * f32::EPSILON), scale * f64::from(8.0 * f32::EPSILON))
    }
}

impl Outline {
    fn get(
        glyphs: &mut HashMap<(usize, u32, u32, u32), Weak<Self>>,
        face: &FontRef<'_>,
        weight: f32,
        width: f32,
        span: f32,
        font: usize,
        id: u32,
    ) -> Arc<Self> {
        let key = (font, id, weight.to_bits(), width.to_bits());
        if let Some(glyph) = glyphs.get(&key).and_then(Weak::upgrade) {
            return glyph;
        }
        let mut path = Vec::new();
        if let Some(glyph) = face.outline_glyphs().get(GlyphId::new(id)) {
            let location = face.axes().location([(WGHT, weight), (WDTH, width)]);
            glyph
                .draw(
                    DrawSettings::unhinted(Size::unscaled(), &location).with_path_style(PathStyle::HarfBuzz),
                    &mut path,
                )
                .expect("read font outline");
        }
        let mut curves = Vec::new();
        for segment in outline_segments(path, span) {
            match segment {
                PathSeg::Line(line) => {
                    curves.push(Curve::new(QuadBez::new(line.p0, line.p0.midpoint(line.p1), line.p1)));
                }
                PathSeg::Quad(quad) => curves.push(Curve::new(quad)),
                PathSeg::Cubic(cubic) => {
                    let spline = cubics_to_quadratic_splines(&[cubic], 1e-4).expect("font cubic approximation failed");
                    curves.extend(spline[0].to_quads().map(Curve::new));
                }
            }
        }
        let bounds = curves.iter().copied().map(Curve::bounds).reduce(|a, b| a.union(b)).unwrap_or(Rect::ZERO);
        let glyph = Glyph {
            start: 0,
            count: curves.len() as u32,
            min: vec2(bounds.x0 as f32, bounds.y0 as f32),
            max: vec2(bounds.x1 as f32, bounds.y1 as f32),
            field_start: 0,
            field_size: UVec2::ZERO,
        };
        let mut words = Vec::new();
        for curve in curves {
            curve.append(&mut words);
        }
        let outline =
            Arc::new(Self { data: GlyphData { glyph, words: words.into_boxed_slice() }, field: OnceLock::new() });
        glyphs.insert(key, Arc::downgrade(&outline));
        outline
    }
}

const WGHT: Tag = Tag::new(b"wght");
const WDTH: Tag = Tag::new(b"wdth");

/// CPU runs own exact glyphs; offsets are assigned only when recording a frame.
struct Outline {
    data: GlyphData,
    field: OnceLock<GlyphData>,
}

struct GlyphData {
    glyph: Glyph,
    words: Box<[u32]>,
}

impl TextCache {
    fn place_glyph(&mut self, shaped: &ShapedGlyph, field: bool) -> u32 {
        let outline = &shaped.outline;
        let key = (Arc::as_ptr(outline) as usize, field);
        self.resident
            .entry(key)
            .or_insert_with(|| {
                let base = self.words.len() as u32;
                let data = if field { outline.field() } else { &outline.data };
                self.words.extend_from_slice(&data.words);
                let mut glyph = data.glyph;
                glyph.start += base;
                glyph.field_start += base;
                // Keep the allocation identity alive until this frame ends.
                (Arc::downgrade(outline), glyph.append(&mut self.words))
            })
            .1
    }
}

struct ShapedGlyph {
    placed: PlacedGlyph,
    outline: Arc<Outline>,
}

fn axis_range(face: &FontRef<'_>, tag: Tag, fallback: f32) -> (f32, f32) {
    face.axes()
        .iter()
        .find(|axis| axis.tag() == tag)
        .map_or((fallback, fallback), |axis| (axis.min_value(), axis.max_value()))
}

fn outline_segments(outline: Vec<PathElement>, span: f32) -> impl Iterator<Item = PathSeg> {
    let point = move |x, y| Point::new(f64::from(x) / f64::from(span), f64::from(y) / f64::from(span));
    segments(outline.into_iter().map(move |element| match element {
        PathElement::MoveTo { x, y } => PathEl::MoveTo(point(x, y)),
        PathElement::LineTo { x, y } => PathEl::LineTo(point(x, y)),
        PathElement::QuadTo { cx0, cy0, x, y } => PathEl::QuadTo(point(cx0, cy0), point(x, y)),
        PathElement::CurveTo { cx0, cy0, cx1, cy1, x, y } => {
            PathEl::CurveTo(point(cx0, cy0), point(cx1, cy1), point(x, y))
        }
        PathElement::Close => PathEl::ClosePath,
    }))
}

/// Reusable glyph placement and metrics before positioning on screen.
#[derive(Clone, Default)]
pub struct ShapedLine {
    glyphs: Arc<[ShapedGlyph]>,
    field: bool,
    /// Unplaced text geometry and metrics.
    pub text: Text,
}

type RunCache = HashMap<String, HashMap<(u32, u32, u32), (ShapedLine, bool)>>;

/// Font outlines, cached text runs, and per-frame glyph placements.
pub struct TextCache {
    fonts: Vec<Cow<'static, [u8]>>,
    weight_range: (f32, f32),
    width_range: (f32, f32),
    glyphs: HashMap<(usize, u32, u32, u32), Weak<Outline>>,
    words: Vec<u32>,
    resident: HashMap<(usize, bool), (Weak<Outline>, u32)>,
    runs: RunCache,
    span: f32,
    baseline: f32,
}

impl Resources for TextCache {
    fn data(&self) -> ResourceData<'_> {
        &self.words
    }

    fn begin_frame(&mut self) {
        self.words.clear();
        self.resident.clear();
        self.runs.retain(|_, sizes| {
            sizes.retain(|_, (_, used)| mem::replace(used, false));
            !sizes.is_empty()
        });
        self.glyphs.retain(|_, glyph| glyph.strong_count() != 0);
    }
}

impl Outline {
    fn field(&self) -> &GlyphData {
        self.field.get_or_init(|| {
            let mut glyph = self.data.glyph;
            let mut words = Vec::new();
            let size = ((glyph.max - glyph.min + FIELD_PADDING * 2.0) * FIELD_RESOLUTION).ceil();
            glyph.field_size = UVec2::new(size.x as u32 + 1, size.y as u32 + 1);
            glyph.field_start = 0;
            let count = glyph.field_size.x * glyph.field_size.y;
            for index in (0..count).step_by(2) {
                let sample = |index: u32| {
                    let grid = vec2((index % glyph.field_size.x) as f32, (index / glyph.field_size.x) as f32);
                    glyph_distance(
                        &self.data.words,
                        glyph.start,
                        glyph.count,
                        glyph.min - FIELD_PADDING + grid / FIELD_RESOLUTION,
                    )
                };
                let pair = vec2(sample(index), sample((index + 1).min(count - 1)));
                F16x2::from_vec2(pair).append(&mut words);
            }
            GlyphData { glyph, words: words.into_boxed_slice() }
        })
    }
}

impl ShapedLine {
    /// Uses cached distance fields for small text, materialized on first placement.
    pub fn cached_distance(mut self) -> Self {
        self.field |= self.text.size <= 32.0;
        self
    }
}

impl TextCache {
    /// Prepares text at an ascent-to-descent pixel size and font-axis weight for placement.
    pub fn line(&mut self, content: &str, size: f32, weight: f32) -> Text {
        let shaped = self.shape(content, size, weight);
        self.place(&shaped, Vec2::ZERO)
    }

    /// Uses the first font's metrics and later fonts for missing characters.
    /// Static font bytes are borrowed; dynamic font buffers can be moved in.
    pub fn new(fonts: impl IntoIterator<Item = impl Into<Cow<'static, [u8]>>>) -> Self {
        let fonts: Vec<_> = fonts.into_iter().map(Into::into).collect();
        let faces: Vec<_> = fonts.iter().map(|font| FontRef::new(font).expect("parse font")).collect();
        let metrics = faces.first().expect("at least one font").metrics(Size::unscaled(), LocationRef::default());
        let span = metrics.ascent - metrics.descent;
        let baseline = f32::midpoint(metrics.ascent, metrics.descent) / span;
        let range = |tag, fallback| {
            faces
                .iter()
                .map(|face| axis_range(face, tag, fallback))
                .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
                .unwrap()
        };
        let weight_range = range(WGHT, 400.0);
        let width_range = range(WDTH, 100.0);
        Self {
            fonts,
            weight_range,
            width_range,
            glyphs: HashMap::new(),
            words: Vec::new(),
            resident: HashMap::new(),
            runs: RunCache::default(),
            span: span / f32::from(metrics.units_per_em),
            baseline,
        }
    }

    /// Places a cached run with its left advance edge and vertical center at the position.
    pub fn place(&mut self, shaped: &ShapedLine, position: Vec2) -> Text {
        // Materialize outlines before the contiguous placement records. Only
        // glyphs visible in this frame occupy the GPU resource arena.
        for glyph in &*shaped.glyphs {
            self.place_glyph(glyph, shaped.field);
        }
        let start = self.words.len().next_multiple_of(PlacedGlyph::WORDS);
        self.words.resize(start, 0);
        let first = (start / PlacedGlyph::WORDS) as u32;
        for glyph in &*shaped.glyphs {
            PlacedGlyph { glyph: self.place_glyph(glyph, shaped.field), ..glyph.placed }.append(&mut self.words);
        }
        Text { first, ..shaped.text }.translated(position)
    }

    /// Caches a run using character-to-glyph mapping and advances, without kerning or complex shaping.
    pub fn shape(&mut self, text: &str, size: f32, weight: f32) -> ShapedLine {
        self.shape_width(text, size, weight, 100.0)
    }

    pub fn shape_width(&mut self, text: &str, size: f32, weight: f32, width: f32) -> ShapedLine {
        let key = (size.to_bits(), weight.to_bits(), width.to_bits());
        if let Some((line, used)) = self.runs.get_mut(text).and_then(|sizes| sizes.get_mut(&key)) {
            *used = true;
            return line.clone();
        }
        let line = self.shape_uncached(text, size, weight, width);
        self.runs.entry_ref(text).or_default().insert(key, (line.clone(), true));
        line
    }

    fn shape_uncached(&mut self, text: &str, size: f32, font_weight: f32, font_width: f32) -> ShapedLine {
        if !size.is_finite() || size <= 0.0 {
            return ShapedLine::default();
        }
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        let weight = font_weight.clamp(self.weight_range.0, self.weight_range.1);
        let width_axis = font_width.clamp(self.width_range.0, self.width_range.1);
        let mut glyphs = Vec::new();
        let faces: Vec<_> = self.fonts.iter().map(|font| FontRef::new(font).expect("parse font")).collect();
        let locations: Vec<_> =
            faces.iter().map(|face| face.axes().location([(WGHT, weight), (WDTH, width_axis)])).collect();
        let fonts: Vec<_> = faces
            .iter()
            .zip(&locations)
            .map(|(face, location)| {
                (
                    face.charmap(),
                    face.glyph_metrics(Size::unscaled(), location),
                    self.span * f32::from(face.metrics(Size::unscaled(), LocationRef::default()).units_per_em),
                )
            })
            .collect();
        let mut x = 0.0;
        for character in text.chars().filter(|character| !matches!(character, '\u{fe0e}' | '\u{fe0f}')) {
            let (font, id) = fonts
                .iter()
                .enumerate()
                .find_map(|(index, (charmap, ..))| charmap.map(character).map(|id| (index, id)))
                .unwrap_or_default();
            let (_, metrics, span) = &fonts[font];
            let outline = Outline::get(&mut self.glyphs, &faces[font], weight, width_axis, *span, font, id.to_u32());
            let data = outline.data.glyph;
            if data.count > 0 {
                min = min.min(vec2(x + data.min.x, -data.max.y));
                max = max.max(vec2(x + data.max.x, -data.min.y));
                glyphs.push(ShapedGlyph {
                    placed: PlacedGlyph { x, y: 0.0, glyph: 0, left: x + data.min.x, right: x + data.max.x },
                    outline,
                });
            }
            x += metrics.advance_width(id).unwrap_or(0.0) / span;
        }
        let width = x.max(0.0) * size;
        glyphs.sort_by(|a, b| a.placed.left.total_cmp(&b.placed.left));
        let mut right = f32::NEG_INFINITY;
        for glyph in &mut glyphs {
            right = right.max(glyph.placed.right);
            glyph.placed.right = right;
        }
        ShapedLine {
            text: Text { min: min * size, max: max * size, size, width, count: glyphs.len() as u32, ..Text::default() }
                .translated(vec2(0.0, self.baseline * size)),
            glyphs: glyphs.into(),
            field: false,
        }
    }
}
