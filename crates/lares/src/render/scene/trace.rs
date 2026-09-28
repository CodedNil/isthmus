use super::{FAR, MIN_HIT_DISTANCE};
use isthmus::prelude::*;

/// Reuse ray reciprocals across the entire BVH traversal. Zero marks a
/// parallel axis, avoiding both division by zero and 0 * infinity on faces.
pub(super) fn inverse(direction: Vec3) -> Vec3 {
    let component = |v: f32| if v.abs() < 1.0e-8 { 0.0 } else { 1.0 / v };
    vec3(component(direction.x), component(direction.y), component(direction.z))
}

pub(super) struct Ray {
    origin: Vec3,
    inverse: Vec3,
}
impl Ray {
    pub(super) fn new(origin: Vec3, direction: Vec3) -> Self {
        let inverse = inverse(direction);
        Self { origin, inverse }
    }

    pub(super) fn bounds(&self, min: Vec3, max: Vec3) -> (f32, f32) {
        let a = (min - self.origin) * self.inverse;
        let b = (max - self.origin) * self.inverse;
        let parallel = self.inverse.cmpeq(Vec3::ZERO);
        let inside = self.origin.cmpge(min) & self.origin.cmple(max);
        let parallel_near = Vec3::select(inside, Vec3::splat(-FAR), Vec3::splat(FAR + 1.0));
        let near = Vec3::select(parallel, parallel_near, a.min(b));
        let far = Vec3::select(parallel, Vec3::splat(FAR), a.max(b));
        (near.max_element().max(0.0), far.min_element())
    }
}

/// Validated over-relaxation: overlapping empty spheres certify each longer
/// step. A failed trial returns to the last safe sphere boundary.
pub(super) fn march(
    origin: Vec3,
    direction: Vec3,
    enter: f32,
    exit: f32,
    pixel_angle: f32,
    distance: impl Fn(Vec3) -> f32,
) -> f32 {
    let mut travelled = enter;
    let mut previous = 0.0;
    let mut step = 0.0;
    let mut relax = 1.6;
    for _ in 0..192 {
        let epsilon = (travelled * pixel_angle * 0.35).clamp(MIN_HIT_DISTANCE, 0.002);
        let radius = distance(origin + direction * travelled).abs();
        if previous + radius < step {
            travelled -= step - previous;
            previous = 0.0;
            step = 0.0;
            relax = 1.0;
            continue;
        }
        if radius <= epsilon {
            return travelled;
        }
        // A safe step beyond the bounds proves a miss, without sampling outside.
        if travelled + radius > exit {
            return FAR;
        }
        previous = radius;
        step = (radius * relax).min(exit - travelled);
        travelled += step;
    }
    FAR
}

/// Interval where a*t*t + b*t + c <= 0 (a >= 0).
fn quadratic(a: f32, b: f32, c: f32) -> Vec2 {
    if a < 1.0e-12 {
        if b.abs() < 1.0e-12 {
            return if c <= 0.0 { vec2(-FAR, FAR) } else { vec2(FAR, -FAR) };
        }
        let root = -c / b;
        return if b > 0.0 { vec2(-FAR, root) } else { vec2(root, FAR) };
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return vec2(FAR, -FAR);
    }
    let q = -0.5 * (b + if b >= 0.0 { discriminant.sqrt() } else { -discriminant.sqrt() });
    let r0 = q / a;
    let r1 = if q.abs() < 1.0e-20 { r0 } else { c / q };
    vec2(r0.min(r1), r0.max(r1))
}
const fn overlap(a: Vec2, b: Vec2) -> Vec2 {
    vec2(a.x.max(b.x), a.y.min(b.y))
}

/// Exact intersection with the cupped parabolic leaf, including its thin rim.
/// Two half-leaves remove abs(y), leaving only quadratic inequalities.
pub(super) fn leaf(origin: Vec3, direction: Vec3, size: Vec3, enter: f32, exit: f32) -> f32 {
    let o = super::decor::leaf_frame(origin, size);
    let d = super::decor::leaf_frame(direction, size);
    let length = size.x * 0.5;
    let width = size.y * 0.5;
    let mut nearest = FAR;
    for side in 0..2 {
        let sign = side as f32 * 2.0 - 1.0;
        let mut span = vec2(enter, exit.min(nearest));
        span = overlap(span, quadratic(0.0, -sign * d.y, -sign * o.y));
        span = overlap(span, quadratic(0.0, d.x, o.x - length));
        span = overlap(span, quadratic(0.0, -d.x, -o.x - length));
        let curve = width / (length * length);
        span = overlap(
            span,
            quadratic(curve * d.x * d.x, 2.0 * curve * o.x * d.x + sign * d.y, curve * o.x * o.x + sign * o.y - width),
        );
        let bend = 0.18 / length;
        let square = bend * d.x * d.x;
        let linear = d.z + 2.0 * bend * o.x * d.x - 0.32 * sign * d.y;
        let constant = o.z + bend * o.x * o.x - 0.32 * sign * o.y;
        span = overlap(span, quadratic(square, linear, constant - 0.0025));
        let hollow = quadratic(square, linear, constant + 0.0025);
        let entry = span.x;
        if entry > hollow.x && entry < hollow.y {
            span.x = hollow.y;
        }
        if span.x <= span.y {
            nearest = nearest.min(span.x);
        }
    }
    nearest
}

pub(super) fn stem(o: Vec3, d: Vec3, size: Vec3) -> f32 {
    let radius = super::stem_radius(size);
    let length = size.length();
    let axis = size / length.max(1.0e-8);
    let axial_o = o.dot(axis);
    let axial_d = d.dot(axis);
    let perpendicular_o = o - axis * axial_o;
    let perpendicular_d = d - axis * axial_d;
    let cylinder = quadratic(
        perpendicular_d.length_squared(),
        2.0 * perpendicular_o.dot(perpendicular_d),
        perpendicular_o.length_squared() - radius * radius,
    );
    let mut nearest = FAR;
    for root in 0..2 {
        let t = if root == 0 { cylinder.x } else { cylinder.y };
        if t >= 0.0 && (axial_o + t * axial_d).abs() <= length * 0.5 {
            nearest = nearest.min(t);
        }
    }
    for side in 0..2 {
        let sign = side as f32 * 2.0 - 1.0;
        let p = o - size * (sign * 0.5);
        let sphere = quadratic(1.0, 2.0 * p.dot(d), p.length_squared() - radius * radius);
        let t = if sphere.x >= 0.0 { sphere.x } else { sphere.y };
        if t >= 0.0 {
            nearest = nearest.min(t);
        }
    }
    nearest
}

pub(super) fn normal(point: Vec3, distance: impl Fn(Vec3) -> f32) -> Vec3 {
    let directions = [vec3(1.0, -1.0, -1.0), vec3(-1.0, -1.0, 1.0), vec3(-1.0, 1.0, -1.0), Vec3::ONE];
    let mut gradient = Vec3::ZERO;
    let mut index = 0;
    while index < directions.len() {
        let direction = directions[index];
        gradient += direction * distance(point + direction * super::NORMAL_OFFSET);
        index += 1;
    }
    let length = gradient.length();
    if length > 0.0 { gradient / length } else { Vec3::Z }
}
