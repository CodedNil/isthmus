use core::{f32::consts::TAU, ops::Range};
use isthmus::{
    Float, Fragment, MinMax, Paint, Primitive, Program, Quad, Rect, Vertex, VertexInput,
    glam::{Vec2, Vec4, vec2},
    raster,
    spirv_std::arch::Derivative,
};

/// A distance function with automatically composed bounds.
#[derive(Clone, Copy)]
pub struct Shape<F = fn(Vec2) -> f32> {
    distance: F,
    bounds: Rect,
    outline: f32,
}

impl Shape {
    /// Creates an unrestricted field with a declared conservative draw region.
    pub fn from_fn<F: Fn(Vec2) -> f32 + Copy>(bounds: impl Into<Rect>, distance: F) -> Shape<F> {
        Shape { distance, bounds: bounds.into(), outline: 0.0 }
    }

    /// Creates a rounded rectangle preserving the supplied outer dimensions.
    pub fn rounded_rect(quad: impl Into<Quad>, radius: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Self::side_rounded_rect(quad, radius, radius)
    }

    /// Creates a rounded rectangle whose left and right corners are shaped separately.
    pub fn side_rounded_rect(
        quad: impl Into<Quad>,
        left: impl Into<Corner>,
        right: impl Into<Corner>,
    ) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let quad = quad.into();
        let limit = (quad.size.min_element() * 0.5).max(0.0);
        let (left, right) = (left.into(), right.into());
        let (left, right) = (left.clamped(limit), right.clamped(limit));
        Self::from_fn(quad, move |point| {
            let local = quad.local(point);
            let Corner { radius, roundness } = if local.x < 0.0 { left } else { right };
            let corner = local.abs() - quad.size * 0.5 + radius;
            let outside = corner.max(Vec2::ZERO);
            // Squircle corners measure the overshoot with the 4-norm, circular ones with the 2-norm.
            let reach = if roundness >= 1.0 {
                outside.length()
            } else {
                (outside * outside).length().sqrt().lerp(outside.length(), roundness)
            };
            reach + corner.max_element().min(0.0) - radius
        })
    }

    /// Creates a rectangle with sharp corners.
    pub fn rectangle(quad: impl Into<Quad>) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Self::rounded_rect(quad, 0.0)
    }

    /// Fits a capsule to a rectangle.
    pub fn pill(quad: impl Into<Quad>) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let quad = quad.into();
        Self::rounded_rect(quad, quad.size.min_element() * 0.5)
    }

    /// Creates a regular pointy-top hexagon whose vertices lie `radius` from its centre. Round the corners
    /// by shrinking `radius` and calling `offset`, which keeps the exact distance.
    pub fn hexagon(center: Vec2, radius: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let apothem = radius.max(0.0) * 0.866_025_4;
        Self::from_fn(Rect::from_center_size(center, vec2(apothem * 2.0, radius.max(0.0) * 2.0)), move |point| {
            // The usual flat-top hexagon distance, with the axes swapped to point it upwards.
            let k = vec2(-0.866_025_4, 0.5);
            let offset = (point - center).abs();
            let mut point = vec2(offset.y, offset.x);
            point -= 2.0 * k.dot(point).min(0.0) * k;
            point -= vec2(point.x.clamp_num(-0.577_350_26 * apothem, 0.577_350_26 * apothem), apothem);
            point.length() * if point.y > 0.0 { 1.0 } else { -1.0 }
        })
    }

    /// Creates a disk with a nonnegative radius.
    pub fn circle(center: Vec2, radius: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = radius.max(0.0);
        Self::from_fn(Rect::from_center_size(center, Vec2::splat(radius * 2.0)), move |point| {
            point.distance(center) - radius
        })
    }

    /// Creates a round-ended segment with the given full width.
    pub fn segment(start: Vec2, end: Vec2, width: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = width.max(0.0) * 0.5;
        let axis = end - start;
        let inverse_length_squared = axis.length_squared().max(f32::MIN_POSITIVE).recip();
        Self::from_fn(Rect::new(start.min(end), start.max(end)).expanded(radius), move |point| {
            let along = (point - start).dot(axis) * inverse_length_squared;
            (point - start - axis * along.saturate()).length() - radius
        })
    }

    /// Creates an unsigned circular arc centerline.
    pub fn arc(center: Vec2, radius: f32, start: f32, sweep: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let sweep = sweep.clamp_num(-TAU, TAU);
        let radius = radius.max(0.0);
        let axis = Vec2::from_angle(start + sweep * 0.5);
        let edge = Vec2::from_angle(sweep.abs() * 0.5);
        Self::from_fn(
            Quad::new(
                center + axis * (radius * (1.0 + edge.x) * 0.5),
                vec2(1.0 - edge.x, if edge.x >= 0.0 { edge.y * 2.0 } else { 2.0 }) * radius,
                axis,
            ),
            move |point| {
                let offset = point - center;
                let local = vec2(offset.dot(axis), offset.dot(axis.perp()).abs());
                if local.x >= edge.x * local.length() {
                    (local.length() - radius).abs()
                } else {
                    (local - edge * radius).length()
                }
            },
        )
    }

    /// Creates a five-pointed star centered at the origin.
    pub fn star(radius: f32, inner_radius: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = radius.max(0.0);
        let inner_radius = inner_radius.clamp_num(0.0, radius);
        // Tips reach sin(72°) of the radius sideways, and the lowest sit cos(36°) below the centre.
        let bounds = Rect::new(vec2(-0.951_056_5, -1.0) * radius, vec2(0.951_056_5, 0.809_017) * radius);
        Self::from_fn(bounds, move |point| {
            if radius == 0.0 {
                return point.length();
            }
            let k = vec2(0.809_017, -0.587_785_25);
            let other = vec2(-k.x, k.y);
            let mut point = vec2(point.x.abs(), -point.y);
            point -= 2.0 * k.dot(point).max(0.0) * k;
            point -= 2.0 * other.dot(point).max(0.0) * other;
            point.x = point.x.abs();
            point.y -= radius;
            let edge = inner_radius * vec2(-k.y, k.x) - vec2(0.0, radius);
            let along = (point.dot(edge) / edge.length_squared()).saturate();
            (point - edge * along).length() * if point.y * edge.x - point.x * edge.y < 0.0 { -1.0 } else { 1.0 }
        })
    }

    /// Creates a rounded equilateral triangle.
    pub fn rounded_triangle(size: f32, radius: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = radius.clamp_num(0.0, size.max(0.0));
        let extent = size.max(0.0) - radius;
        let k = 1.732_050_8;
        Self::from_fn(
            Rect::from_center_size(vec2(0.0, extent * 0.25), vec2(k * extent, 1.5 * extent) + radius * 2.0),
            move |point| {
                let mut point = vec2(point.x.abs(), point.y);
                let h = (point.x + k * point.y).max(0.0);
                point -= 0.5 * vec2(h, h * k);
                point -= vec2(point.x.clamp_num(-0.5 * extent * k, 0.5 * extent * k), -0.5 * extent);
                point.length() * if point.y > 0.0 { -1.0 } else { 1.0 } - radius
            },
        )
    }

    /// Creates an open chevron with its tip at the origin.
    pub fn chevron(extent: Vec2) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Self::segment(Vec2::ZERO, extent, 0.0).union(Self::segment(Vec2::ZERO, vec2(extent.x, -extent.y), 0.0))
    }
}

