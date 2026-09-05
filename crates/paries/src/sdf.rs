#[path = "shaders/bamboo.rs"]
pub mod bamboo;

pub use bamboo::Bamboo;
use isthmus::{Float as _, Sdf, glam::Vec2};

isthmus::program!();

/// Signed distance to a line segment with a variable radius at each end.
pub fn tapered_segment(point: Vec2, start: Vec2, end: Vec2, start_radius: f32, end_radius: f32) -> Sdf {
    let line = end - start;
    let t = ((point - start).dot(line) / line.length_squared()).clamp(0.0, 1.0);
    Sdf::new((point - start - line * t).length() - start_radius.lerp(end_radius, t))
}

/// Approximate signed distance to an ellipse with stable antialiasing.
pub fn ellipse(point: Vec2, radii: Vec2) -> Sdf {
    let normalized = point / radii;
    Sdf::new((normalized.length() - 1.0) * radii.x.min(radii.y))
}

/// Small deterministic hash for procedural placement and texture.
pub fn hash(value: f32) -> f32 {
    (value.sin() * 43_758.547).fract().abs()
}

pub fn noise(point: Vec2) -> f32 {
    let cell = point.floor();
    let fraction = point - cell;
    let blend = fraction * fraction * (Vec2::splat(3.0) - fraction * 2.0);
    let a = hash(cell.dot(Vec2::new(127.1, 311.7)));
    let b = hash((cell + Vec2::X).dot(Vec2::new(127.1, 311.7)));
    let c = hash((cell + Vec2::Y).dot(Vec2::new(127.1, 311.7)));
    let d = hash((cell + Vec2::ONE).dot(Vec2::new(127.1, 311.7)));
    a.lerp(b, blend.x).lerp(c.lerp(d, blend.x), blend.y)
}

pub fn fbm(mut point: Vec2) -> f32 {
    let mut value = 0.0;
    let mut weight = 0.5;
    for _ in 0..4 {
        value += noise(point) * weight;
        point = Vec2::new(point.x * 1.7 + point.y * 1.1, point.y * 1.7 - point.x * 1.1);
        weight *= 0.5;
    }
    value
}
