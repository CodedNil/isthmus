//! Glyph outlines read from font files and converted to quadratic curves and distance fields.
use super::glyph::{Curve, FIELD_PADDING, FIELD_RESOLUTION, Glyph, glyph_distance};
use hashbrown::HashMap;
use isthmus::{
    F16x2, ShaderData,
    glam::{UVec2, vec2},
};
use kurbo::{PathEl, PathSeg, Point, QuadBez, Rect, Shape as _, cubics_to_quadratic_splines, segments};
use skrifa::{
    FontRef, GlyphId, MetadataProvider, Tag,
    instance::Size,
    outline::{
        DrawSettings,
        pen::{PathElement, PathStyle},
    },
};
use std::sync::{Arc, OnceLock, Weak};

pub(super) const WGHT: Tag = Tag::new(b"wght");
pub(super) const WDTH: Tag = Tag::new(b"wdth");

/// CPU runs own exact glyphs; offsets are assigned only when recording a frame.
pub(super) struct Outline {
    pub(super) data: GlyphData,
    field: OnceLock<GlyphData>,
}

pub(super) struct GlyphData {
    pub(super) glyph: Glyph,
    pub(super) words: Box<[u32]>,
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

impl Outline {
    pub(super) fn get(
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

impl Outline {
    pub(super) fn field(&self) -> &GlyphData {
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
