//! Entry furniture and the five-tier lounge display, authored as whole objects.
use super::{Hit, Material, capsule, cylinder, rounded_box, taper};
use isthmus::prelude::*;
const IRON: Vec3 = vec3(0.11, 0.12, 0.13);
const WALNUT: Vec3 = vec3(0.33, 0.25, 0.19);

pub(super) fn shoe_storage(p: Vec3, size: Vec3) -> Hit {
    let body = rounded_box(p - Vec3::Z * 0.035, size - vec3(0.0, 0.0, 0.07), 0.008);
    let folded = vec3(p.x.abs(), p.y, p.z);
    let row = if p.z < 0.02 { -0.215 } else { 0.225 };
    let door = rounded_box(
        folded - vec3(size.x * 0.25, size.y * 0.5 + 0.003, row),
        vec3(size.x * 0.5 - 0.012, 0.018, 0.425),
        0.003,
    );
    let notch =
        rounded_box(folded - vec3(size.x * 0.25, size.y * 0.5 + 0.005, row + 0.20), vec3(0.075, 0.04, 0.027), 0.01);
    let legs = rounded_box(
        vec3(p.x.abs() - size.x * 0.45, p.y.abs() - size.y * 0.36, p.z + size.z * 0.46),
        vec3(0.028, 0.028, 0.08),
        0.003,
    );
    let inset = rounded_box(p - vec3(0.0, size.y * 0.5, 0.02), vec3(size.x - 0.025, 0.022, 0.89), 0.002);
    Hit::new(body.max(-inset).min(door.max(-notch)).min(legs), Material::Paint, vec3(0.29, 0.31, 0.33)).union(Hit::new(
        rounded_box(p - vec3(0.0, size.y * 0.5 - 0.02, 0.02), vec3(size.x - 0.03, 0.012, 0.88), 0.002),
        Material::Paint,
        IRON,
    ))
}

pub(super) fn mirror(p: Vec3, size: Vec3) -> Hit {
    // Round the silhouette in the wall plane, independently of its thin depth.
    let outer = rounded_box(vec3(p.x, p.z, p.y), vec3(size.x, size.z, 0.30), 0.075).max(p.y.abs() - 0.016);
    let inner = rounded_box(vec3(p.x, p.z, 0.0), vec3(size.x - 0.016, size.z - 0.016, 0.30), 0.070);
    let glass = inner.max((p.y - 0.012).abs() - 0.003);
    Hit::new(outer.max(-inner), Material::Metal, IRON).union(Hit::new(glass, Material::Mirror, vec3(0.94, 0.96, 0.96)))
}

