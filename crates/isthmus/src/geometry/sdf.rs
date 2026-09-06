use super::{GeometrySample, Quad, text::TextResources};
use crate::{
    glam::{FloatExt, Vec2, Vec4, vec2},
    spirv_std::arch::Derivative,
};
use core::{
    f32::consts::TAU,
    ops::{Add, Neg, Sub},
};

/// A bounded distance field; CSG preserves membership but may approximate interior distance.
pub trait SdfShape: crate::ShaderData {
    /// Encloses every point with distance at most `outset`; `None` means the region is empty.
    fn bounds(self, outset: f32) -> Option<Quad>;
    /// Evaluates the signed distance at a logical screen position.
    fn distance_at(self, point: Vec2) -> Sdf;

    /// Tests membership, including points on the boundary.
    fn contains(self, point: Vec2) -> bool {
        self.distance_at(point).distance <= 0.0
    }
}

/// A rectangle with a clamped corner radius; zero gives a rectangle and maximum gives a pill.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct RoundedRect {
    /// Rectangle enclosing the rounded shape.
    pub quad: Quad,
    /// Corner radius, between zero and half the rectangle's shortest side.
    pub radius: f32,
}

impl SdfShape for RoundedRect {
    fn bounds(self, outset: f32) -> Option<Quad> {
        let quad = self.quad.expanded(outset);
        (quad.size.min_element() >= 0.0).then_some(quad)
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        Sdf::rounded_box(self.quad.local(point), self.quad.size * 0.5, self.radius)
    }
}

/// A disk with an exact signed distance inside and outside.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Circle {
    /// Center in logical pixels.
    pub center: Vec2,
    /// Nonnegative radius in logical pixels.
    pub radius: f32,
}

impl SdfShape for Circle {
    fn bounds(self, outset: f32) -> Option<Quad> {
        let radius = self.radius + outset;
        (radius >= 0.0).then_some(Quad::new(self.center, Vec2::splat(radius * 2.0), Vec2::X))
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        Sdf::new((point - self.center).length() - self.radius)
    }
}

/// Creates a rounded rectangle, clamping the radius to fit its dimensions.
pub fn rounded_rect(quad: impl Into<Quad>, radius: f32) -> Shape<RoundedRect> {
    let quad = quad.into();
    Shape::new(RoundedRect { quad, radius: radius.clamp(0.0, (quad.size.min_element() * 0.5).max(0.0)) })
}

/// Creates a rectangle with sharp corners.
pub fn rectangle(quad: impl Into<Quad>) -> Shape<RoundedRect> {
    rounded_rect(quad, 0.0)
}

/// A round-ended segment fitted to a quad, including vertical pills and circles.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Capsule {
    /// Rectangle enclosing the capsule along its local axes.
    pub quad: Quad,
}

impl SdfShape for Capsule {
    fn bounds(self, outset: f32) -> Option<Quad> {
        let quad = self.quad.expanded(outset);
        (quad.size.min_element() >= 0.0).then_some(quad)
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        let half_size = self.quad.size * 0.5;
        let radius = half_size.min_element();
        Sdf::new((self.quad.local(point).abs() - half_size + radius).max(Vec2::ZERO).length() - radius)
    }
}

/// Fits a capsule to a rectangle, using its shorter side as the diameter.
pub fn pill(quad: impl Into<Quad>) -> Shape<Capsule> {
    Shape::new(Capsule { quad: quad.into() })
}

/// A round-ended stroke between two endpoints; coincident endpoints produce a circle.
pub fn segment(start: Vec2, end: Vec2, radius: f32) -> Shape<Capsule> {
    let direction = end - start;
    let length = direction.length();
    let diameter = radius.max(0.0) * 2.0;
    pill(Quad::new(
        start.midpoint(end),
        vec2(length + diameter, diameter),
        if length > 0.0 { direction / length } else { Vec2::X },
    ))
}

/// Creates a disk, clamping negative radii to zero.
pub const fn circle(center: Vec2, radius: f32) -> Shape<Circle> {
    Shape::new(Circle { center, radius: radius.max(0.0) })
}

