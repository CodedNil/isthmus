//! Glyph outlines read from font files and converted to quadratic curves and distance fields.
use super::glyph::{Curve, FIELD_PADDING, FIELD_RESOLUTION, Glyph, glyph_distance};
use hashbrown::HashMap;
use isthmus::{
    F16x2, ShaderData,
    glam::{UVec2, vec2},
};
use kurbo::{BezPath, PathSeg, Point, QuadBez, Rect, Shape as _, cubics_to_quadratic_splines};
use skrifa::{DrawMemory, GlyphId, PathSink, ScalerFont, Tag};
use std::sync::{
    Arc, OnceLock, Weak,
    atomic::{AtomicUsize, Ordering},
};

pub(super) const WGHT: Tag = Tag::new(b"wght");
pub(super) const WDTH: Tag = Tag::new(b"wdth");

/// Which outline: the font, the glyph, and the weight and width the font was instanced at.
pub(super) type OutlineKey = (usize, u32, u32, u32);

/// Gives each outline, and each line of text, a number no other has, to tell them apart within a frame.
pub(super) fn next_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A glyph's words as the shader reads them, which are the same wherever and whenever they are written.
pub(super) struct Outline {
    /// Distinguishes this outline from every other, even after one is freed.
    pub(super) id: usize,
    pub(super) glyph: Glyph,
    /// The glyph header followed by its curves.
    curves: Box<[u32]>,
    /// The header followed by distance field samples, built when first wanted.
    field: OnceLock<Box<[u32]>>,
}

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

/// Collects a glyph's contours in units of the font's ascent-to-descent span.
struct Contours {
    path: BezPath,
    span: f64,
}

impl Contours {
    fn point(&self, x: f32, y: f32) -> Point {
        Point::new(f64::from(x) / self.span, f64::from(y) / self.span)
    }
}

impl PathSink for Contours {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(self.point(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.point(x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to(self.point(cx0, cy0), self.point(x, y));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

impl Outline {
    /// The outline of a glyph, drawn at the scaler's font instance with `span` font units to the unit.
    pub(super) fn get(
        glyphs: &mut HashMap<OutlineKey, Weak<Self>>,
        key: OutlineKey,
        scaler: &ScalerFont,
        span: f32,
    ) -> Arc<Self> {
        if let Some(glyph) = glyphs.get(&key).and_then(Weak::upgrade) {
            return glyph;
        }
        let mut contours = Contours { path: BezPath::new(), span: f64::from(span) };
        if let Some(outline) = scaler.outline(GlyphId::new(key.1)) {
            outline.draw(DrawMemory::Scratch, &mut contours).expect("read font outline");
        }
        let mut curves = Vec::new();
        for segment in contours.path.segments() {
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
            count: curves.len() as u32,
            min: vec2(bounds.x0 as f32, bounds.y0 as f32),
            max: vec2(bounds.x1 as f32, bounds.y1 as f32),
            field_size: UVec2::ZERO,
        };
        let mut words = Vec::new();
        glyph.append(&mut words);
        for curve in curves {
            curve.append(&mut words);
        }
        let outline = Arc::new(Self { id: next_id(), glyph, curves: words.into(), field: OnceLock::new() });
        glyphs.insert(key, Arc::downgrade(&outline));
        outline
    }

    /// The words to write for the glyph: its curves, or its distance field, which is sampled once and kept.
    pub(super) fn words(&self, field: bool) -> &[u32] {
        if !field {
            return &self.curves;
        }
        self.field.get_or_init(|| {
            let glyph = self.glyph;
            let size = ((glyph.max - glyph.min + FIELD_PADDING * 2.0) * FIELD_RESOLUTION).ceil();
            let glyph = Glyph { field_size: UVec2::new(size.x as u32 + 1, size.y as u32 + 1), ..glyph };
            let count = glyph.field_size.x * glyph.field_size.y;
            let mut words = Vec::new();
            glyph.append(&mut words);
            for index in (0..count).step_by(2) {
                let sample = |index: u32| {
                    let grid = vec2((index % glyph.field_size.x) as f32, (index / glyph.field_size.x) as f32);
                    let point = glyph.min - FIELD_PADDING + grid / FIELD_RESOLUTION;
                    glyph_distance(&self.curves, Glyph::WORDS as u32, glyph.count, point)
                };
                let pair = vec2(sample(index), sample((index + 1).min(count - 1)));
                F16x2::from_vec2(pair).append(&mut words);
            }
            words.into()
        })
    }
}