/// One end of a rounded rectangle.
#[derive(Clone, Copy)]
pub struct Corner {
    /// Size of the rounding, in the same units as the rectangle.
    pub radius: f32,
    /// 0 gives squircle corners that hug the rectangle, 1 gives circular ones.
    pub roundness: f32,
}

impl Corner {
    fn clamped(self, limit: f32) -> Self {
        Self { radius: self.radius.clamp_num(0.0, limit), roundness: self.roundness.saturate() }
    }
}

impl From<f32> for Corner {
    /// A circular corner of the given radius.
    fn from(radius: f32) -> Self {
        Self { radius, roundness: 1.0 }
    }
}

/// Signed distance, geometry bounds and reserved outline shared by shapes and glyphs.
pub trait Sdf: Copy {
    /// Axis-aligned bounds before outline expansion.
    fn geometry_bounds(self) -> Rect;
    /// Reserved exterior margin.
    fn outline_width(self) -> f32;
    /// Evaluates signed distance to the contour.
    fn distance_at(self, point: Vec2) -> f32;

    /// Encloses the geometry, outline and additional effect reach.
    fn bounds(self, reach: f32) -> Rect {
        self.geometry_bounds().expanded(self.outline_width() + reach)
    }
    /// Tests membership in the unoutlined geometry.
    fn contains(self, point: Vec2) -> bool {
        self.distance_at(point) <= 0.0
    }
    /// Resolves an adjusted distance within the reserved outline.
    fn sample_distance(self, distance: f32) -> Sample {
        Sample::new(distance, self.outline_width())
    }
    /// Evaluates distance, gradient and antialiased coverage.
    fn sample_at(self, point: Vec2) -> Sample {
        self.sample_distance(self.distance_at(point))
    }
    /// Samples antialiased interior coverage.
    fn fill_at(self, point: Vec2) -> f32 {
        self.sample_at(point).fill()
    }
    /// Reserves exterior space without changing the contour.
    fn outlined(self, width: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape {
            bounds: self.geometry_bounds(),
            outline: width.max(0.0),
            distance: move |point| self.distance_at(point),
        }
    }
    /// Includes either operand.
    fn union(self, other: impl Sdf) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape {
            bounds: self.geometry_bounds().union(other.geometry_bounds()),
            outline: self.outline_width().max(other.outline_width()),
            distance: move |point| self.distance_at(point).min(other.distance_at(point)),
        }
    }
    /// Keeps the shared region, retaining inverted bounds for empty intersections.
    fn intersection(self, other: impl Sdf) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape {
            bounds: self.geometry_bounds().intersection(other.geometry_bounds()),
            outline: self.outline_width().max(other.outline_width()),
            distance: move |point| self.distance_at(point).max(other.distance_at(point)),
        }
    }
    /// Cuts the second operand out of this shape.
    fn difference(self, other: impl Sdf) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape {
            bounds: self.geometry_bounds(),
            outline: self.outline_width(),
            distance: move |point| self.distance_at(point).max(-other.distance_at(point)),
        }
    }
    /// Blends the first operand toward a polynomial smooth union.
    fn smooth_union(self, other: impl Sdf, radius: f32, amount: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = radius.max(0.0);
        let amount = amount.saturate();
        let bounds = if amount == 0.0 {
            self.geometry_bounds()
        } else {
            self.geometry_bounds().union(other.geometry_bounds()).expanded(radius * amount * 0.25)
        };
        Shape {
            bounds,
            outline: self.outline_width().max(other.outline_width()),
            distance: move |point| {
                let a = self.distance_at(point);
                if amount == 0.0 {
                    return a;
                }
                let b = other.distance_at(point);
                let blend = (1.0 - (a - b).abs() / radius.max(f32::MIN_POSITIVE)).saturate();
                let union = a.min(b) - radius * blend * blend * 0.25;
                if amount >= 1.0 { union } else { a.lerp(union, amount) }
            },
        }
    }
    /// Expands the contour; negative amounts erode it.
    fn offset(self, amount: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape {
            bounds: self.geometry_bounds().expanded(amount),
            outline: self.outline_width(),
            distance: move |point| self.distance_at(point) - amount,
        }
    }
    /// Creates a centered band with the given full thickness.
    fn stroke(self, thickness: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let width = thickness.max(0.0) * 0.5;
        Shape {
            bounds: self.geometry_bounds().expanded(width),
            outline: self.outline_width(),
            distance: move |point| self.distance_at(point).abs() - width,
        }
    }
    /// Scales about the origin by positive per-axis factors. Exact for equal factors; otherwise the
    /// field stays a conservative distance (never larger than the true one) rather than an exact one.
    fn scaled(self, factor: Vec2) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let factor = factor.max(Vec2::splat(f32::MIN_POSITIVE));
        let (inverse, shrink) = (factor.recip(), factor.min_element());
        let bounds = self.geometry_bounds();
        Shape {
            bounds: Rect::new(bounds.min * factor, bounds.max * factor),
            outline: self.outline_width() * factor.max_element(),
            distance: move |point| self.distance_at(point * inverse) * shrink,
        }
    }
    /// Moves geometry and bounds together.
    fn translated(self, offset: Vec2) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let bounds = self.geometry_bounds();
        Shape {
            bounds: Rect::new(bounds.min + offset, bounds.max + offset),
            outline: self.outline_width(),
            distance: move |point| self.distance_at(point - offset),
        }
    }
}

