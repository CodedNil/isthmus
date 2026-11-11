//! The `Sdf` trait: distance, bounds and outline, with combinators that preserve all three.
use super::{Sample, Shape};
use crate::{Rect, raster};
use isthmus::{Float, Fragment, Primitive, Program, Vertex, VertexInput, glam::Vec2};

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