fn book(p: Vec3, size: Vec3, tint: Vec3) -> Hit {
    let cover = rounded_box(p, size, 0.002);
    let page_size = size - vec3(0.006, 0.007, 0.009);
    let pages = rounded_box(p + Vec3::Y * 0.002, page_size, 0.001);
    // Leave the spine intact; expose the page block at the top and fore-edge.
    let recess = rounded_box(p + Vec3::Y * 0.012 + Vec3::Z * 0.009, page_size + vec3(0.0, 0.026, 0.020), 0.001);
    let cover = cover.max(-recess);
    let band = (p.z.abs() - size.z * 0.36).abs() < 0.002;
    let title =
        p.y > size.y * 0.42 && p.x.abs() < size.x * 0.29 && p.z.abs() < size.z * 0.22 && (p.z * 530.0).sin() > 0.25;
    let band = band || title;
    Hit::new(cover, Material::Fabric, if band { vec3(0.71, 0.61, 0.37) } else { tint }).union(Hit::new(
        pages,
        Material::Paint,
        vec3(0.78, 0.75, 0.64),
    ))
}
fn succulent(p: Vec3, scale: f32) -> Hit {
    let q = p / scale;
    let pot = taper(q, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.065), 0.033, 0.045).max(-q.z).max(q.z - 0.065);
    let mut leaves = super::FAR;
    for i in 0..7 {
        let (sn, cs) = (i as f32 * 2.399_963).sin_cos();
        let r = super::unrotate_z(q - Vec3::Z * 0.08, vec2(sn, cs));
        let blade = vec3((r.x - 0.026) * 0.8 - r.z * 0.6, r.y, (r.x - 0.026) * 0.6 + r.z * 0.8);
        leaves = leaves.min(((blade / vec3(0.018, 0.012, 0.070)).length() - 1.0) * 0.012);
    }
    Hit::new(pot * scale, Material::Ceramic, vec3(0.86, 0.85, 0.79)).union(Hit::new(
        leaves * scale,
        Material::Foliage,
        vec3(0.34, 0.44, 0.20),
    ))
}
fn dragon(p: Vec3, scale: f32) -> f32 {
    let q = p / scale;
    let body = taper(q, vec3(0.0, 0.0, 0.018), vec3(0.0, -0.005, 0.055), 0.022, 0.013);
    let neck = capsule(q, vec3(0.0, -0.005, 0.055), vec3(0.0, 0.014, 0.085), 0.009);
    let head = rounded_box(q - vec3(0.0, 0.024, 0.083), vec3(0.022, 0.029, 0.019), 0.005);
    let horns = taper(vec3(q.x.abs(), q.y, q.z), vec3(0.009, 0.015, 0.090), vec3(0.013, 0.002, 0.109), 0.004, 0.0005);
    let w = vec3(q.x.abs(), q.y, q.z);
    let span = ((w.x - 0.012) / 0.052).clamp(0.0, 1.0);
    let top = 0.080 + 0.018 * span;
    let lower = 0.030 + 0.028 * span + 0.008 * (span * 9.42).sin().abs();
    let membrane = (w.y + 0.012 + w.x * 0.12).abs() - 0.002;
    let wings = membrane.max(lower - w.z).max(w.z - top).max(0.012 - w.x).max(w.x - 0.064) * 0.7;
    let spars = capsule(w, vec3(0.012, -0.013, 0.065), vec3(0.064, -0.020, 0.098), 0.003);
    let tail = capsule(q, vec3(0.0, -0.012, 0.025), vec3(0.026, -0.041, 0.011), 0.006);
    body.min(neck).min(head).min(horns).min(wings).min(spars).min(tail).min(cylinder(q, 0.036, 0.008)) * scale
}
fn display(p: Vec3, tier: i32) -> Hit {
    let mut hit = Hit::new(super::FAR, Material::Wood, WALNUT);
    if tier == 0 || tier == 1 || tier == 4 {
        for i in 0..6 {
            let f = i as f32;
            let height = (if tier == 4 { 0.26 } else { 0.22 }) + 0.025 * (f * 2.1 + tier as f32).sin();
            let center = vec3((if tier == 4 { -0.18 } else { -0.26 }) + f * 0.044, -0.025, height * 0.5 + 0.018);
            let tint = if i % 3 == 0 {
                vec3(0.25, 0.39, 0.45)
            } else if i % 3 == 1 {
                vec3(0.62, 0.58, 0.46)
            } else {
                vec3(0.46, 0.17, 0.19)
            };
            hit = hit.union(book(p - center, vec3(0.041, 0.18, height), tint));
        }
        if tier == 0 {
            hit = hit.union(succulent(p - vec3(0.11, 0.015, 0.018), 1.2));
            hit = hit.union(succulent(p - vec3(0.25, -0.025, 0.018), 0.8));
        } else {
            let end = vec3(p.x.abs() - 0.28, p.y, p.z - 0.018);
            let triangular = rounded_box(end - Vec3::Z * 0.11, vec3(0.11, 0.19, 0.22), 0.003)
                .max((end.x - end.z * 0.42 + 0.025) * 0.92);
            let trim = (end.z * 0.8 - end.x * 0.6 - 0.04).abs() < 0.005;
            hit = hit.union(Hit::new(
                triangular,
                Material::Stone,
                if trim { vec3(0.44, 0.47, 0.48) } else { vec3(0.16, 0.19, 0.20) },
            ));
            hit =
                hit.union(Hit::new(dragon(p - vec3(-0.28, 0.0, 0.17), 1.25), Material::Stone, vec3(0.20, 0.23, 0.24)));
        }
        hit = hit.union(Hit::new(dragon(p - vec3(0.25, 0.0, 0.02), 1.5), Material::Metal, vec3(0.32, 0.29, 0.23)));
    } else if tier == 2 {
        for i in 0..3 {
            hit = hit.union(book(
                p - vec3(0.0, 0.0, 0.035 + i as f32 * 0.028),
                vec3(0.23, 0.17, 0.025),
                vec3(0.50 - i as f32 * 0.15, 0.18, 0.20 + i as f32 * 0.15),
            ));
        }
        let cube = p - vec3(0.0, 0.0, 0.16);
        let facet = (cube.x.abs() + cube.y.abs() + cube.z.abs() - 0.11) * 0.57735;
        hit = hit.union(Hit::new(
            rounded_box(cube, Vec3::splat(0.14), 0.002).max(facet),
            Material::Stone,
            vec3(0.20, 0.25, 0.25),
        ));
        // Paired articulated gauntlets: cuffs, palms and four segmented fingers.
        let glove = vec3(p.x.abs() - 0.235, p.y, p.z);
        let cuff = rounded_box(glove - vec3(0.0, -0.04, 0.065), vec3(0.10, 0.12, 0.08), 0.012);
        let palm = rounded_box(glove - vec3(0.0, 0.03, 0.05), vec3(0.10, 0.08, 0.045), 0.014);
        let finger_x = glove.x - ((glove.x / 0.025 - 0.5).round().clamp(-2.0, 1.0) + 0.5) * 0.025;
        let fingers = capsule(vec3(finger_x, glove.y, glove.z), vec3(0.0, 0.06, 0.045), vec3(0.0, 0.115, 0.025), 0.010);
        let cuff = cuff.max(-rounded_box(glove - vec3(0.0, -0.06, 0.083), vec3(0.076, 0.095, 0.07), 0.012));
        let plates = ((glove.y + 0.03) * 200.0).sin() > 0.2;
        hit = hit.union(Hit::new(
            cuff.min(palm).min(fingers),
            Material::Metal,
            if plates { vec3(0.58, 0.57, 0.52) } else { vec3(0.31, 0.25, 0.12) },
        ));
    } else {
        // Two framed illustrations flank a planted terrarium and small figures.
        let q = vec3(p.x.abs() - 0.22, p.y + 0.07, p.z - 0.16);
        let frame = rounded_box(q, vec3(0.15, 0.022, 0.27), 0.003);
        let picture = rounded_box(q - Vec3::Y * 0.012, vec3(0.133, 0.003, 0.25), 0.001);
        let horse = ((q.xz() - vec2(0.0, -0.015)) / vec2(0.045, 0.058)).length() < 1.0;
        let neck = ((q.xz() - vec2(-0.025, 0.055)) / vec2(0.013, 0.051)).length() < 1.0;
        let branches = (q.x * 90.0 + q.z * 38.0 + (q.z * 50.0).sin()).sin().abs() < 0.12;
        let ink = horse || neck || (branches && q.z > 0.08);
        hit = hit.union(Hit::new(frame, Material::Metal, IRON)).union(Hit::new(
            picture,
            Material::Paint,
            if ink { vec3(0.12, 0.19, 0.21) } else { vec3(0.77, 0.73, 0.59) },
        ));
        hit = hit.union(succulent(p - vec3(0.0, -0.01, 0.022), 1.65));
        // Open clear-vessel silhouette: polished lip and base expose the foliage.
        let jar = vec2(p.xy().length() - 0.086, p.z - 0.285).length() - 0.005;
        let base = cylinder(p - Vec3::Z * 0.022, 0.086, 0.010);
        hit = hit.union(Hit::new(jar.min(base), Material::Ceramic, vec3(0.65, 0.75, 0.69)));
        for i in 0..3 {
            hit = hit.union(Hit::new(
                dragon(p - vec3(-0.23 + i as f32 * 0.23, 0.105, 0.025), 0.70),
                Material::Metal,
                vec3(0.62, 0.47, 0.24),
            ));
        }
    }
    hit
}

