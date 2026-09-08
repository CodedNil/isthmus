//! CPU font loading, outline preparation, and cached text layout.

use crate::text::{Curve, Glyph, PlacedGlyph, Text, Weight};
use hashbrown::HashMap;
use isthmus::{
    F16x2, ResourceData, Resources, ShaderData,
    glam::{Vec2, vec2},
};
use kurbo::{PathEl, PathSeg, Point, QuadBez, Rect, Shape as _, cubics_to_quadratic_splines, segments};
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
    fn glyph(&mut self, font: &[u8], weights: &[f32], span: f32, id: u32) -> (u32, Glyph) {
        if let Some(&glyph) = self.glyphs.get(&id) {
            return glyph;
        }
        let face = FontRef::new(font).expect("parse font");
        let outline = face.outline_glyphs().get(GlyphId::new(id));
        let paths: Vec<Vec<PathSeg>> = weights
            .iter()
            .map(|&weight| {
                let mut path = Vec::new();
                let location = face.axes().location([(WGHT, weight)]);
                if let Some(glyph) = &outline {
                    glyph
                        .draw(
                            DrawSettings::unhinted(Size::unscaled(), &location).with_path_style(PathStyle::HarfBuzz),
                            &mut path,
                        )
                        .expect("read font outline");
                }
                outline_segments(path, span).collect()
            })
            .collect();
        assert!(paths.iter().all(|path| path.len() == paths[0].len()), "variable outline topology changed");
        let mut masters = vec![Vec::<Curve>::new(); paths.len()];
        for index in 0..paths[0].len() {
            let quadratics: Option<Vec<_>> = paths
                .iter()
                .map(|path| match path[index] {
                    PathSeg::Line(line) => Some(QuadBez::new(line.p0, line.p0.midpoint(line.p1), line.p1)),
                    PathSeg::Quad(quad) => Some(quad),
                    PathSeg::Cubic(_) => None,
                })
                .collect();
            if let Some(quadratics) = quadratics {
                for (master, quad) in masters.iter_mut().zip(quadratics) {
                    master.push(Curve::new(quad));
                }
            } else {
                let cubics: Vec<_> = paths.iter().map(|path| path[index].to_cubic()).collect();
                let splines = cubics_to_quadratic_splines(&cubics, 1e-4).expect("font cubic approximation failed");
                for (master, spline) in masters.iter_mut().zip(splines) {
                    master.extend(spline.to_quads().map(Curve::new));
                }
            }
        }
        let bounds =
            masters.iter().flatten().copied().map(Curve::bounds).reduce(|a, b| a.union(b)).unwrap_or(Rect::ZERO);
        let glyph = Glyph {
            start: self.words.len() as u32,
            count: masters[0].len() as u32,
            min: vec2(bounds.x0 as f32, bounds.y0 as f32),
            max: vec2(bounds.x1 as f32, bounds.y1 as f32),
        };
        for curve in masters.into_iter().flatten() {
            curve.append(&mut self.words);
        }
        let offset = glyph.append(&mut self.words);
        self.glyphs.insert(id, (offset, glyph));
        (offset, glyph)
    }
}

const WGHT: Tag = Tag::new(b"wght");

#[derive(Default)]
struct Outlines {
    glyphs: HashMap<u32, (u32, Glyph)>,
    words: Vec<u32>,
}

fn weight_locations(face: &FontRef<'_>) -> Vec<f32> {
    let Some(axis) = face.axes().iter().find(|axis| axis.tag() == WGHT) else {
        return vec![400.0];
    };
    let mut weights = vec![axis.min_value(), axis.default_value(), axis.max_value()];
    let steps = ((axis.max_value() - axis.min_value()) / 100.0).ceil() as u32;
    weights.extend((1..steps).map(|step| axis.min_value() + step as f32 * 100.0));
    weights.sort_by(f32::total_cmp);
    weights.dedup();
    weights
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

type RunCache = HashMap<String, HashMap<(u32, u32), (Arc<ShapedLine>, bool)>>;

/// Font outlines, cached text runs, and per-frame glyph placements.
pub struct TextCache {
    font: Box<[u8]>,
    weights: Vec<f32>,
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

    /// Builds the font's geometry and shaping data independently of the GPU backend.
    ///
    /// # Panics
    ///
    /// Panics if the supplied variable font cannot be parsed or its outlines cannot be read.
    pub fn new(font: &[u8]) -> Self {
        let face = FontRef::new(font).expect("parse font");
        let metrics = face.metrics(Size::unscaled(), LocationRef::default());
        let ascent = metrics.ascent;
        let descent = metrics.descent;
        let span = ascent - descent;
        let baseline = (ascent + descent) * 0.5 / span;
        let weights = weight_locations(&face);
        let mut outlines = Outlines::default();
        (weights.len() as u32).append(&mut outlines.words);
        for &weight in &weights {
            weight.append(&mut outlines.words);
        }
        Self { font: font.into(), weights, outlines, runs: RunCache::default(), span, baseline, placed: Vec::new() }
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
        let key = (size.to_bits(), weight.to_bits());
        if let Some((line, used)) = self.runs.get_mut(text).and_then(|sizes| sizes.get_mut(&key)) {
            *used = true;
            return Arc::clone(line);
        }
        let line = Arc::new(self.shape_positioned([(text, Vec2::ZERO)], size, weight));
        self.runs.entry_ref(text).or_default().insert(key, (Arc::clone(&line), true));
        line
    }

    /// Combines independently positioned text parts into one run using logical pixel offsets.
    /// # Panics
    /// Panics if a font contains incompatible variable outlines.
    pub fn shape_positioned<'a>(
        &mut self,
        parts: impl IntoIterator<Item = (&'a str, Vec2)>,
        size: f32,
        font_weight: f32,
    ) -> ShapedLine {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        let weight = font_weight.clamp(self.weights[0], self.weights[self.weights.len() - 1]);
        let mut width: f32 = 0.0;
        let mut glyphs = Vec::new();
        if !size.is_finite() || size <= 0.0 {
            return ShapedLine::default();
        }
        let face = FontRef::new(&self.font).expect("parse font");
        let location = face.axes().location([(WGHT, weight)]);
        let metrics = face.glyph_metrics(Size::unscaled(), &location);
        let charmap = face.charmap();
        for (text, position) in parts {
            let mut x = position.x / size;
            let y = position.y / size;
            for character in text.chars() {
                let id = charmap.map(character).unwrap_or_default();
                let (glyph, data) = self.outlines.glyph(&self.font, &self.weights, self.span, id.to_u32());
                if data.count > 0 {
                    min = min.min(vec2(x + data.min.x, y - data.max.y));
                    max = max.max(vec2(x + data.max.x, y - data.min.y));
                    glyphs.push(PlacedGlyph { x, y: -y, glyph, left: x + data.min.x, right: x + data.max.x });
                }
                x += metrics.advance_width(id).unwrap_or(0.0) / self.span;
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
            text: Text {
                min: min * size,
                max: max * size,
                size,
                width,
                prepared_weight: Weight::resolve(&self.outlines.words, weight),
                count: glyphs.len() as u32,
                ..Text::default()
            }
            .translated(vec2(0.0, self.baseline * size)),
            glyphs: glyphs.into_boxed_slice(),
        }
    }
}
