//! Glyph records shared by the host and the shader: curves, placement, and distance evaluation.
#[cfg(target_arch = "spirv")]
use isthmus::Float;
use isthmus::{
    F16x2, MinMax, ShaderData,
    glam::{UVec2, Vec2, vec2},
};

#[derive(Clone, Copy, ShaderData)]
pub(super) struct Curve {
    pub(super) points: [F16x2; 3],
}

impl Curve {
    pub(super) fn points(self) -> [Vec2; 3] {
        [self.points[0].to_vec2(), self.points[1].to_vec2(), self.points[2].to_vec2()]
    }
}

#[derive(Clone, Copy, ShaderData)]
pub(super) struct Glyph {
    pub(super) min: Vec2,
    pub(super) max: Vec2,
    pub(super) start: u32,
    pub(super) count: u32,
    pub(super) field_start: u32,
    pub(super) field_size: UVec2,
}

pub(super) const FIELD_RESOLUTION: f32 = 48.0;
pub(super) const FIELD_PADDING: f32 = 1.0;

impl Glyph {
    pub(super) fn field_distance(self, words: &[u32], point: Vec2) -> f32 {
        let grid = (point - self.min + FIELD_PADDING) * FIELD_RESOLUTION;
        let limit = vec2(self.field_size.x as f32 - 1.0, self.field_size.y as f32 - 1.0);
        let bounded = grid.clamp_num(Vec2::ZERO, limit);
        let base = bounded.floor().min(limit - 1.0);
        let blend = bounded - base;
        let x = base.x as u32;
        let y = base.y as u32;
        let top = field_sample(words, self.field_start, self.field_size.x, x, y) * (1.0 - blend.x)
            + field_sample(words, self.field_start, self.field_size.x, x + 1, y) * blend.x;
        let bottom = field_sample(words, self.field_start, self.field_size.x, x, y + 1) * (1.0 - blend.x)
            + field_sample(words, self.field_start, self.field_size.x, x + 1, y + 1) * blend.x;
        top * (1.0 - blend.y) + bottom * blend.y + grid.distance(bounded) / FIELD_RESOLUTION
    }
}

fn field_sample(words: &[u32], start: u32, width: u32, x: u32, y: u32) -> f32 {
    let index = y * width + x;
    let pair = F16x2::read(words, (start + index / 2) as usize).to_vec2();
    if index & 1 == 0 { pair.x } else { pair.y }
}

#[derive(Clone, Copy, PartialEq, ShaderData)]
pub(super) struct PlacedGlyph {
    pub x: f32,
    pub y: f32,
    pub glyph: u32,
    pub left: f32,
    pub right: f32,
}

fn quadratic(start: Vec2, control: Vec2, end: Vec2, t: f32) -> Vec2 {
    let inverse = 1.0 - t;
    start * inverse * inverse + control * 2.0 * inverse * t + end * t * t
}

fn quadratic_roots(a: f32, b: f32, c: f32) -> [f32; 2] {
    if a == 0.0 {
        return [if b == 0.0 { -1.0 } else { -c / b }, -1.0];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return [-1.0; 2];
    }
    if discriminant == 0.0 {
        return [-0.5 * b / a, -1.0];
    }
    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
    [q / a, c / q]
}

#[inline(never)]
fn curve_winding(start: Vec2, control: Vec2, end: Vec2, point: Vec2) -> i32 {
    let a = start.y - 2.0 * control.y + end.y;
    let b = 2.0 * (control.y - start.y);
    let c = start.y - point.y;
    if a != 0.0 && b * b - 4.0 * a * c <= 0.0 {
        return 0;
    }
    let crossing = |t: f32| {
        let direction = 2.0 * a * t + b;
        let crosses = if direction > 0.0 { (0.0..1.0).contains(&t) } else { t > 0.0 && t <= 1.0 };
        if crosses && quadratic(start, control, end, t).x > point.x {
            i32::from(direction > 0.0) - i32::from(direction < 0.0)
        } else {
            0
        }
    };
    let roots = quadratic_roots(a, b, c);
    crossing(roots[0]) + crossing(roots[1])
}

fn curve_distance([start, control, end]: [Vec2; 3], point: Vec2, best: f32) -> (f32, i32) {
    let min = start.min(control).min(end);
    let max = start.max(control).max(end);
    let winding = if point.y >= min.y && point.y <= max.y { curve_winding(start, control, end, point) } else { 0 };
    if (point - point.clamp_num(min, max)).length_squared() >= best {
        return (best, winding);
    }
    let a = start - control * 2.0 + end;
    let b = (control - start) * 2.0;
    let c = start - point;
    let mut distance = best.min(c.length_squared()).min((end - point).length_squared());
    if a.length_squared() == 0.0 {
        let t = (-c.dot(b) / b.length_squared().max(f32::MIN_POSITIVE)).clamp_num(0.0, 1.0);
        return (distance.min((c + b * t).length_squared()), winding);
    }
    let coefficients = [2.0 * a.dot(a), 3.0 * a.dot(b), b.dot(b) + 2.0 * a.dot(c), b.dot(c)];
    let evaluate = |t: f32| ((coefficients[0] * t + coefficients[1]) * t + coefficients[2]) * t + coefficients[3];
    let turns = quadratic_roots(3.0 * coefficients[0], 2.0 * coefficients[1], coefficients[2]);
    let intervals = [0.0, turns[0].min(turns[1]).clamp_num(0.0, 1.0), turns[0].max(turns[1]).clamp_num(0.0, 1.0), 1.0];
    for index in 0..3 {
        let mut lo = intervals[index];
        let mut hi = intervals[index + 1];
        distance = distance.min((quadratic(start, control, end, hi) - point).length_squared());
        let negative = evaluate(lo) < 0.0;
        if lo >= hi || negative == (evaluate(hi) < 0.0) {
            continue;
        }
        let mut t = f32::midpoint(lo, hi);
        // Newton steps stay bracketed within a monotone interval of the stationary cubic.
        for _ in 0..24 {
            let value = evaluate(t);
            if value == 0.0 {
                break;
            }
            if negative == (value < 0.0) {
                lo = t;
            } else {
                hi = t;
            }
            let slope = (3.0 * coefficients[0] * t + 2.0 * coefficients[1]) * t + coefficients[2];
            let next = t - value / slope;
            if (next - t).abs() <= f32::EPSILON * 2.0 {
                break;
            }
            t = if next > lo && next < hi { next } else { f32::midpoint(lo, hi) };
        }
        distance = distance.min((quadratic(start, control, end, t) - point).length_squared());
    }
    (distance, winding)
}

pub(super) fn glyph_distance(curves: &[u32], start: u32, count: u32, point: Vec2) -> f32 {
    let mut distance_squared = f32::MAX;
    let mut winding = 0;
    // Rust-GPU cannot lower this runtime slice iterator without a pointer-to-integer conversion.
    for index in 0..count {
        let points = Curve::read(curves, start as usize + index as usize * Curve::WORDS).points();
        let (distance, edge_winding) = curve_distance(points, point, distance_squared);
        distance_squared = distance;
        winding += edge_winding;
    }
    distance_squared.sqrt() * if winding == 0 { 1.0 } else { -1.0 }
}
