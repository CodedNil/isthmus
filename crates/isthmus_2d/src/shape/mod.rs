//! Signed-distance shapes: constructors for common outlines, composed through [`Sdf`].
use crate::{Quad, Rect};
use core::f32::consts::TAU;
use isthmus::{
    Float, MinMax,
    glam::{Vec2, vec2},
};
pub use sample::Sample;
pub use sdf::Sdf;

mod sample;
mod sdf;
#[cfg(test)]
mod tests;

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
