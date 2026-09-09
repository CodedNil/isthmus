use core::{f32::consts::TAU, ops::Range};
use isthmus::{
    Fragment, Paint, Primitive, Program, Quad, Rect, Vertex, VertexInput,
    glam::{FloatExt, Vec2, Vec4, vec2},
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
        let quad = quad.into();
        let radius = radius.clamp(0.0, (quad.size.min_element() * 0.5).max(0.0));
        Self::from_fn(quad, move |point| {
            let corner = quad.local(point).abs() - quad.size * 0.5 + radius;
            corner.max(Vec2::ZERO).length() + corner.max_element().min(0.0) - radius
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

    /// Creates a disk with a nonnegative radius.
    pub fn circle(center: Vec2, radius: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = radius.max(0.0);
        Self::from_fn(Rect::from_center_size(center, Vec2::splat(radius * 2.0)), move |point| {
            point.distance(center) - radius
        })
    }

    /// Creates a round-ended segment with the given full width.
    pub fn segment(start: Vec2, end: Vec2, width: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let width = width.max(0.0);
        Self::pill(Quad::oriented(start.midpoint(end), vec2(start.distance(end) + width, width), end - start))
    }

    /// Creates an unsigned circular arc centerline.
    pub fn arc(center: Vec2, radius: f32, start: f32, sweep: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let sweep = sweep.clamp(-TAU, TAU);
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
        let inner_radius = inner_radius.clamp(0.0, radius);
        Self::from_fn(Vec2::splat(radius * 2.0), move |point| {
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
        let radius = radius.clamp(0.0, size.max(0.0));
        let extent = size.max(0.0) - radius;
        let k = 1.732_050_8;
        Self::from_fn(
            Rect::from_center_size(vec2(0.0, extent * 0.25), vec2(k * extent, 1.5 * extent) + radius * 2.0),
            move |point| {
                let mut point = vec2(point.x.abs(), point.y);
                let h = (point.x + k * point.y).max(0.0);
                point -= 0.5 * vec2(h, h * k);
                point -= vec2(point.x.clamp(-0.5 * extent * k, 0.5 * extent * k), -0.5 * extent);
                point.length() * if point.y > 0.0 { -1.0 } else { 1.0 } - radius
            },
        )
    }

    /// Creates an open chevron with its tip at the origin.
    pub fn chevron(extent: Vec2) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Self::segment(Vec2::ZERO, extent, 0.0).union(Self::segment(Vec2::ZERO, vec2(extent.x, -extent.y), 0.0))
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
            outline: self.outline_width(),
            distance: move |point| self.distance_at(point).min(other.distance_at(point)),
        }
    }
    /// Keeps the shared region, retaining inverted bounds for empty intersections.
    fn intersection(self, other: impl Sdf) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape {
            bounds: self.geometry_bounds().intersection(other.geometry_bounds()),
            outline: self.outline_width(),
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
            outline: self.outline_width(),
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
    type Outputs = ();
    type Sample = Sample;

    fn sample(self, fragment: Fragment, (): ()) -> (Sample, f32) {
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
