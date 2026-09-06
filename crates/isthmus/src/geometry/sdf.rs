use super::{FragmentGeometry, Quad, text::TextResources};
use crate::{
    glam::{FloatExt, Vec2, vec2},
    spirv_std::arch::Derivative,
};
use core::{f32::consts::TAU, ops::Deref};

/// A bounded distance field; CSG preserves membership but may approximate interior distance.
pub trait SdfShape: crate::ShaderData {
    /// Encloses every point with distance at most `outset`; `None` means the region is empty.
    fn bounds(self, outset: f32) -> Option<Quad>;

    /// Evaluates the signed distance at a logical screen position.
    fn distance_at(self, point: Vec2) -> f32;

    /// Returns antialiased interior coverage in a fragment shader at a logical screen position.
    fn fill_at(self, point: Vec2) -> f32 {
        fill(self.distance_at(point))
    }

    /// Returns an antialiased exterior outline, excluding the original fill.
    fn outline_at(self, point: Vec2, width: f32) -> f32 {
        self.fill_outline_at(point, width).1
    }

    /// Returns disjoint fill and exterior-outline masks, sampling the field and AA width once.
    fn fill_outline_at(self, point: Vec2, width: f32) -> (f32, f32) {
        fill_outline(self.distance_at(point), width)
    }

    /// Tests membership, including points on the boundary.
    fn contains(self, point: Vec2) -> bool {
        self.distance_at(point) <= 0.0
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

    fn distance_at(self, point: Vec2) -> f32 {
        let corner = self.quad.local(point).abs() - self.quad.size * 0.5 + self.radius;
        corner.max(Vec2::ZERO).length() + corner.x.max(corner.y).min(0.0) - self.radius
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

    fn distance_at(self, point: Vec2) -> f32 {
        (point - self.center).length() - self.radius
    }
}

impl Shape<RoundedRect> {
    /// Creates a rounded rectangle, clamping the radius to fit its dimensions.
    pub fn rounded_rect(quad: impl Into<Quad>, radius: f32) -> Self {
        let quad = quad.into();
        Self::new(RoundedRect { quad, radius: radius.clamp(0.0, (quad.size.min_element() * 0.5).max(0.0)) })
    }

    /// Creates a rectangle with sharp corners.
    pub fn rectangle(quad: impl Into<Quad>) -> Self {
        Self::rounded_rect(quad, 0.0)
    }
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

    fn distance_at(self, point: Vec2) -> f32 {
        let half_size = self.quad.size * 0.5;
        let radius = half_size.min_element();
        (self.quad.local(point).abs() - half_size + radius).max(Vec2::ZERO).length() - radius
    }
}

impl Shape<Capsule> {
    /// Fits a capsule to a rectangle, using its shorter side as the diameter.
    pub fn pill(quad: impl Into<Quad>) -> Self {
        Self::new(Capsule { quad: quad.into() })
    }

    /// Creates a round-ended segment with full stroke width; coincident endpoints produce a circle.
    pub fn segment(start: Vec2, end: Vec2, width: f32) -> Self {
        let direction = end - start;
        let length = direction.length();
        let diameter = width.max(0.0);
        Self::pill(Quad::new(
            start.midpoint(end),
            vec2(length + diameter, diameter),
            if length > 0.0 { direction / length } else { Vec2::X },
        ))
    }
}

impl Shape<Circle> {
    /// Creates a disk, clamping negative radii to zero.
    pub const fn circle(center: Vec2, radius: f32) -> Self {
        Self::new(Circle { center, radius: radius.max(0.0) })
    }
}

impl Shape<Arc> {
    /// A circular centerline swept from `start` through `sweep` radians; use `stroke` for thickness.
    pub fn arc(center: Vec2, radius: f32, start: f32, sweep: f32) -> Self {
        let sweep = sweep.clamp(-TAU, TAU);
        Self::new(Arc {
            center,
            radius: radius.max(0.0),
            axis: Vec2::from_angle(start + sweep * 0.5),
            edge: Vec2::from_angle(sweep.abs() * 0.5),
        })
    }
}

/// An unsigned circular arc centerline, created with [`Shape::arc`].
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

    fn distance_at(self, point: Vec2) -> f32 {
        let offset = point - self.center;
        let local = vec2(offset.dot(self.axis), offset.dot(self.axis.perp()).abs());
        let length = local.length();
        if local.x >= self.edge.x * length {
            (length - self.radius).abs()
        } else {
            (local - self.edge * self.radius).length()
        }
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

impl<S> Deref for Shape<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.shape
    }
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

    /// Reserves an effect's outward reach and displacement, including antialiasing.
    pub fn with_effect(mut self, effect: impl super::effect::Effect) -> Self {
        self.margin += effect.outset().max(0.0) + effect.displacement().max(0.0);
        self
    }
}

macro_rules! binary_shape {
    ($name:ident, |$a:ident, $b:ident| $distance:expr, |$this:ident, $outset:ident| $bounds:block) => {
        #[doc = concat!("A bounded `", stringify!($name), "` of two distance fields.")]
        #[derive(Clone, Copy, crate::ShaderData)]
        pub struct $name<A, B> {
            /// First operand, whose region is retained by difference.
            pub a: A,
            /// Second operand, whose region is removed by difference.
            pub b: B,
        }

        impl<A: SdfShape, B: SdfShape> SdfShape for $name<A, B> {
            fn bounds($this, $outset: f32) -> Option<Quad> $bounds

            fn distance_at(self, point: Vec2) -> f32 {
                let $a = self.a.distance_at(point);
                let $b = self.b.distance_at(point);
                $distance
            }
        }
    };
}

binary_shape!(Union, |a, b| a.min(b), |self, outset| { enclosing(self.a.bounds(outset), self.b.bounds(outset)) });

binary_shape!(Intersection, |a, b| a.max(b), |self, outset| {
    let (a_min, a_max) = self.a.bounds(outset)?.extents();
    let (b_min, b_max) = self.b.bounds(outset)?.extents();
    let min = a_min.max(b_min);
    let max = a_max.min(b_max);
    min.cmple(max).all().then_some(Quad::from_min_max(min, max))
});

binary_shape!(Difference, |a, b| a.max(-b), |self, outset| { self.a.bounds(outset) });

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

