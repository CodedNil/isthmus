use core::{f32::consts::TAU, ops::Range};
use isthmus::{
    Fragment, Primitive, Program, Quad, Vertex, VertexInput,
    glam::{FloatExt, Vec2, vec2},
    spirv_std::arch::Derivative,
    surface,
};

/// A distance function with automatically composed bounds.
#[derive(Clone, Copy)]
#[must_use]
pub struct Shape<F = fn(Vec2) -> f32> {
    distance: F,
    bounds: Quad,
}

impl Shape {
    /// Creates an unrestricted field with a declared conservative draw region.
    pub fn from_fn<F: Fn(Vec2) -> f32 + Copy>(bounds: impl Into<Quad>, distance: F) -> Shape<F> {
        let (min, max) = bounds.into().extents();
        Shape { distance, bounds: Quad::from_min_max(min, max) }
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
        Self::from_fn(Quad::new(center, Vec2::splat(radius * 2.0), Vec2::X), move |point| {
            point.distance(center) - radius
        })
    }

    /// Creates a round-ended segment with the given full width.
    pub fn segment(start: Vec2, end: Vec2, width: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let direction = end - start;
        let length = direction.length();
        let width = width.max(0.0);
        Self::pill(Quad::new(
            start.midpoint(end),
            vec2(length + width, width),
            if length > 0.0 { direction / length } else { Vec2::X },
        ))
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
        Self::from_fn(Quad::new(Vec2::ZERO, Vec2::splat(radius * 2.0), Vec2::X), move |point| {
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
            Quad::new(vec2(0.0, extent * 0.25), vec2(k * extent, 1.5 * extent) + radius * 2.0, Vec2::X),
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

impl<F: Fn(Vec2) -> f32 + Copy> Shape<F> {
    /// Encloses the field and its declared effects at the requested distance threshold.
    pub fn bounds(self, reach: f32) -> Option<Quad> {
        let bounds = self.bounds.expanded(reach);
        bounds.size.cmpge(Vec2::ZERO).all().then_some(bounds)
    }

    /// Samples the composed signed distance.
    pub fn distance_at(self, point: Vec2) -> f32 {
        (self.distance)(point)
    }

    /// Tests membership, including the boundary.
    pub fn contains(self, point: Vec2) -> bool {
        self.distance_at(point) <= 0.0
    }

    /// Declares an exterior outline, expanding raster bounds while preserving the contour.
    pub const fn outlined(self, width: f32) -> Outlined<F> {
        Outlined { shape: self, width: width.max(0.0) }
    }

    /// Evaluates the field and its screen-space gradient once for coverage queries.
    pub fn sample_at(self, point: Vec2) -> Sample {
        Sample::new(self.distance_at(point), 0.0)
    }

    /// Samples antialiased interior coverage.
    pub fn fill_at(self, point: Vec2) -> f32 {
        self.sample_at(point).fill()
    }

    fn combined_bounds(self, other: Quad, intersection: bool) -> Quad {
        let (amin, amax) = self.bounds.extents();
        let (bmin, bmax) = other.extents();
        if intersection {
            Quad::from_min_max(amin.max(bmin), amax.min(bmax))
        } else {
            Quad::from_min_max(amin.min(bmin), amax.max(bmax))
        }
    }

    /// Includes either operand.
    pub fn union(self, other: Shape<impl Fn(Vec2) -> f32 + Copy>) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape::from_fn(self.combined_bounds(other.bounds, false), move |point| {
            (self.distance)(point).min((other.distance)(point))
        })
    }

    /// Includes both operands, retaining inverted bounds until rasterization.
    pub fn intersection(self, other: Shape<impl Fn(Vec2) -> f32 + Copy>) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape::from_fn(self.combined_bounds(other.bounds, true), move |point| {
            (self.distance)(point).max((other.distance)(point))
        })
    }

    /// Cuts the second operand out of this shape.
    pub fn difference(self, other: Shape<impl Fn(Vec2) -> f32 + Copy>) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape::from_fn(self.bounds, move |point| (self.distance)(point).max(-(other.distance)(point)))
    }

    /// Blends toward a polynomial smooth union.
    pub fn smooth_union(
        self,
        other: Shape<impl Fn(Vec2) -> f32 + Copy>,
        radius: f32,
        amount: f32,
    ) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let radius = radius.max(0.0);
        let amount = amount.saturate();
        let bounds = if amount == 0.0 {
            self.bounds
        } else {
            self.combined_bounds(other.bounds, false).expanded(radius * amount * 0.25)
        };
        Shape::from_fn(bounds, move |point| {
            let a = (self.distance)(point);
            if amount == 0.0 {
                return a;
            }
            let b = (other.distance)(point);
            let blend = (1.0 - (a - b).abs() / radius.max(f32::MIN_POSITIVE)).saturate();
            let union = a.min(b) - radius * blend * blend * 0.25;
            if amount >= 1.0 { union } else { a.lerp(union, amount) }
        })
    }

    /// Expands the contour; negative amounts erode it.
    pub fn offset(self, amount: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape::from_fn(self.bounds.expanded(amount), move |point| (self.distance)(point) - amount)
    }

    /// Creates a centered band with the given full thickness.
    pub fn stroke(self, thickness: f32) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let width = thickness.max(0.0) * 0.5;
        Shape::from_fn(self.bounds.expanded(width), move |point| (self.distance)(point).abs() - width)
    }

    /// Moves the field and its bounds together.
    pub fn translated(self, offset: Vec2) -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        let mut bounds = self.bounds;
        bounds.center += offset;
        Shape::from_fn(bounds, move |point| (self.distance)(point - offset))
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

/// A composed shape with an exterior outline and automatically expanded bounds.
#[derive(Clone, Copy)]
pub struct Outlined<F> {
    /// Geometry retained for additional distance queries.
    pub shape: Shape<F>,
    width: f32,
}

impl<F: Fn(Vec2) -> f32 + Copy> From<Shape<F>> for Outlined<F> {
    fn from(shape: Shape<F>) -> Self {
        shape.outlined(0.0)
    }
}

impl<F: Fn(Vec2) -> f32 + Copy> Outlined<F> {
    fn raster<P: Program>(self) -> impl Primitive<P, Outputs = (), Sample = ()> {
        surface(self.bounds(0.0), |_| ((), 1.0))
    }

    /// Encloses the shape, its outline, and any additional displacement.
    pub fn bounds(self, reach: f32) -> Option<Quad> {
        self.shape.bounds(reach + self.width)
    }

    /// Samples a distance using the declared outline width.
    pub fn sample(self, distance: f32) -> Sample {
        Sample::new(distance, self.width)
    }
}

macro_rules! primitive {
    ($ty:ident, $sample:expr) => {
        impl<P: Program, F: Fn(Vec2) -> f32 + Copy> Primitive<P> for $ty<F> {
            type Outputs = ();
            type Sample = Sample;

            fn sample(self, fragment: Fragment, (): ()) -> (Sample, f32) {
                let sample = ($sample)(self, fragment.pixel);
                (sample, sample.coverage)
            }

            fn vertex_count(self, pixel_size: f32) -> u32 {
                Primitive::<P>::vertex_count(Outlined::from(self).raster(), pixel_size)
            }

            fn vertex(self, input: VertexInput<P>) -> Vertex {
                Outlined::from(self).raster().vertex(input)
            }
        }
    };
}

primitive!(Shape, |shape: Shape<F>, point| shape.sample_at(point));
primitive!(Outlined, |outlined: Outlined<F>, point| outlined.sample(outlined.shape.distance_at(point)));
