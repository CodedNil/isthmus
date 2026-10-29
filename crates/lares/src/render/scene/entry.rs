//! Entry furniture and the five-tier lounge display, authored as whole objects.
use super::{Material, Sample, capsule, cylinder, rounded_box, taper};
use isthmus::prelude::*;
const IRON: Vec3 = vec3(0.11, 0.12, 0.13);
const WALNUT: Vec3 = vec3(0.33, 0.25, 0.19);

#[inline(never)]
pub(super) fn shoe_storage<S: Sample>(p: Vec3, size: Vec3) -> S {
    // STÄLL: four flush tilt-out fronts, a shallow carcass and two front
    // feet. Dimensions include the feet; its back clears the skirting.
    let bottom = -size.z * 0.5 + 0.085;
    let height = size.z - 0.085;
    let center = bottom + height * 0.5;
    let upright =
        rounded_box(vec3(p.x.abs() - size.x * 0.5 + 0.009, p.y, p.z - center), vec3(0.018, size.y, height), 0.0015);
    let horizontal = rounded_box(
        p - vec3(0.0, 0.0, if p.z > center { size.z * 0.5 - 0.009 } else { bottom + 0.009 }),
        vec3(size.x, size.y, 0.018),
        0.0015,
    );
    let legs = rounded_box(
        vec3(p.x.abs() - size.x * 0.5 + 0.019, p.y - size.y * 0.5 + 0.019, p.z + size.z * 0.5 - 0.044),
        vec3(0.038, 0.038, 0.088),
        0.002,
    );
    let row_height = (height - 0.036 - 0.006) * 0.5;
    let row = if p.z > center { bottom + 0.018 + row_height * 1.5 + 0.006 } else { bottom + 0.018 + row_height * 0.5 };
    let q = vec3(p.x.abs() - (size.x - 0.036) * 0.25, p.y - size.y * 0.5 + 0.009, p.z - row);
    let panel = rounded_box(q, vec3((size.x - 0.044) * 0.5, 0.018, row_height), 0.0015);
    let handle = q - vec3(0.0, 0.0, row_height * 0.5 - 0.005);
    let cutout = rounded_box(handle, vec3(0.094, 0.05, 0.045), 0.009);
    let rim = rounded_box(vec3(handle.x, handle.z, 0.0), vec3(0.094, 0.045, 0.10), 0.009)
        .max_num((handle.y - 0.010).abs() - 0.0015)
        .max_num(-rounded_box(handle, vec3(0.089, 0.06, 0.040), 0.007))
        .max_num(q.z - row_height * 0.5);
    let inside =
        rounded_box(p - vec3(0.0, -0.016, center), vec3(size.x - 0.036, size.y - 0.036, height - 0.036), 0.001);
    S::new(upright.min_num(horizontal).min_num(legs).min_num(panel.max_num(-cutout)), || {
        (Material::Paint, vec3(0.23, 0.25, 0.27))
    })
    .union(S::new(inside, || (Material::Paint, vec3(0.075, 0.082, 0.09))))
    .union(S::new(rim, || (Material::Metal, vec3(0.40, 0.42, 0.43))))
}

#[inline(never)]
pub(super) fn mirror<S: Sample>(p: Vec3, size: Vec3) -> S {
    // Round the silhouette in the wall plane, independently of its thin depth.
    let outer = rounded_box(vec3(p.x, p.z, p.y), vec3(size.x, size.z, 0.30), 0.075).max_num(p.y.abs() - 0.016);
    let inner = rounded_box(vec3(p.x, p.z, 0.0), vec3(size.x - 0.016, size.z - 0.016, 0.30), 0.070);
    let glass = inner.max_num((p.y - 0.012).abs() - 0.003);
    S::new(outer.max_num(-inner), || (Material::Metal, IRON))
        .union(S::new(glass, || (Material::Mirror, vec3(0.94, 0.96, 0.96))))
}

