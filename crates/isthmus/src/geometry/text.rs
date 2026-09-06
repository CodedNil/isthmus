//! Vector text geometry and rendering.

use super::{FragmentGeometry, Quad, Raster};
#[cfg(target_arch = "spirv")]
use crate::Float as _;
use crate::{
    F16x2, ShaderData, Unorm8x4,
    data::load,
    glam::{Vec2, Vec4, vec2},
};
#[cfg(not(target_arch = "spirv"))]
pub use host::{ShapedLine, TextCache, TextLayout};

#[derive(Clone, Copy, crate::ShaderData)]
#[doc(hidden)]
pub struct Curve {
    points: [F16x2; 3],
}

impl Curve {
    fn points(self) -> [Vec2; 3] {
        [self.points[0].to_vec2(), self.points[1].to_vec2(), self.points[2].to_vec2()]
    }
}

#[cfg(not(target_arch = "spirv"))]
impl Curve {
    fn new(quad: kurbo::QuadBez) -> Self {
        Self { points: [quad.p0, quad.p1, quad.p2].map(|p| F16x2::from_vec2(vec2(p.x as f32, p.y as f32))) }
    }

    fn bounds(self) -> kurbo::Rect {
        use kurbo::{QuadBez, Shape};
        let [a, b, c] = self.points().map(|p| (f64::from(p.x), f64::from(p.y)));
        let bounds = QuadBez::new(a, b, c).bounding_box();
        // Every interpolated curve lies between its endpoint curves; allow arithmetic rounding too.
        let scale = [bounds.x0, bounds.x1, bounds.y0, bounds.y1].into_iter().map(f64::abs).fold(1.0, f64::max);
        bounds.inflate(scale * f64::from(8.0 * f32::EPSILON), scale * f64::from(8.0 * f32::EPSILON))
    }
}

/// A placed text run and its declared rasterization and distance-query bounds.
#[derive(Clone, Copy, Default, crate::ShaderData)]
pub struct Text {
    /// Minimum rasterization corner in logical screen coordinates.
    pub min: Vec2,
    /// Maximum rasterization corner in logical screen coordinates.
    pub max: Vec2,
    /// Baseline origin in logical screen coordinates.
    pub origin: Vec2,
    /// Font ascent-to-descent span in logical pixels.
    pub size: f32,
    weight: f32,
    prepared_weight: Weight,
    weight_count: u32,
    /// Number of placed glyphs in the run.
    pub count: u32,
    /// First glyph in the current frame's placement buffer.
    pub first: u32,
    /// Default straight-alpha text color.
    pub color: Unorm8x4,
    /// Maximum distance queried outside glyphs, including antialiasing space.
    pub effect_radius: f32,
    padding: f32,
}

impl Text {
    /// Returns the prepared font weight; use `distance_with_weight` for shader-side variation.
    pub const fn weight(self) -> f32 {
        self.weight
    }

    #[must_use]
    /// Moves the baseline and rasterization bounds by a logical pixel offset.
    pub fn translated(mut self, offset: Vec2) -> Self {
        self.min += offset;
        self.max += offset;
        self.origin += offset;
        self
    }

    #[must_use]
    /// Sets the straight-alpha text color, quantizing its channels to eight bits.
    pub fn with_color(mut self, color: Vec4) -> Self {
        self.color = Unorm8x4::from_vec4(color);
        self
    }

    /// Reserves an effect's distance-query and displacement reach, including antialiasing.
    #[must_use]
    pub fn with_effect(mut self, effect: impl super::effect::Effect) -> Self {
        let radius = (effect.outset().max(0.0) + 1.0).max(self.effect_radius);
        let padding = radius - self.effect_radius + effect.displacement().max(0.0);
        self.effect_radius = radius;
        self.min -= padding;
        self.max += padding;
        self.padding += padding;
        self
    }
}

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct TextResources<'a> {
    pub placed_glyphs: &'a [u32],
    pub outlines: &'a [u32],
}

/// Glyph distance queries for a placed text run.
#[derive(Clone, Copy)]
pub struct TextSample<'a> {
    /// The placed run being rendered.
    pub line: Text,
    text: TextResources<'a>,
    candidates: [u32; 2],
}

#[derive(Clone, Copy, Default, crate::ShaderData)]
struct Weight {
    masters: [u32; 2],
    blend: f32,
}