impl<F: Fn(Vec2) -> f32 + Copy> Sdf for Shape<F> {
    fn geometry_bounds(self) -> Rect {
        self.bounds
    }

    fn outline_width(self) -> f32 {
        self.outline
    }

    fn distance_at(self, point: Vec2) -> f32 {
        (self.distance)(point)
    }
}

impl<P: Program, F: Fn(Vec2) -> f32 + Copy> Primitive<P> for Shape<F> {
    type Sample = Sample;

    fn sample(self, fragment: Fragment) -> (Sample, f32) {
        let sample = self.sample_at(fragment.pixel);
        (sample, sample.coverage)
    }

    fn vertex_count(self) -> u32 {
        Primitive::<P>::vertex_count(raster(self.bounds(0.0)))
    }

    fn vertex(self, input: VertexInput<P>) -> Vertex {
        raster(self.bounds(0.0)).vertex(input)
    }
}

/// A distance evaluation with cached derivatives and primitive coverage.
#[derive(Clone, Copy)]
pub struct Sample {
    /// Signed distance after any local deformation.
    pub distance: f32,
    /// Distance derivatives along framebuffer x and y.
    pub gradient: Vec2,
    /// Antialiased coverage of the primitive's selected distance region.
    pub coverage: f32,
}

impl Paint for Sample {
    fn paint(self, fill: Vec4, outline: Vec4) -> Vec4 {
        let foreground = fill.w * self.fill();
        let background = outline.w * (self.coverage - foreground).max(0.0);
        let alpha = foreground + background;
        ((fill.truncate() * foreground + outline.truncate() * background) / alpha.max(f32::MIN_POSITIVE)).extend(alpha)
    }
}