fn succulent<S: Sample>(p: Vec3, scale: f32) -> S {
    let q = p / scale;
    let pot = taper(q, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.065), 0.033, 0.045).max_num(-q.z).max_num(q.z - 0.065);
    let mut leaves = super::FAR;
    let mut i = 0;
    while i < 7 {
        let (sn, cs) = (i as f32 * 2.399_963).sin_cos();
        let r = super::unrotate_z(q - Vec3::Z * 0.08, vec2(sn, cs));
        let blade = vec3((r.x - 0.026) * 0.8 - r.z * 0.6, r.y, (r.x - 0.026) * 0.6 + r.z * 0.8);
        leaves = leaves.min_num(((blade / vec3(0.018, 0.012, 0.070)).length() - 1.0) * 0.012);
        i += 1;
    }
    S::new(pot * scale, || (Material::Ceramic, vec3(0.86, 0.85, 0.79)))
        .union(S::new(leaves * scale, || (Material::Foliage, vec3(0.34, 0.44, 0.20))))
}
/// A few readable book spines, stacked volumes and planters. Keep the
/// five-tier silhouette without tiny figurines or individually carved titles.
fn display<S: Sample>(p: Vec3, tier: i32) -> S {
    let cell = ((p.x + 0.22) / 0.044).round().clamp_num(0.0, 6.0);
    let height = 0.20 + (cell * 1.7 + tier as f32).sin().abs() * 0.06;
    let book = p - vec3(-0.22 + cell * 0.044, -0.02, 0.018 + height * 0.5);
    let spine = rounded_box(book, vec3(0.037, 0.18, height), 0.002);
    let stripe = (book.z.abs() - height * 0.35).abs() < 0.004;
    let mut hit = S::new(spine, || {
        (
            Material::Fabric,
            if stripe {
                vec3(0.65, 0.58, 0.40)
            } else if cell < 2.0 {
                vec3(0.28, 0.38, 0.43)
            } else if cell < 4.0 {
                vec3(0.57, 0.24, 0.22)
            } else {
                vec3(0.58, 0.54, 0.40)
            },
        )
    });
    let stack = rounded_box(p - vec3(0.20, 0.0, 0.07), vec3(0.16, 0.20, 0.10), 0.003);
    hit = hit.union(S::new(stack, || (Material::Fabric, vec3(0.38, 0.32, 0.27))));
    if tier == 0 || tier == 3 {
        hit = hit.union(succulent(p - vec3(0.21, 0.0, 0.12), 1.05));
    }
    hit
}

#[inline(never)]
pub(super) fn shelf<S: Sample>(p: Vec3, size: Vec3) -> S {
    // The back stays against the wall; the front rail slopes inward toward
    // the top. Boards retain their width while each tier becomes shallower.
    let bottom = -size.z * 0.5;
    let back = -size.y * 0.43;
    let front_bottom = size.y * 0.43;
    let front_top = front_bottom - size.y * 0.30;
    let folded = vec3(p.x.abs(), p.y, p.z);
    let rear = capsule(folded, vec3(size.x * 0.48, back, bottom), vec3(size.x * 0.48, back, -bottom), 0.010);
    let front =
        capsule(folded, vec3(size.x * 0.48, front_bottom, bottom), vec3(size.x * 0.48, front_top, -bottom), 0.010);
    let uprights = rear.min_num(front);
    let braces = capsule(p, vec3(-size.x * 0.46, back, -0.12), vec3(size.x * 0.46, back, 0.22), 0.006)
        .min_num(capsule(p, vec3(size.x * 0.46, back, -0.12), vec3(-size.x * 0.46, back, 0.22), 0.006));
    let top_rail = rounded_box(
        vec3(p.x.abs() - size.x * 0.48, p.y + size.y * 0.15, p.z - size.z * 0.5 + 0.02),
        vec3(0.018, size.y * 0.60, 0.022),
        0.002,
    );
    let mut hit = S::new(uprights.min_num(braces).min_num(top_rail), || (Material::Metal, IRON));
    let mut tier = 0;
    while tier < 5 {
        let depth = size.y * (1.0 - tier as f32 * 0.075);
        let q = p - vec3(0.0, -(size.y - depth) * 0.5, -0.80 + tier as f32 * 0.36);
        let board = rounded_box(q, vec3(size.x - 0.025, depth, 0.025), 0.003);
        hit = hit.union(S::new(board, || (Material::Wood, WALNUT)));
        let bounds = super::box_sdf(q - Vec3::Z * 0.16, vec3(0.74, 0.33, 0.32));
        if bounds <= hit.distance() {
            hit = hit.union(display(q, tier));
        }
        tier += 1;
    }
    hit
}

