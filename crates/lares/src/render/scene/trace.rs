use super::{FAR, MIN_HIT_DISTANCE};
use isthmus::prelude::*;

// Explicit loop counters avoid iterator machinery forcing Rust-GPU to inline these routines.

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
        let near = Vec3::select(parallel, parallel_near, a.min_num(b));
        let far = Vec3::select(parallel, Vec3::splat(FAR), a.max_num(b));
        (near.max_element().max_num(0.0), far.min_element())
    }
}

/// Validated, slope-adaptive over-relaxation. Consecutive samples estimate the
/// largest step whose empty spheres overlap; every trial is checked before it
/// is accepted. Failed trials return to the last certified sphere boundary.
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
    for _ in 0..192 {
        // Resolve primary geometry to < 0.1 pixel instead of imposing a fixed
        // sub-millimetre tolerance at every viewing distance. Shadow/depth
        // queries use pixel_angle = 0 and retain the original exact tolerance.
        let epsilon = (travelled * pixel_angle * 0.08).max_num(MIN_HIT_DISTANCE).min_num(0.002);
        let radius = distance(origin + direction * travelled).abs();
        if previous + radius < step {
            travelled -= step - previous;
            previous = 0.0;
            step = 0.0;
            continue;
        }
        if radius <= epsilon {
            return travelled;
        }
        // A safe step beyond the bounds proves a miss, without sampling outside.
        if travelled + radius > exit {
            return FAR;
        }
        let relax = if step > 0.0 {
            let slope = (radius - previous) / step;
            (2.0 / (1.0 - slope).max_num(0.05)).clamp_num(1.0, 1.95)
        } else {
            1.0
        };
        previous = radius;
        step = (radius * relax).min_num(exit - travelled);
        travelled += step;
    }
    FAR
}

/// Interval where a*t*t + b*t + c <= 0 (a >= 0).
#[inline(never)]
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
    vec2(r0.min_num(r1), r0.max_num(r1))
}
fn overlap(a: Vec2, b: Vec2) -> Vec2 {
    vec2(a.x.max_num(b.x), a.y.min_num(b.y))
}

/// Exact intersection with the cupped parabolic leaf, including its thin rim.
/// Two half-leaves remove abs(y), leaving only quadratic inequalities.
#[inline(never)]
pub(super) fn leaf(origin: Vec3, direction: Vec3, size: Vec3, enter: f32, exit: f32, shield: bool) -> f32 {
    let o = super::decor::leaf_frame(origin, size);
    let d = super::decor::leaf_frame(direction, size);
    let length = size.x * 0.5;
    let width = size.y * 0.5;
    let mut nearest = FAR;
    let mut half = 0;
    while half < 2 {
        let sign = half as f32 * 2.0 - 1.0;
        let mut span = vec2(enter, exit.min_num(nearest));
        span = overlap(span, quadratic(0.0, -sign * d.y, -sign * o.y));
        span = overlap(span, quadratic(0.0, d.x, o.x - length));
        span = overlap(span, quadratic(0.0, -d.x, -o.x - length));
        let curve = width / (length * length) * if shield { 0.625 / 0.973_44 } else { 1.0 };
        let slope = if shield { width * 0.31 / (0.973_44 * length) } else { 0.0 };
        let maximum = width * if shield { 0.935 / 0.973_44 } else { 1.0 };
        if shield {
            span = overlap(span, quadratic(0.0, -d.x - 1.1 * sign * d.y, -o.x - length * 0.65 - 1.1 * sign * o.y));
        }
        span = overlap(
            span,
            quadratic(
                curve * d.x * d.x,
                2.0 * curve * o.x * d.x + slope * d.x + sign * d.y,
                curve * o.x * o.x + slope * o.x + sign * o.y - maximum,
            ),
        );
        let bend = 0.18 / length;
        let transverse = if shield { 0.32 / width } else { 0.0 };
        let mut square = bend * d.x * d.x - transverse * d.y * d.y;
        let mut linear =
            d.z + 2.0 * bend * o.x * d.x - if shield { 2.0 * transverse * o.y * d.y } else { 0.32 * sign * d.y };
        let mut constant = o.z + bend * o.x * o.x - if shield { transverse * o.y * o.y } else { 0.32 * sign * o.y };
        // A saddle sheet can curve either way along a ray. Its absolute
        // thickness band is unchanged by flipping the quadratic's sign.
        if square < 0.0 {
            square = -square;
            linear = -linear;
            constant = -constant;
        }
        span = overlap(span, quadratic(square, linear, constant - 0.0025));
        let hollow = quadratic(square, linear, constant + 0.0025);
        let entry = span.x;
        if entry > hollow.x && entry < hollow.y {
            span.x = hollow.y;
        }
        if span.x <= span.y {
            nearest = nearest.min_num(span.x);
        }
        half += 1;
    }
    nearest
}