impl Weight {
    fn resolve(weights: &[u32], count: u32, weight: f32) -> Self {
        let mut low = 0;
        let last = count.max(1) - 1;
        let mut high = last;
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            if f32::read(weights, middle as usize) <= weight {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let next = (low + 1).min(last);
        let a = f32::read(weights, low as usize);
        let b = f32::read(weights, next as usize);
        let blend = if b > a { ((weight - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        Self { masters: [low, if blend == 0.0 { low } else { next }], blend }
    }
}

/// An axis-aligned text strip carrying its prepared candidate-glyph range.
#[derive(Clone, Copy)]
pub struct TextRaster(Quad);

impl Raster for TextRaster {
    const VERTICES: u32 = 4;

    fn from_data([center, size, _]: [Vec2; 3]) -> Self {
        Self(Quad::new(center, size, Vec2::X))
    }

    fn vertex(self, vertex: u32) -> Vec2 {
        self.0.vertex(vertex)
    }
}

impl<'a> FragmentGeometry<'a> for Text {
    type Payload = Self;
    type Raster = TextRaster;
    type Sample = TextSample<'a>;

    fn sample(_: Vec2, raster: [Vec2; 3], payload: Self, text: TextResources<'a>) -> TextSample<'a> {
        TextSample { line: payload, text, candidates: [raster[2].x.to_bits(), raster[2].y.to_bits()] }
    }
}

impl TextSample<'_> {
    /// Approximate pixel distance, saturated to the line's declared effect radius.
    pub fn distance_with_weight(&self, point: Vec2, weight: f32) -> f32 {
        self.distance(point, Weight::resolve(self.text.outlines, self.line.weight_count, weight))
    }

    fn distance(&self, point: Vec2, weight: Weight) -> f32 {
        let line = self.line;
        if line.count == 0 || line.size <= 0.0 {
            return line.effect_radius.max(1.0);
        }
        let inverse_size = 1.0 / line.size;
        let line_point = (point - line.origin) * inverse_size;
        let padding = line.effect_radius * inverse_size;
        let mut best = line.effect_radius;
        for glyph_index in self.candidates[0]..self.candidates[1] {
            let placed = load::<PlacedGlyph>(self.text.placed_glyphs, glyph_index);
            let glyph = Glyph::read(self.text.outlines, placed.glyph as usize);
            let glyph_point = vec2(line_point.x - placed.x, placed.y - line_point.y);
            if glyph_point.x >= glyph.min.x - padding
                && glyph_point.y >= glyph.min.y - padding
                && glyph_point.x <= glyph.max.x + padding
                && glyph_point.y <= glyph.max.y + padding
            {
                best = best.min(glyph_distance(
                    self.text.outlines,
                    glyph.start,
                    glyph.count,
                    weight.masters,
                    weight.blend,
                    glyph_point,
                    line.size,
                    line.effect_radius,
                ));
            }
        }
        best
    }

    /// Uses the line's weight with the same local-effect limits as `distance_with_weight`.
    pub fn distance_at(&self, point: Vec2) -> f32 {
        self.distance(point, self.line.prepared_weight)
    }

    /// Tests glyph membership at the line's weight, including points on the boundary.
    pub fn contains(&self, point: Vec2) -> bool {
        self.distance_at(point) <= 0.0
    }

    /// Returns antialiased glyph coverage at a logical screen position.
    pub fn fill_at(&self, point: Vec2) -> f32 {
        super::sdf::fill(self.distance_at(point))
    }

    /// Returns an antialiased exterior outline within the line's declared effect radius.
    pub fn outline_at(&self, point: Vec2, width: f32) -> f32 {
        self.fill_outline_at(point, width).1
    }

    /// Returns disjoint fill and exterior-outline masks within the line's declared effect radius.
    pub fn fill_outline_at(&self, point: Vec2, width: f32) -> (f32, f32) {
        super::sdf::fill_outline(self.distance_at(point), width)
    }

    /// Applies coverage to the run's straight-alpha color.
    pub fn color(&self, coverage: f32) -> Vec4 {
        let color = self.line.color.to_vec4();
        color.truncate().extend(coverage * color.w)
    }
}

#[derive(Clone, Copy, crate::ShaderData)]
#[doc(hidden)]
pub struct Glyph {
    min: Vec2,
    max: Vec2,
    start: u32,
    count: u32,
}

#[derive(Clone, Copy, PartialEq, crate::ShaderData)]
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

#[expect(clippy::manual_range_contains, reason = "Range::contains does not lower cleanly through Rust-GPU")]
// Share the winding calculation across text shaders.
#[inline(never)]
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

fn curve_distance([start, control, end]: [Vec2; 3], point: Vec2, best: f32) -> (f32, i32) {
    let min = start.min(control).min(end);
    let max = start.max(control).max(end);
    let winding = if point.y >= min.y && point.y <= max.y { curve_winding(start, control, end, point) } else { 0 };
    if (point - point.clamp(min, max)).length_squared() >= best {
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

fn glyph_distance(
    curves: &[u32],
    start: u32,
    count: u32,
    masters: [u32; 2],
    weight: f32,
    point: Vec2,
    size: f32,
    effect_radius: f32,
) -> f32 {
    let radius = effect_radius / size;
    let mut distance_squared = radius * radius;
    let mut winding = 0;
    // Rust-GPU cannot lower this runtime slice iterator without a pointer-to-integer conversion.
    for index in 0..count {
        let a = Curve::read(curves, start as usize + (masters[0] * count + index) as usize * Curve::WORDS).points();
        let points = if masters[0] == masters[1] {
            a
        } else {
            let b = Curve::read(curves, start as usize + (masters[1] * count + index) as usize * Curve::WORDS).points();
            [a[0].lerp(b[0], weight), a[1].lerp(b[1], weight), a[2].lerp(b[2], weight)]
        };
        let (distance, edge_winding) = curve_distance(points, point, distance_squared);
        distance_squared = distance;
        winding += edge_winding;
    }
    let scaled_squared = distance_squared * size * size;
    let distance = if scaled_squared >= effect_radius * effect_radius { effect_radius } else { scaled_squared.sqrt() };
    distance * if winding == 0 { 1.0 } else { -1.0 }
}

#[cfg(not(target_arch = "spirv"))]
mod host {
    use super::{Curve, Glyph, PlacedGlyph, Text, Weight};
    use crate::{
        Quad, ShaderData, Unorm8x4,
        backend::buffer::UploadBuffer,
        geometry::Geometry,
        glam::{Vec2, Vec3, vec2},
    };
    use kurbo::{PathEl, PathSeg, Point, QuadBez, Rect, cubics_to_quadratic_splines, segments};
    use skrifa::{
        FontRef, GlyphId, MetadataProvider, Tag,
        instance::{LocationRef, Size},
        outline::{
            DrawSettings,
            pen::{PathElement, PathStyle},
        },
    };
    use std::{
        borrow::Borrow,
        cell::RefCell,
        collections::HashMap,
        mem,
        ops::{Range, RangeInclusive},
        sync::Arc,
    };

    const EFFECT_PADDING: f32 = 3.5;

    impl Borrow<()> for TextCache {
        fn borrow(&self) -> &() {
            &()
        }
    }

    impl Geometry for Text {
        type Context = TextCache;
        type Fragment = Self;

        fn payload(self) -> Self {
            self
        }

        fn primitives(self, context: &TextCache) -> impl Iterator<Item = [Vec2; 3]> {
            context.primitives(self)
        }
    }

    const WGHT: Tag = Tag::new(b"wght");

    #[derive(Default)]
    struct Outlines {
        glyphs: HashMap<u32, (u32, Glyph)>,
        words: Vec<u32>,
    }

    fn append<T: ShaderData>(words: &mut Vec<u32>, value: T) -> u32 {
        let offset = u32::try_from(words.len()).expect("font data exceeds u32");
        words.resize(words.len() + T::WORDS, 0);
        value.write(words, offset as usize);
        offset
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
        min: Vec2,
        max: Vec2,
        /// Total advance in logical pixels.
        pub width: f32,
        baseline: f32,
        size: f32,
        weight: f32,
        prepared_weight: Weight,
    }

    type RunCache = HashMap<(String, u32, u32), (Arc<ShapedLine>, bool)>;

    /// Font outlines, cached text runs, and per-frame glyph placements.
    pub struct TextCache {
        font: Box<[u8]>,
        weights: Vec<f32>,
        outlines: RefCell<Outlines>,
        runs: RefCell<RunCache>,
        span: f32,
        baseline: f32,
        pub(crate) placed: Vec<PlacedGlyph>,
        color: Unorm8x4,
    }

    /// A prepared text run awaiting alignment and placement.
    pub struct TextLayout<'a> {
        text: &'a mut TextCache,
        shaped: Arc<ShapedLine>,
    }

    impl TextCache {
        pub(crate) fn begin_frame(&mut self) {
            self.placed.clear();
            self.runs.get_mut().retain(|_, (_, used)| mem::replace(used, false));
        }

        /// Prepares text at an ascent-to-descent pixel size and font-axis weight for placement.
        pub fn line(&mut self, content: &str, size: f32, weight: f32) -> TextLayout<'_> {
            TextLayout { shaped: self.shape(content, size, weight), text: self }
        }
    }

    impl TextLayout<'_> {
        /// Centers the run's advance and font metrics on a logical screen position.
        pub fn centered(self, center: Vec2) -> Text {
            let origin = vec2(center.x - self.shaped.width * 0.5, center.y + self.shaped.baseline);
            self.text.place(&self.shaped, origin)
        }

        /// Aligns the run's right advance edge to x and its vertical center to y.
        pub fn right(self, position: Vec2) -> Text {
            self.text.place(&self.shaped, vec2(position.x - self.shaped.width, position.y + self.shaped.baseline))
        }

        /// Centers text within horizontal bounds if it fits, otherwise aligns left without clipping.
        pub fn fit(self, y: f32, bounds: Range<f32>) -> Text {
            self.text.fit(&self.shaped, y, bounds)
        }

        /// Positions the left advance edge and vertical center, clipping to a horizontal range.
        pub fn visible(self, left: Vec2, clip: Range<f32>) -> Text {
            self.text.visible(&self.shaped, left, clip)
        }
    }

    impl TextCache {
        /// Builds the font's geometry and shaping data independently of the GPU backend.
        ///
        /// # Panics
        ///
        /// Panics if the supplied variable font cannot be parsed or its outlines cannot be read.
        pub fn new(font: &[u8], color: Vec3) -> Self {
            let face = FontRef::new(font).expect("parse font");
            let metrics = face.metrics(Size::unscaled(), LocationRef::default());
            let ascent = metrics.ascent;
            let descent = metrics.descent;
            let span = ascent - descent;
            let baseline = (ascent + descent) * 0.5 / span;
            let weights = weight_locations(&face);
            let mut outlines = Outlines::default();
            for &weight in &weights {
                append(&mut outlines.words, weight);
            }
            Self {
                font: font.into(),
                weights,
                outlines: RefCell::new(outlines),
                runs: RefCell::default(),
                span,
                baseline,
                placed: Vec::new(),
                color: Unorm8x4::from_vec3(color),
            }
        }

        pub(crate) fn upload_outlines(&self, buffer: &mut UploadBuffer, device: &wgpu::Device, queue: &wgpu::Queue) {
            let outlines = self.outlines.borrow();
            buffer.upload_appended(device, queue, &outlines.words);
        }

        pub(crate) fn primitives(&self, line: Text) -> impl Iterator<Item = [Vec2; 3]> {
            let placed = &self.placed[line.first as usize..(line.first + line.count) as usize];
            placed.iter().copied().enumerate().filter_map(move |(index, glyph)| {
                let left = line.origin.x + glyph.left * line.size - line.padding;
                let right =
                    placed.get(index + 1).map_or(line.origin.x + glyph.right * line.size + line.padding, |next| {
                        line.origin.x + next.left * line.size - line.padding
                    });
                let min = vec2(left.max(line.min.x), line.min.y);
                let max = vec2(right.min(line.max.x), line.max.y);
                (min.x < max.x).then(|| {
                    let reach = line.padding / line.size;
                    let first =
                        placed.partition_point(|glyph| glyph.right < (min.x - line.origin.x) / line.size - reach);
                    let end = placed.partition_point(|glyph| glyph.left <= (max.x - line.origin.x) / line.size + reach);
                    let quad = Quad::from_min_max(min, max);
                    [
                        quad.center,
                        quad.size,
                        vec2(f32::from_bits(line.first + first as u32), f32::from_bits(line.first + end as u32)),
                    ]
                })
            })
        }

        /// Places a cached run at its left edge and vertical center, clipping horizontally.
        pub fn visible(&mut self, shaped: &ShapedLine, left: Vec2, clip: Range<f32>) -> Text {
            if clip.start >= clip.end {
                return Text::default();
            }
            // Keep contour data available when effects or displacement extend beyond the initial clip.
            let mut line = self.place(shaped, vec2(left.x, left.y + shaped.baseline));
            line.min.x = line.min.x.max(clip.start);
            line.max.x = line.max.x.min(clip.end);
            line
        }

        /// Centers a cached run if it fits, otherwise aligns left without clipping.
        pub fn fit(&mut self, shaped: &ShapedLine, y: f32, Range { start: left, end: right }: Range<f32>) -> Text {
            let x = if shaped.width <= right - left + 0.5 { (left + right - shaped.width) * 0.5 } else { left };
            self.place(shaped, vec2(x, y + shaped.baseline))
        }

        fn place(&mut self, shaped: &ShapedLine, origin: Vec2) -> Text {
            let first = self.placed.len();
            let count = shaped.glyphs.len();
            let (min, max) = if count == 0 {
                (Vec2::ZERO, Vec2::ZERO)
            } else {
                (origin + shaped.min * shaped.size - EFFECT_PADDING, origin + shaped.max * shaped.size + EFFECT_PADDING)
            };
            self.placed.extend_from_slice(&shaped.glyphs);
            Text {
                min,
                max,
                origin,
                size: shaped.size,
                weight: shaped.weight,
                prepared_weight: shaped.prepared_weight,
                weight_count: self.weights.len() as u32,
                count: count as u32,
                first: first as u32,
                color: self.color,
                effect_radius: EFFECT_PADDING,
                padding: EFFECT_PADDING,
            }
        }
    }

    impl TextCache {
        /// Returns the font's supported weight range, or 400..=400 for a static font.
        pub fn weight_range(&self) -> RangeInclusive<f32> {
            self.weights[0]..=self.weights[self.weights.len() - 1]
        }

        fn glyph(&self, id: u32) -> (u32, Glyph) {
            let mut cache = self.outlines.borrow_mut();
            if let Some(&glyph) = cache.glyphs.get(&id) {
                return glyph;
            }
            let face = FontRef::new(&self.font).expect("parse font");
            let outline = face.outline_glyphs().get(GlyphId::new(id));
            let paths: Vec<Vec<PathSeg>> = self
                .weights
                .iter()
                .map(|&weight| {
                    let mut path = Vec::new();
                    let location = face.axes().location([(WGHT, weight)]);
                    if let Some(glyph) = &outline {
                        glyph
                            .draw(
                                DrawSettings::unhinted(Size::unscaled(), &location)
                                    .with_path_style(PathStyle::HarfBuzz),
                                &mut path,
                            )
                            .expect("read font outline");
                    }
                    outline_segments(path, self.span).collect()
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
                start: cache.words.len() as u32,
                count: masters[0].len() as u32,
                min: vec2(bounds.x0 as f32, bounds.y0 as f32),
                max: vec2(bounds.x1 as f32, bounds.y1 as f32),
            };
            for curve in masters.into_iter().flatten() {
                append(&mut cache.words, curve);
            }
            let offset = append(&mut cache.words, glyph);
            cache.glyphs.insert(id, (offset, glyph));
            (offset, glyph)
        }

        /// Caches a run using character-to-glyph mapping and advances, without kerning or complex shaping.
        pub fn shape(&self, text: &str, size: f32, weight: f32) -> Arc<ShapedLine> {
            let mut runs = self.runs.borrow_mut();
            let (line, used) = runs
                .entry((text.to_owned(), size.to_bits(), weight.to_bits()))
                .or_insert_with(|| (Arc::new(self.shape_positioned([(text, Vec2::ZERO)], size, weight)), true));
            *used = true;
            Arc::clone(line)
        }

        /// Returns the cached run's total advance in logical pixels.
        pub fn width(&self, text: &str, size: f32, weight: f32) -> f32 {
            self.shape(text, size, weight).width
        }

        /// Combines independently positioned text parts into one run using logical pixel offsets.
        /// # Panics
        /// Panics if a font contains incompatible variable outlines.
        pub fn shape_positioned<'a>(
            &self,
            parts: impl IntoIterator<Item = (&'a str, Vec2)>,
            size: f32,
            font_weight: f32,
        ) -> ShapedLine {
            let mut min = Vec2::splat(f32::MAX);
            let mut max = Vec2::splat(f32::MIN);
            let range = self.weight_range();
            let weight = font_weight.clamp(*range.start(), *range.end());
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
                    let (glyph, data) = self.glyph(id.to_u32());
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
                glyphs: glyphs.into_boxed_slice(),
                min,
                max,
                width,
                baseline: self.baseline * size,
                size,
                weight,
                prepared_weight: Weight::resolve(&self.outlines.borrow().words, self.weights.len() as u32, weight),
            }
        }
    }
}