pub(super) fn shelf(p: Vec3, size: Vec3) -> Hit {
    let folded = vec3(p.x.abs(), p.y.abs(), p.z);
    let uprights = rounded_box(folded - vec3(size.x * 0.48, size.y * 0.43, 0.0), vec3(0.018, 0.022, size.z), 0.003);
    let braces = capsule(p, vec3(-0.35, -0.135, -0.12), vec3(0.35, -0.135, 0.22), 0.006).min(capsule(
        p,
        vec3(0.35, -0.135, -0.12),
        vec3(-0.35, -0.135, 0.22),
        0.006,
    ));
    let top_rail = rounded_box(
        vec3(p.x.abs() - size.x * 0.48, p.y, p.z - size.z * 0.5 + 0.02),
        vec3(0.018, size.y * 0.88, 0.022),
        0.002,
    );
    let mut hit = Hit::new(uprights.min(braces).min(top_rail), Material::Metal, IRON);
    for tier in 0..5 {
        let q = p - Vec3::Z * (-0.80 + tier as f32 * 0.36);
        let board = rounded_box(q, vec3(size.x - 0.025, size.y, 0.025), 0.003);
        hit = hit.union(Hit::new(board, Material::Wood, WALNUT));
        let bolt = (vec3(p.x.abs() - size.x * 0.48, p.y.abs() - size.y * 0.465, q.z)).length() - 0.006;
        hit = hit.union(Hit::new(bolt, Material::Metal, vec3(0.36, 0.37, 0.37)));
        let bounds = super::box_sdf(q - Vec3::Z * 0.16, vec3(0.74, 0.33, 0.32));
        if bounds <= hit.distance {
            hit = hit.union(display(q, tier));
        }
    }
    hit
}