#[inline(never)]
pub(super) fn swords<S: Sample>(p: Vec3, _size: Vec3) -> S {
    let mut hit = S::new(super::FAR, || (Material::Metal, vec3(0.65, 0.68, 0.69)));
    let mut weapon = 0;
    while weapon < 2 {
        let q = p - vec3(if weapon == 0 { -0.10 } else { 0.10 }, 0.0, if weapon == 0 { 0.18 } else { -0.27 });
        let z = if weapon == 0 { q.z } else { -q.z };
        let point = vec3(q.x, q.y, z);
        let progress = ((0.28 - z) / 1.0).max_num(0.0).min_num(1.0);
        let width = (if weapon == 0 { 0.039 } else { 0.027 }) * (1.0 - progress.powi(8));
        // Diamond section, shouldered ricasso and a recessed central fuller.
        let bevel = (q.y.abs() + q.x.abs() * 0.20 - 0.010).max_num(q.x.abs() - width);
        let blade = bevel.max_num(z - 0.28).max_num(-0.72 - z) * 0.72;
        let fuller = rounded_box(point - vec3(0.0, 0.010, -0.13), vec3(0.009, 0.008, 0.71), 0.003);
        let blade = blade.max_num(-fuller);
        let guard_point = vec3(point.x.abs(), point.y, point.z);
        let guard = taper(
            guard_point,
            vec3(0.015, 0.0, 0.30),
            vec3(0.08, 0.0, if weapon == 0 { 0.245 } else { 0.31 }),
            0.016,
            0.010,
        )
        .min_num(taper(
            guard_point,
            vec3(0.08, 0.0, if weapon == 0 { 0.245 } else { 0.31 }),
            vec3(0.125, 0.0, 0.22),
            0.010,
            0.004,
        ));
        let grip = taper(point, vec3(0.0, 0.0, 0.32), vec3(0.0, 0.0, 0.60), 0.016, 0.012);
        let ring = vec2(point.xz().distance(vec2(0.0, 0.64)) - 0.025, point.y).length() - 0.007;
        let collar = cylinder(vec3(point.x, point.y, (point.z - 0.46).abs() - 0.14), 0.019, 0.014);
        let wraps = (point.z * 240.0 + point.x * 20.0).sin() > 0.2;
        let rune = q.x.abs() < 0.023
            && q.x.abs() > 0.007
            && z > -0.50
            && z < 0.18
            && ((z * 125.0).sin() + (q.x * 280.0 + z * 80.0).sin()).abs() < 0.30;
        hit = hit
            .union(S::new(blade.min_num(guard).min_num(ring).min_num(collar), || {
                (
                    Material::Metal,
                    if rune {
                        vec3(0.20, 0.24, 0.25)
                    } else if weapon == 0 {
                        vec3(0.72, 0.74, 0.74)
                    } else {
                        vec3(0.31, 0.33, 0.35)
                    },
                )
            }))
            .union(S::new(grip, || {
                (Material::Fabric, if wraps { vec3(0.27, 0.24, 0.19) } else { vec3(0.13, 0.115, 0.09) })
            }));
        weapon += 1;
    }
    hit
}