    fn distance_at(self, point: Vec2) -> f32 {
        let base = self.base.distance_at(point);
        if self.amount == 0.0 {
            base
        } else {
            let other = self.other.distance_at(point);
            let union = if self.radius <= 0.0 {
                base.min(other)
            } else {
                let blend = (0.5 + 0.5 * (other - base) / self.radius).clamp(0.0, 1.0);
                other.lerp(base, blend) - self.radius * blend * (1.0 - blend)
            };
            base.lerp(union, self.amount)
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

    fn distance_at(self, point: Vec2) -> f32 {
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

    fn distance_at(self, point: Vec2) -> f32 {
        self.shape.distance_at(point).abs() - self.half_width
    }
}

impl<S: SdfShape> SdfShape for Translated<S> {
    fn bounds(self, outset: f32) -> Option<Quad> {
        self.shape.bounds(outset).map(|mut quad| {
            quad.center += self.offset;
            quad
        })
    }

    fn distance_at(self, point: Vec2) -> f32 {
        self.shape.distance_at(point - self.offset)
    }
}

fn enclosing(a: Option<Quad>, b: Option<Quad>) -> Option<Quad> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (a_min, a_max) = a.extents();
            let (b_min, b_max) = b.extents();
            Some(Quad::from_min_max(a_min.min(b_min), a_max.max(b_max)))
        }
        (a, b) => a.or(b),
    }
}

impl<S: SdfShape> FragmentGeometry<'_> for S {
    type Payload = Self;
    type Raster = Quad;
    type Sample = Self;

    fn sample(_: Vec2, _: [Vec2; 3], payload: Self, _: TextResources<'_>) -> Self {
        payload
    }
}

#[cfg(not(target_arch = "spirv"))]
impl<S: SdfShape> super::Geometry for Shape<S> {
    type Context = ();
    type Fragment = S;

    fn payload(self) -> S {
        self.shape
    }

    fn primitives(self, (): &()) -> impl Iterator<Item = [Vec2; 3]> {
        self.shape.bounds(self.margin).into_iter().map(Quad::data)
    }
}

/// A five-pointed star centered at the origin.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Star {
    /// Outer vertex radius.
    pub radius: f32,
    /// Inner vertex radius.
    pub inner_radius: f32,
}

impl Shape<Star> {
    /// A five-pointed star centered at the origin.
    pub const fn star(radius: f32, inner_radius: f32) -> Self {
        Self::new(Star { radius: radius.max(0.0), inner_radius: inner_radius.clamp(0.0, radius.max(0.0)) })
    }
}

impl SdfShape for Star {
    fn bounds(self, outset: f32) -> Option<Quad> {
        Some(Quad::new(Vec2::ZERO, Vec2::splat(self.radius * 2.0), Vec2::X).expanded(outset.max(0.0)))
    }