pub(super) fn swords(p: Vec3, _size: Vec3) -> Hit {
    let mut hit = Hit::new(super::FAR, Material::Metal, vec3(0.65, 0.68, 0.69));
    for i in 0..2 {
        let q = p - vec3(if i == 0 { -0.10 } else { 0.10 }, 0.0, if i == 0 { 0.18 } else { -0.27 });
        let z = if i == 0 { q.z } else { -q.z };
        let point = vec3(q.x, q.y, z);
        let t = ((0.28 - z) / 1.0).clamp(0.0, 1.0);
        let width = (if i == 0 { 0.039 } else { 0.027 }) * (1.0 - t.powi(8));
        // Diamond section, shouldered ricasso and a recessed central fuller.
        let bevel = (q.y.abs() + q.x.abs() * 0.20 - 0.010).max(q.x.abs() - width);
        let blade = bevel.max(z - 0.28).max(-0.72 - z) * 0.72;
        let fuller = rounded_box(point - vec3(0.0, 0.010, -0.13), vec3(0.009, 0.008, 0.71), 0.003);
        let blade = blade.max(-fuller);
        let g = vec3(point.x.abs(), point.y, point.z);
        let guard = taper(g, vec3(0.015, 0.0, 0.30), vec3(0.08, 0.0, if i == 0 { 0.245 } else { 0.31 }), 0.016, 0.010)
            .min(taper(g, vec3(0.08, 0.0, if i == 0 { 0.245 } else { 0.31 }), vec3(0.125, 0.0, 0.22), 0.010, 0.004));
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
            .union(Hit::new(
                blade.min(guard).min(ring).min(collar),
                Material::Metal,
                if rune {
                    vec3(0.20, 0.24, 0.25)
                } else if i == 0 {
                    vec3(0.72, 0.74, 0.74)
                } else {
                    vec3(0.31, 0.33, 0.35)
                },
            ))
            .union(Hit::new(
                grip,
                Material::Fabric,
                if wraps { vec3(0.27, 0.24, 0.19) } else { vec3(0.13, 0.115, 0.09) },
            ));
    }
    hit
}