/// A circular centerline swept from `start` through `sweep` radians; use `stroke` for thickness.
pub fn arc(center: Vec2, radius: f32, start: f32, sweep: f32) -> Shape<Arc> {
    let sweep = sweep.clamp(-TAU, TAU);
    Shape::new(Arc {
        center,
        radius: radius.max(0.0),
        axis: Vec2::from_angle(start + sweep * 0.5),
        edge: Vec2::from_angle(sweep.abs() * 0.5),
    })
}

/// An unsigned circular arc centerline, created with [`arc`].
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Arc {
    /// Center of the arc's circle in logical pixels.
    pub center: Vec2,
    /// Nonnegative radius of the arc's circle.
    pub radius: f32,
    axis: Vec2,
    edge: Vec2,
}

impl SdfShape for Arc {
    fn bounds(self, outset: f32) -> Option<Quad> {
        let height = if self.edge.x >= 0.0 { self.edge.y } else { 1.0 };
        (outset >= 0.0).then_some(
            Quad::new(
                self.center + self.axis * (self.radius * (1.0 + self.edge.x) * 0.5),
                vec2(1.0 - self.edge.x, height * 2.0) * self.radius,
                self.axis,
            )
            .expanded(outset),
        )
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        let offset = point - self.center;
        let local = vec2(offset.dot(self.axis), offset.dot(self.axis.perp()).abs());
        let length = local.length();
        Sdf::new(if local.x >= self.edge.x * length {
            (length - self.radius).abs()
        } else {
            (local - self.edge * self.radius).length()
        })
    }
}

/// Rasterizes a shape's own bounds while preserving its unexpanded shader payload.
#[derive(Clone, Copy)]
#[must_use]
pub struct Shape<S> {
    /// Distance field passed to the shader independently of rasterization margins.
    pub shape: S,
    margin: f32,
}

impl<S: SdfShape> Shape<S> {
    /// Prepares a distance field for painting with one pixel of antialiasing margin.
    pub const fn new(shape: S) -> Self {
        Self { shape, margin: 1.0 }
    }

    /// Includes points inside either shape and encloses both shapes' bounds.
    pub const fn union<T: SdfShape>(self, other: Shape<T>) -> Shape<Union<S, T>> {
        Shape { shape: Union { a: self.shape, b: other.shape }, margin: self.margin.max(other.margin) }
    }

    /// Keeps only points inside both shapes and intersects their bounds.
    pub const fn intersection<T: SdfShape>(self, other: Shape<T>) -> Shape<Intersection<S, T>> {
        Shape { shape: Intersection { a: self.shape, b: other.shape }, margin: self.margin.max(other.margin) }
    }

    /// Cuts the other shape out of this one, retaining this shape's bounds.
    pub const fn difference<T: SdfShape>(self, other: Shape<T>) -> Shape<Difference<S, T>> {
        Shape { shape: Difference { a: self.shape, b: other.shape }, margin: self.margin.max(other.margin) }
    }

    /// Blends from this shape to the rounded union; `amount` is clamped to 0..=1.
    pub const fn smooth_union<T: SdfShape>(
        self,
        other: Shape<T>,
        radius: f32,
        amount: f32,
    ) -> Shape<SmoothUnion<S, T>> {
        Shape {
            shape: SmoothUnion {
                base: self.shape,
                other: other.shape,
                radius: radius.max(0.0),
                amount: amount.clamp(0.0, 1.0),
            },
            margin: self.margin.max(other.margin),
        }
    }

    /// Expands the distance field; negative amounts erode it.
    pub const fn offset(self, amount: f32) -> Shape<Offset<S>> {
        Shape { shape: Offset { shape: self.shape, amount }, margin: self.margin }
    }

    /// Forms a band around the zero contour, including round caps for open curves.
    pub const fn stroke(self, half_width: f32) -> Shape<Stroke<S>> {
        Shape { shape: Stroke { shape: self.shape, half_width: half_width.max(0.0) }, margin: self.margin }
    }

    /// Moves the distance field and its bounds by a logical pixel offset.
    pub const fn translated(self, offset: Vec2) -> Shape<Translated<S>> {
        Shape { shape: Translated { shape: self.shape, offset }, margin: self.margin }
    }

    /// Reserves an outward effect radius plus one antialiasing pixel.
    pub fn effects(mut self, radius: f32) -> Self {
        self.margin = self.margin.max(radius.max(0.0) + 1.0);
        self
    }
}