#[inline(never)]
pub(super) fn stem(o: Vec3, d: Vec3, size: Vec3) -> f32 {
    let radius = super::stem_radius(size);
    let length = size.length();
    let axis = size / length.max_num(1.0e-8);
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
    let mut root = 0;
    while root < 2 {
        let t = if root == 0 { cylinder.x } else { cylinder.y };
        if t >= 0.0 && (axial_o + t * axial_d).abs() <= length * 0.5 {
            nearest = nearest.min_num(t);
        }
        root += 1;
    }
    let mut half = 0;
    while half < 2 {
        let sign = half as f32 * 2.0 - 1.0;
        let p = o - size * (sign * 0.5);
        let sphere = quadratic(1.0, 2.0 * p.dot(d), p.length_squared() - radius * radius);
        let t = if sphere.x >= 0.0 { sphere.x } else { sphere.y };
        if t >= 0.0 {
            nearest = nearest.min_num(t);
        }
        half += 1;
    }
    nearest
}

pub(super) fn normal(point: Vec3, distance: impl Fn(Vec3) -> f32) -> Vec3 {
    let a = vec3(1.0, -1.0, -1.0);
    let b = vec3(-1.0, -1.0, 1.0);
    let c = vec3(-1.0, 1.0, -1.0);
    let d = Vec3::ONE;
    let gradient = a * distance(point + a * super::NORMAL_OFFSET)
        + b * distance(point + b * super::NORMAL_OFFSET)
        + c * distance(point + c * super::NORMAL_OFFSET)
        + d * distance(point + d * super::NORMAL_OFFSET);
    let length = gradient.length();
    if length > 0.0 { gradient / length } else { Vec3::Z }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analytic_leaf_hits_match_sdf_sheets_across_shapes_scales_and_tilts() {
        use super::super::{Distance, Kind, Sample, render_furniture};
        for kind in [Kind::Leaf, Kind::ShieldLeaf] {
            for length in [0.12, 0.32, 0.65] {
                for lean in [-0.8, 0.0, 0.65] {
                    let size = vec3(length, length * 0.56, length * lean);
                    for sample in 0..121 {
                        let origin = vec3(
                            ((sample % 11) as f32 / 10.0 - 0.5) * length * 1.3,
                            ((sample / 11) as f32 / 10.0 - 0.5) * size.y * 1.3,
                            length * 1.5,
                        );
                        let direction = vec3(0.13, -0.08, -1.0).normalize();
                        let actual = leaf(origin, direction, size, 0.0, length * 4.0, kind == Kind::ShieldLeaf);
                        let expected = march(origin, direction, 0.0, length * 4.0, 0.0, |p| {
                            render_furniture::<Distance>(kind, p, size).distance()
                        });
                        if actual < FAR {
                            assert!(
                                expected < FAR && (actual - expected).abs() < 0.002,
                                "analytic leaf mismatch: {size:?} {actual} vs {expected}"
                            );
                        } else if expected < FAR {
                            // The marcher accepts a small SDF tolerance outside a
                            // silhouette. A genuine analytic miss must lie there.
                            let p = origin + direction * expected;
                            assert!(render_furniture::<Distance>(kind, p, size).distance().abs() < 0.0003);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn adaptive_steps_agree_with_analytic_sphere_intersections_across_scales() {
        for radius in [0.1, 1.0, 5.0] {
            for sample in 0..100 {
                let relative = sample as f32 / 100.0;
                let origin = vec3(-3.0 * radius, relative * radius, 0.0);
                let hit = march(origin, Vec3::X, 0.0, 6.0 * radius, 0.0, |p| p.length() - radius);
                let cosine = (1.0 - relative * relative).sqrt();
                let expected = radius * (3.0 - cosine);
                let tolerance = MIN_HIT_DISTANCE / cosine * 1.5;
                assert!(
                    (hit - expected).abs() <= tolerance,
                    "sphere entry mismatch at radius={radius}, offset={relative}"
                );
            }
            for relative in [1.01, 1.1, 2.0] {
                let miss = march(vec3(-3.0 * radius, relative * radius, 0.0), Vec3::X, 0.0, 6.0 * radius, 0.0, |p| {
                    p.length() - radius
                });
                assert!(miss >= FAR);
            }
        }
    }
}