impl Sample {
    /// Resolves a distance and its declared outline with antialiased coverage.
    pub fn new(distance: f32, outline: f32) -> Self {
        let mut sample = Self { distance, gradient: vec2(distance.dfdx(), distance.dfdy()), coverage: 0.0 };
        sample.coverage = sample.below(outline.max(0.0));
        sample
    }

    fn below(self, edge: f32) -> f32 {
        (0.5 + (edge - self.distance) / self.gradient.abs().element_sum().max(f32::MIN_POSITIVE)).saturate()
    }

    /// Antialiased coverage inside the original contour.
    pub fn fill(self) -> f32 {
        self.below(0.0)
    }

    /// Antialiased coverage of an interval in the original distance field.
    pub fn band(self, range: Range<f32>) -> f32 {
        (self.below(range.end) - self.below(range.start)).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Declared bounds minus the true extent of the interior, per side: (min, max).
    fn slack(shape: impl Sdf) -> (Vec2, Vec2) {
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for x in 0..800 {
            for y in 0..800 {
                let point = vec2(x as f32, y as f32) * 0.25 - 100.0;
                if shape.contains(point) {
                    (min, max) = (min.min(point), max.max(point));
                }
            }
        }
        let bounds = shape.geometry_bounds();
        (min - bounds.min, bounds.max - max)
    }

    /// Bounds must never cut the shape off; exact ones must also hug it (to within the sampling grid and thin tips).
    fn assert_bounds(name: &str, shape: impl Sdf, exact: bool) {
        let (min, max) = slack(shape);
        let (low, high) = (min.min(max).min_element(), min.max(max).max_element());
        assert!(low >= -0.3, "{name} bounds cut off the shape by {}", -low);
        assert!(!exact || high <= 0.5, "{name} bounds overshoot by {high}");
    }

    #[test]
    fn primitive_bounds_hug_their_shapes_and_combinators_enclose_them() {
        let rect = Rect::new(vec2(-40.0, -20.0), vec2(60.0, 30.0));
        let (a, b) = (Shape::rounded_rect(rect, 12.0), Shape::circle(vec2(30.0, 10.0), 25.0));
        assert_bounds("rounded_rect", a, true);
        assert_bounds("pill", Shape::pill(rect), true);
        assert_bounds("squircle", Shape::side_rounded_rect(rect, Corner { radius: 18.0, roundness: 0.0 }, 9.0), true);
        assert_bounds("circle", b, true);
        assert_bounds("segment", Shape::segment(vec2(-30.0, -10.0), vec2(50.0, 40.0), 10.0), true);
        assert_bounds("arc", Shape::arc(Vec2::ZERO, 40.0, 0.5, 2.0).offset(3.0), false);
        assert_bounds("star", Shape::star(40.0, 16.0), true);
        assert_bounds("hexagon", Shape::hexagon(vec2(5.0, -3.0), 50.0), true);
        assert_bounds("rounded hexagon", Shape::hexagon(vec2(5.0, -3.0), 30.0).offset(15.0), true);
        assert_bounds("offset", a.offset(6.0), true);
        assert_bounds("stroke", a.stroke(8.0), true);
        assert_bounds("scaled", a.scaled(vec2(1.5, 0.7)), true);
        assert_bounds("union", a.union(b), true);
        assert_bounds("intersection", a.intersection(b), true);
        assert_bounds("difference", a.difference(b), false);
        assert_bounds("smooth_union", a.smooth_union(b, 20.0, 1.0), false);
    }

    #[test]
    fn scaling_keeps_the_field_a_distance_and_unions_keep_the_larger_outline() {
        let shape = Shape::rounded_rect(Rect::new(vec2(10.0, -20.0), vec2(90.0, 30.0)), 12.0);
        let uniform = shape.scaled(Vec2::splat(2.0)).distance_at(vec2(200.0, 0.0));
        assert!((uniform - 2.0 * shape.distance_at(vec2(100.0, 0.0))).abs() < 1e-3);
        // Unequal factors may only underestimate, so antialiasing and shadows never overshoot.
        let stretched = shape.scaled(vec2(1.0, 3.0));
        assert!(stretched.distance_at(vec2(150.0, 0.0)) <= shape.distance_at(vec2(150.0, 0.0)) + 1e-3);

        let (small, large) = (Shape::circle(Vec2::ZERO, 5.0), Shape::circle(vec2(20.0, 0.0), 5.0).outlined(12.0));
        for combined in [small.union(large).outline_width(), small.smooth_union(large, 4.0, 1.0).outline_width()] {
            assert_eq!(combined, 12.0);
        }
        assert_eq!(large.intersection(small).outline_width(), 12.0);
    }
}