macro_rules! binary_shape {
    ($name:ident, $operation:ident, |$this:ident, $outset:ident| $bounds:block) => {
        #[doc = concat!("A bounded `", stringify!($operation), "` of two distance fields.")]
        #[derive(Clone, Copy, crate::ShaderData)]
        pub struct $name<A, B> {
            /// First operand, whose region is retained by difference.
            pub a: A,
            /// Second operand, whose region is removed by difference.
            pub b: B,
        }

        impl<A: SdfShape, B: SdfShape> SdfShape for $name<A, B> {
            fn bounds($this, $outset: f32) -> Option<Quad> $bounds

            fn distance_at(self, point: Vec2) -> Sdf {
                self.a.distance_at(point).$operation(self.b.distance_at(point))
            }
        }
    };
}

binary_shape!(Union, union, |self, outset| { enclosing(self.a.bounds(outset), self.b.bounds(outset)) });

binary_shape!(Intersection, intersection, |self, outset| {
    let (a_min, a_max) = extents(self.a.bounds(outset)?);
    let (b_min, b_max) = extents(self.b.bounds(outset)?);
    let min = a_min.max(b_min);
    let max = a_max.min(b_max);
    min.cmple(max).all().then_some(Quad::from_min_max(min, max))
});

binary_shape!(Difference, difference, |self, outset| { self.a.bounds(outset) });

/// A bounded polynomial smooth union blended from its base shape.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct SmoothUnion<A, B> {
    /// Shape present when the blend amount is zero.
    pub base: A,
    /// Shape joined as the blend amount increases.
    pub other: B,
    /// Nonnegative smoothing radius in logical pixels.
    pub radius: f32,
    /// Blend amount from zero to one.
    pub amount: f32,
}

impl<A: SdfShape, B: SdfShape> SdfShape for SmoothUnion<A, B> {
    fn bounds(self, outset: f32) -> Option<Quad> {
        if self.amount == 0.0 {
            return self.base.bounds(outset);
        }
        // Polynomial smoothing subtracts at most radius * amount / 4 from the minimum distance.
        let outset = outset + self.radius * self.amount * 0.25;
        enclosing(self.base.bounds(outset), self.other.bounds(outset))
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        let base = self.base.distance_at(point);
        if self.amount == 0.0 {
            base
        } else {
            base.smooth_union(self.other.distance_at(point), self.radius, self.amount)
        }
    }
}

/// A distance field expanded or eroded by a constant amount.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Offset<S> {
    /// Original distance field.
    pub shape: S,
    /// Expansion distance; negative values erode the field.
    pub amount: f32,
}

impl<S: SdfShape> SdfShape for Offset<S> {
    fn bounds(self, outset: f32) -> Option<Quad> {
        self.shape.bounds(outset + self.amount)
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        self.shape.distance_at(point) - self.amount
    }
}

/// A distance field shifted in logical screen coordinates.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Translated<S> {
    /// Original distance field.
    pub shape: S,
    /// Translation in logical pixels.
    pub offset: Vec2,
}

/// A band centered on a distance field's zero contour.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Stroke<S> {
    /// Distance field whose contour is stroked.
    pub shape: S,
    /// Nonnegative thickness on each side of the contour.
    pub half_width: f32,
}

impl<S: SdfShape> SdfShape for Stroke<S> {
    fn bounds(self, outset: f32) -> Option<Quad> {
        let reach = self.half_width + outset;
        if reach < 0.0 { None } else { self.shape.bounds(reach) }
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        Sdf::new(self.shape.distance_at(point).distance.abs() - self.half_width)
    }
}

impl<S: SdfShape> SdfShape for Translated<S> {
    fn bounds(self, outset: f32) -> Option<Quad> {
        self.shape.bounds(outset).map(|mut quad| {
            quad.center += self.offset;
            quad
        })
    }

    fn distance_at(self, point: Vec2) -> Sdf {
        self.shape.distance_at(point - self.offset)
    }
}

fn extents(quad: Quad) -> (Vec2, Vec2) {
    let half_size = (quad.axis.abs() * quad.size.x + quad.axis.perp().abs() * quad.size.y) * 0.5;
    (quad.center - half_size, quad.center + half_size)
}