    fn distance_at(self, point: Vec2) -> f32 {
        let Self { radius, inner_radius } = self;
        if radius == 0.0 {
            return point.length();
        }

        let k1 = vec2(0.809_017, -0.587_785_25);
        let k2 = vec2(-k1.x, k1.y);
        let mut point = vec2(point.x.abs(), -point.y);
        point -= 2.0 * k1.dot(point).max(0.0) * k1;
        point -= 2.0 * k2.dot(point).max(0.0) * k2;
        point.x = point.x.abs();
        point.y -= radius;
        let edge = inner_radius * vec2(-k1.y, k1.x) - vec2(0.0, radius);
        let edge_t = (point.dot(edge) / edge.length_squared()).saturate();
        let cross = point.y * edge.x - point.x * edge.y;
        (point - edge * edge_t).length() * if cross < 0.0 { -1.0 } else { 1.0 }
    }
}

/// A rounded equilateral triangle around its construction origin.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct RoundedTriangle {
    /// Construction size before rounding.
    pub size: f32,
    /// Corner radius.
    pub radius: f32,
}

impl Shape<RoundedTriangle> {
    /// A rounded equilateral triangle around its construction origin.
    pub const fn rounded_triangle(size: f32, radius: f32) -> Self {
        Self::new(RoundedTriangle { size: size.max(0.0), radius: radius.clamp(0.0, size.max(0.0)) })
    }
}

impl SdfShape for RoundedTriangle {
    fn bounds(self, outset: f32) -> Option<Quad> {
        let extent = self.size - self.radius;
        Some(
            Quad::new(vec2(0.0, extent * 0.25), vec2(1.732_050_8 * extent, 1.5 * extent) + self.radius * 2.0, Vec2::X)
                .expanded(outset.max(0.0)),
        )
    }

    fn distance_at(self, point: Vec2) -> f32 {
        let Self { size, radius } = self;

        let k = 1.732_050_8;
        let mut point = vec2(point.x.abs(), point.y);
        let h = (point.x + k * point.y).max(0.0);
        point -= 0.5 * vec2(h, h * k);
        point -= vec2(point.x.clamp(-0.5 * (size - radius) * k, 0.5 * (size - radius) * k), -0.5 * (size - radius));
        point.length() * if point.y > 0.0 { -1.0 } else { 1.0 } - radius
    }
}

/// An open chevron with its tip at the origin; use stroke for thickness.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Chevron {
    /// One endpoint; the other is mirrored vertically.
    pub extent: Vec2,
}

impl Shape<Chevron> {
    /// An open chevron with its tip at the origin; use stroke for thickness.
    pub const fn chevron(extent: Vec2) -> Self {
        Self::new(Chevron { extent })
    }
}

impl SdfShape for Chevron {
    fn bounds(self, outset: f32) -> Option<Quad> {
        (outset >= 0.0).then_some(
            Quad::from_min_max(
                vec2(self.extent.x.min(0.0), -self.extent.y.abs()),
                vec2(self.extent.x.max(0.0), self.extent.y.abs()),
            )
            .expanded(outset),
        )
    }

    fn distance_at(self, point: Vec2) -> f32 {
        let Self { extent } = self;

        Shape::segment(Vec2::ZERO, vec2(extent.x, extent.y.abs()), 0.0).distance_at(vec2(point.x, point.y.abs()))
    }
}

/// Converts a negative-inside distance into antialiased interior coverage in a fragment shader.
pub fn fill(distance: f32) -> f32 {
    mask(distance, aa_width(distance))
}

/// Returns antialiased coverage of a band extending `half_width` either side of the contour.
pub fn stroke(distance: f32, half_width: f32) -> f32 {
    mask(distance.abs() - half_width.max(0.0), aa_width(distance))
}

/// Returns disjoint fill and exterior-outline masks using one AA-width calculation.
pub fn fill_outline(distance: f32, width: f32) -> (f32, f32) {
    let aa = aa_width(distance);
    let fill = mask(distance, aa);
    (fill, (mask(distance - width.max(0.0), aa) - fill).max(0.0))
}

fn aa_width(distance: f32) -> f32 {
    // Keep antialiasing local because derivatives spike at primitive boundaries.
    (distance.fwidth() * 0.5).clamp(0.35, 1.0)
}

fn mask(distance: f32, half_width: f32) -> f32 {
    distance.smoothstep(half_width, -half_width)
}
