use crate::text::{Curve, Glyph, PlacedGlyph, Text};
use hashbrown::HashMap;
use isthmus::{
    F16x2, ResourceData, Resources, ShaderData,
    glam::{Vec2, vec2},
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
use std::{mem, sync::Arc};

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

impl Outlines {
    fn glyph(&mut self, face: &FontRef<'_>, weight: f32, width: f32, span: f32, font: usize, id: u32) -> (u32, Glyph) {
        let key = (font, id, weight.to_bits(), width.to_bits());
        if let Some(&glyph) = self.glyphs.get(&key) {
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
        let path: Vec<_> = outline_segments(path, span).collect();
        let mut curves = Vec::new();
        for segment in path {
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
            start: self.words.len() as u32,
            count: curves.len() as u32,
            min: vec2(bounds.x0 as f32, bounds.y0 as f32),
            max: vec2(bounds.x1 as f32, bounds.y1 as f32),
        };
        for curve in curves {
            curve.append(&mut self.words);
        }
        let offset = glyph.append(&mut self.words);
        self.glyphs.insert(key, (offset, glyph));
        (offset, glyph)
    }
}

const WGHT: Tag = Tag::new(b"wght");
const WDTH: Tag = Tag::new(b"wdth");

#[derive(Default)]
struct Outlines {
    glyphs: HashMap<(usize, u32, u32, u32), (u32, Glyph)>,
    words: Vec<u32>,
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
#[derive(Default)]
pub struct ShapedLine {
    glyphs: Box<[PlacedGlyph]>,
    /// Unplaced text geometry and metrics.
    pub text: Text,
}

type RunCache = HashMap<String, HashMap<(u32, u32, u32), (Arc<ShapedLine>, bool)>>;

/// Font outlines, cached text runs, and per-frame glyph placements.
pub struct TextCache {
    fonts: Vec<Box<[u8]>>,
    weight_range: (f32, f32),
    width_range: (f32, f32),
    outlines: Outlines,
    runs: RunCache,
    span: f32,
    baseline: f32,
    placed: Vec<u32>,
}

impl Resources for TextCache {
    fn data(&self) -> ResourceData<'_> {
        ResourceData { transient: &self.placed, persistent: &self.outlines.words }
    }

    fn begin_frame(&mut self) {
        self.placed.clear();
        self.runs.retain(|_, sizes| {
            sizes.retain(|_, (_, used)| mem::replace(used, false));
            !sizes.is_empty()
        });
    }
}

impl TextCache {
    /// Prepares text at an ascent-to-descent pixel size and font-axis weight for placement.
    pub fn line(&mut self, content: &str, size: f32, weight: f32) -> Text {
        let shaped = self.shape(content, size, weight);
        self.place(&shaped, Vec2::ZERO)
    }

    /// Uses the first font's line metrics and later fonts for missing characters.
    pub fn new(fonts: &[&[u8]]) -> Self {
        let faces: Vec<_> = fonts.iter().map(|font| FontRef::new(font).expect("parse font")).collect();
        let metrics = faces.first().expect("at least one font").metrics(Size::unscaled(), LocationRef::default());
        let span = metrics.ascent - metrics.descent;
        let baseline = f32::midpoint(metrics.ascent, metrics.descent) / span;
        let weight_range =
            faces.iter().map(|face| axis_range(face, WGHT, 400.0)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap();
        let width_range =
            faces.iter().map(|face| axis_range(face, WDTH, 100.0)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap();
        let outlines = Outlines::default();
        Self {
            fonts: fonts.iter().map(|font| (*font).into()).collect(),
            weight_range,
            width_range,
            outlines,
            runs: RunCache::default(),
            span: span / f32::from(metrics.units_per_em),
            baseline,
            placed: Vec::new(),
        }
    }

    /// Places a cached run with its left advance edge and vertical center at the position.
    pub fn place(&mut self, shaped: &ShapedLine, position: Vec2) -> Text {
        let first = (self.placed.len() / PlacedGlyph::WORDS) as u32;
        for &glyph in &shaped.glyphs {
            glyph.append(&mut self.placed);
        }
        Text { first, ..shaped.text }.translated(position)
    }

    /// Caches a run using character-to-glyph mapping and advances, without kerning or complex shaping.
    pub fn shape(&mut self, text: &str, size: f32, weight: f32) -> Arc<ShapedLine> {
        self.shape_width(text, size, weight, 100.0)
    }

    pub fn shape_width(&mut self, text: &str, size: f32, weight: f32, width: f32) -> Arc<ShapedLine> {
        let key = (size.to_bits(), weight.to_bits(), width.to_bits());
        if let Some((line, used)) = self.runs.get_mut(text).and_then(|sizes| sizes.get_mut(&key)) {
            *used = true;
            return Arc::clone(line);
        }
        let line = Arc::new(self.shape_positioned_width([(text, Vec2::ZERO)], size, weight, width));
        self.runs.entry_ref(text).or_default().insert(key, (Arc::clone(&line), true));
        line
    }

    /// Combines independently positioned text parts into one run using logical pixel offsets.
    fn shape_positioned_width<'a>(
        &mut self,
        parts: impl IntoIterator<Item = (&'a str, Vec2)>,
        size: f32,
        font_weight: f32,
        font_width: f32,
    ) -> ShapedLine {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        let weight = font_weight.clamp(self.weight_range.0, self.weight_range.1);
        let width_axis = font_width.clamp(self.width_range.0, self.width_range.1);
        let mut width: f32 = 0.0;
        let mut glyphs = Vec::new();
        if !size.is_finite() || size <= 0.0 {
            return ShapedLine::default();
        }
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
        for (text, position) in parts {
            let mut x = position.x / size;
            let y = position.y / size;
            for character in text.chars() {
                if matches!(character, '\u{fe0e}' | '\u{fe0f}') {
                    continue;
                }
                let (font, id) = fonts
                    .iter()
                    .enumerate()
                    .find_map(|(index, (charmap, ..))| charmap.map(character).map(|id| (index, id)))
                    .unwrap_or_default();
                let (_, metrics, span) = &fonts[font];
                let (glyph, data) = self.outlines.glyph(&faces[font], weight, width_axis, *span, font, id.to_u32());
                if data.count > 0 {
                    min = min.min(vec2(x + data.min.x, y - data.max.y));
                    max = max.max(vec2(x + data.max.x, y - data.min.y));
                    glyphs.push(PlacedGlyph { x, y: -y, glyph, left: x + data.min.x, right: x + data.max.x });
                }
                x += metrics.advance_width(id).unwrap_or(0.0) / span;
            }
            width = width.max(x * size);
        }
        glyphs.sort_by(|a, b| a.left.total_cmp(&b.left));
        let mut right = f32::NEG_INFINITY;
        for glyph in &mut glyphs {
            right = right.max(glyph.right);
            glyph.right = right;
        }
        ShapedLine {
            text: Text { min: min * size, max: max * size, size, width, count: glyphs.len() as u32, ..Text::default() }
                .translated(vec2(0.0, self.baseline * size)),
            glyphs: glyphs.into_boxed_slice(),
        }
    }
}