fn enclosing(a: Option<Quad>, b: Option<Quad>) -> Option<Quad> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (a_min, a_max) = extents(a);
            let (b_min, b_max) = extents(b);
            Some(Quad::from_min_max(a_min.min(b_min), a_max.max(b_max)))
        }
        (a, b) => a.or(b),
    }
}

impl<S: SdfShape> GeometrySample<'_> for S {
    type Payload = Self;
    type Raster = Quad;

    fn sample(_: Vec2, _: [Vec2; 3], payload: Self, _: TextResources<'_>) -> Self {
        payload
    }
}

#[cfg(not(target_arch = "spirv"))]
impl<S: SdfShape> super::Geometry for Shape<S> {
    type Context = ();
    type Sample = S;

    fn payload(self) -> S {
        self.shape
    }

    fn primitives(self, (): &()) -> impl Iterator<Item = [Vec2; 3]> {
        self.shape.bounds(self.margin).into_iter().map(Quad::data)
    }
}

/// A signed distance at one point, negative inside the shape.
#[derive(Clone, Copy)]
#[must_use]
pub struct Sdf {
    /// Signed distance in the shape's coordinate units, negative inside.
    pub distance: f32,
}

impl Sdf {
    /// Evaluates a centered rounded rectangle with a radius no larger than either half-size.
    pub fn rounded_box(point: Vec2, half_size: Vec2, radius: f32) -> Self {
        let corner = point.abs() - half_size + radius;
        Self::new(corner.max(Vec2::ZERO).length() + corner.x.max(corner.y).min(0.0) - radius)
    }

    /// Evaluates a horizontal capsule with endpoints at `±half_span` and the given radius.
    pub fn capsule(point: Vec2, half_span: f32, radius: f32) -> Self {
        Self::new((point - vec2(point.x.clamp(-half_span, half_span), 0.0)).length() - radius)
    }

    /// Evaluates a five-pointed star; `indent` controls the inner vertices relative to the radius.
    pub fn star(point: Vec2, radius: f32, indent: f32) -> Self {
        let k1 = vec2(0.809_017, -0.587_785_25);
        let k2 = vec2(-k1.x, k1.y);
        let mut point = vec2(point.x.abs(), -point.y);
        point -= 2.0 * k1.dot(point).max(0.0) * k1;
        point -= 2.0 * k2.dot(point).max(0.0) * k2;
        point.x = point.x.abs();
        point.y -= radius;
        let edge = indent * vec2(-k1.y, k1.x) - vec2(0.0, radius);
        let edge_t = (point.dot(edge) / edge.length_squared()).saturate();
        let cross = point.y * edge.x - point.x * edge.y;
        Self::new((point - edge * edge_t).length() * if cross < 0.0 { -1.0 } else { 1.0 })
    }

    /// Evaluates a rounded equilateral triangle pointing toward negative y.
    pub fn rounded_triangle(point: Vec2, side_len: f32, radius: f32) -> Self {
        let k = 1.732_050_8;
        let mut point = vec2(point.x.abs(), point.y);
        let h = (point.x + k * point.y).max(0.0);
        point -= 0.5 * vec2(h, h * k);
        point -= vec2(
            point.x.clamp(-0.5 * (side_len - radius) * k, 0.5 * (side_len - radius) * k),
            -0.5 * (side_len - radius),
        );
        Self::new(point.length() * if point.y > 0.0 { -1.0 } else { 1.0 } - radius)
    }

    /// Shortest distance from `point` to the line segment between `start` and `end`.
    pub fn segment(point: Vec2, start: Vec2, end: Vec2) -> Self {
        let segment = end - start;
        let length_squared = segment.length_squared();
        let along = if length_squared > 0.0 { ((point - start).dot(segment) / length_squared).saturate() } else { 0.0 };
        Self::new((point - start - segment * along).length())
    }

    /// "‹" chevron with its tip at the origin, spanning to `extent` and its mirror; negate `extent.x` for a "›".
    pub fn chevron(point: Vec2, extent: Vec2) -> Self {
        Self::segment(point, Vec2::ZERO, extent).union(Self::segment(point, Vec2::ZERO, vec2(extent.x, -extent.y)))
    }

    /// Wraps a signed distance, negative inside the shape.
    pub const fn new(distance: f32) -> Self {
        Self { distance }
    }

    /// Computes fragment derivatives once for reuse across coverage and outline queries.
    pub fn sample(self) -> SdfSample {
        SdfSample::new(self.distance)
    }

    /// Returns interior coverage from zero to one with derivative-based antialiasing.
    pub fn fill(self) -> f32 {
        self.sample().fill()
    }

    /// Returns antialiased coverage of a band extending `half_width` either side of the contour.
    pub fn stroke(self, half_width: f32) -> f32 {
        self.sample().stroke(half_width)
    }

    /// Takes the minimum distance, preserving union membership but not exact interior distance.
    pub const fn union(self, other: Self) -> Self {
        Self::new(self.distance.min(other.distance))
    }

    /// Takes the maximum distance, keeping only the region inside both fields.
    pub const fn intersection(self, other: Self) -> Self {
        Self::new(self.distance.max(other.distance))
    }

    /// Removes the other field's interior from this field.
    pub const fn difference(self, other: Self) -> Self {
        Self::new(self.distance.max(-other.distance))
    }

    /// Linearly interpolates two distance fields without clamping the blend amount.
    pub fn lerp(self, other: Self, amount: f32) -> Self {
        Self::new(self.distance.lerp(other.distance, amount))
    }

    /// Blends toward a polynomial smooth union; nonpositive radii give an ordinary union.
    pub fn smooth_union(self, other: Self, radius: f32, amount: f32) -> Self {
        if radius <= 0.0 {
            return self.lerp(self.union(other), amount);
        }
        let blend = (0.5 + 0.5 * (other.distance - self.distance) / radius).clamp(0.0, 1.0);
        let union = other.distance.lerp(self.distance, blend) - radius * blend * (1.0 - blend);
        Self::new(self.distance.lerp(union, amount))
    }
}

impl Add<f32> for Sdf {
    type Output = Self;

    fn add(self, rhs: f32) -> Self {
        Self::new(self.distance + rhs)
    }
}

impl Sub<f32> for Sdf {
    type Output = Self;

    fn sub(self, rhs: f32) -> Self {
        Self::new(self.distance - rhs)
    }
}

impl Neg for Sdf {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.distance)
    }
}

/// A negative-inside signed distance with derivative-aware antialiasing.
#[must_use]
pub struct SdfSample {
    /// Signed distance at the sampled position.
    pub distance: f32,
    /// Antialiasing half-width in the same units as the distance.
    pub half_width: f32,
}

impl SdfSample {
    /// Samples fragment derivatives and clamps the antialiasing half-width to 0.35..=1.0.
    pub fn new(distance: f32) -> Self {
        Self {
            distance,
            // Keep antialiasing local because derivatives spike at primitive boundaries.
            half_width: (distance.fwidth() * 0.5).clamp(0.35, 1.0),
        }
    }

    /// Returns antialiased interior coverage from zero to one.
    pub fn fill(&self) -> f32 {
        self.coverage(self.distance)
    }

    /// Returns coverage after expanding the field; negative distances erode it.
    pub fn expanded(&self, pixels: f32) -> f32 {
        self.coverage(self.distance - pixels)
    }

    /// Returns coverage of the exterior outline, excluding the original fill.
    pub fn outline(&self, pixels: f32) -> f32 {
        (self.expanded(pixels) - self.fill()).max(0.0)
    }

    /// Returns coverage of a band extending `half_width` either side of the contour.
    pub fn stroke(&self, half_width: f32) -> f32 {
        self.coverage(self.distance.abs() - half_width)
    }

    /// Composites straight-alpha fill and outline colors.
    pub fn color(&self, fill: Vec4, outline: Vec4, pixels: f32) -> Vec4 {
        let fill_alpha = self.fill() * fill.w;
        let outline_alpha = self.outline(pixels) * outline.w;
        let alpha = fill_alpha + outline_alpha;
        ((fill.truncate() * fill_alpha + outline.truncate() * outline_alpha) / alpha.max(0.0001)).extend(alpha)
    }

    fn coverage(&self, distance: f32) -> f32 {
        distance.smoothstep(self.half_width, -self.half_width)
    }
}
