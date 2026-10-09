use super::{Kind, Material, Sample};
use core::f32::consts::{PI, TAU};
use isthmus::prelude::*;

/// Pointed leaf blade: a tapered lens extruded along a cupped, drooping sheet.
/// Normalize the deformation's gradient bound to keep sphere steps conservative.
fn leaf(p: Vec3, length: f32, width: f32) -> f32 {
    let t = (p.x / length).max_num(-1.0).min_num(1.0);
    let profile = width * (1.0 - t * t).max_num(0.0);
    let edge = (p.y.abs() - profile).max_num(p.x.abs() - length);
    let sheet = p.z + 0.18 * p.x * t - 0.32 * p.y.abs();
    let d = vec2(edge, sheet.abs() - 0.0025);
    (d.max_num(Vec2::ZERO).length() + d.max_element().min_num(0.0)) * 0.55
}
fn flower(p: Vec3, radius: f32) -> f32 {
    // Polar repetition produces five overlapping petals, with a cupped profile.
    let angle = p.y.atan2(p.x);
    let sector = TAU / 5.0;
    let a = angle - sector * (angle / sector).round();
    let r = p.xy().length();
    let q = vec3(r * a.cos() - radius * 0.45, r * a.sin(), p.z - 0.16 * r);
    let outline = ((q.xy() / vec2(radius * 0.65, radius * 0.40)).length() - 1.0) * radius * 0.40;
    let shell = vec2(outline, q.z.abs() - 0.0025);
    let petal = shell.max_num(Vec2::ZERO).length() + shell.max_element().min_num(0.0);
    petal.min_num(p.length() - radius * 0.18) * 0.85
}
#[inline(never)]
pub(super) fn plant<S: Sample>(kind: Kind, p: Vec3, size: Vec3) -> S {
    let blossom = kind == Kind::Blossom;
    let scale = size.x;
    let bottom = -size.z * 0.5;
    let top = bottom + size.z * if blossom { 0.24 } else { 0.16 };
    let pot = if blossom {
        let shoulder = bottom + size.z * 0.17;
        super::taper(p, vec3(0.0, 0.0, bottom), vec3(0.0, 0.0, shoulder), scale * 0.065, scale * 0.105)
            .min_num(super::taper(p, vec3(0.0, 0.0, shoulder), vec3(0.0, 0.0, top), scale * 0.105, scale * 0.055))
            .max_num(bottom - p.z)
            .max_num(p.z - top)
            .max_num(-super::cylinder(p - Vec3::Z * top, scale * 0.043, 0.05))
    } else {
        super::taper(p, vec3(0.0, 0.0, bottom), vec3(0.0, 0.0, top), scale * 0.14, scale * 0.18)
            .max_num(bottom - p.z)
            .max_num(p.z - top)
            .max_num(-super::taper(
                p,
                vec3(0.0, 0.0, bottom + 0.025),
                vec3(0.0, 0.0, top + 0.03),
                scale * 0.11,
                scale * 0.155,
            ))
    };
    let soil = super::cylinder(p - Vec3::Z * (top - 0.025), scale * if blossom { 0.043 } else { 0.145 }, 0.025);
    let hit =
        S::new(pot, || (Material::Ceramic, if blossom { vec3(0.19, 0.14, 0.12) } else { vec3(0.78, 0.75, 0.67) }))
            .union(S::new(soil, || (Material::Soil, vec3(0.22, 0.17, 0.10))));
    if blossom {
        return hit;
    }
    if kind == Kind::Palm || kind == Kind::SillPlant {
        return S::new(pot, || (Material::Ceramic, vec3(0.10, 0.115, 0.11)))
            .union(S::new(soil, || (Material::Soil, vec3(0.22, 0.17, 0.10))));
    }
    let wood = vec3(0.36, 0.28, 0.19);
    let trunk =
        super::taper(p, vec3(0.0, 0.0, top - 0.01), vec3(0.025, -0.025, size.z * 0.38), scale * 0.022, scale * 0.003);
    hit.union(S::new(trunk, || (Material::Wood, wood)))
}
#[inline(never)]
pub(super) fn hanging_planter<S: Sample>(p: Vec3, size: Vec3) -> S {
    let top = size.z * 0.5;
    let mount = super::rounded_box(p - vec3(0.0, -0.035, top - 0.009), vec3(0.045, 0.035, 0.018), 0.004);
    let hook = super::capsule(p, vec3(0.0, -0.035, top - 0.01), vec3(0.0, 0.015, top - 0.045), 0.003);
    S::new(mount.min_num(hook), || (Material::Metal, vec3(0.18, 0.17, 0.14)))
}
#[inline(never)]
pub(super) fn leaf_frame(p: Vec3, size: Vec3) -> Vec3 {
    let lean = -(size.z / size.x).max_num(-0.95).min_num(0.95);
    let upright = (1.0 - lean * lean).sqrt();
    vec3(p.x * upright + p.z * lean, p.y, p.z * upright - p.x * lean)
}
#[inline(never)]
pub(super) fn leaf_normal(p: Vec3, size: Vec3) -> Vec3 {
    let q = leaf_frame(p, size);
    let length = size.x * 0.5;
    let width = size.y * 0.5;
    let sign = if q.y >= 0.0 { 1.0 } else { -1.0 };
    let sheet = q.z + 0.18 * q.x * q.x / length - 0.32 * q.y.abs();
    let edge = q.y.abs() - width * (1.0 - (q.x / length).powi(2));
    let gradient = if edge > sheet.abs() - 0.0025 {
        vec3(2.0 * width * q.x / (length * length), sign, 0.0)
    } else {
        vec3(0.36 * q.x / length, -0.32 * sign, 1.0) * if sheet >= 0.0 { 1.0 } else { -1.0 }
    };
    // The inverse is the transpose of the leaf frame.
    let lean = -(size.z / size.x).max_num(-0.95).min_num(0.95);
    let upright = (1.0 - lean * lean).sqrt();
    vec3(gradient.x * upright - gradient.z * lean, gradient.y, gradient.z * upright + gradient.x * lean).normalize()
}
#[inline(never)]
pub(super) fn blade<S: Sample>(kind: Kind, p: Vec3, size: Vec3) -> S {
    if kind == Kind::Stem {
        return S::new(super::capsule(p, -size * 0.5, size * 0.5, super::stem_radius(size)), || {
            (Material::Wood, vec3(0.36, 0.28, 0.19))
        });
    }
    if kind == Kind::Petal {
        let lean = (size.z / size.x).max_num(0.15).min_num(0.95);
        let upright = (1.0 - lean * lean).sqrt();
        return S::new(flower(vec3(p.x * upright + p.z * lean, p.y, p.z * upright - p.x * lean), size.x * 0.5), || {
            (
                Material::Fabric,
                vec3(0.98, 0.92, 0.93).lerp(vec3(0.72, 0.30, 0.46), (1.0 - p.length() / (size.x * 0.28)).max_num(0.0)),
            )
        });
    }
    let q = leaf_frame(p, size);
    let length = size.x * 0.5;
    let width = size.y * 0.5;
    let profile = width * (1.0 - (q.x / length).powi(2)).max_num(0.02);
    let margin = (q.y.abs() / profile).max_num(0.0).min_num(1.0);
    let vein = (1.0 - q.y.abs() / 0.0015).max_num(0.0).min_num(1.0);
    let tint = vec3(0.25, 0.36, 0.18).lerp(vec3(0.61, 0.65, 0.43), margin.powf(5.0) * 0.55 + vein * 0.08);
    S::new(leaf(q, length, width), || (Material::Foliage, tint))
}

#[inline(never)]
pub(super) fn table_lamp<S: Sample>(p: Vec3) -> S {
    // Woven oval shade, solid warm inner diffuser and a small dark foot.
    let q = p - Vec3::Z * 0.18;
    let oval = ((q / vec3(0.065, 0.065, 0.16)).length() - 1.0) * 0.065;
    let angle = q.y.atan2(q.x);
    let warp = (angle * 36.0 + q.z * 24.0).sin().abs();
    let weft = (q.z * 210.0 - angle * 2.0).sin().abs();
    let weave = oval.abs() - 0.0035;
    let shade = weave.max_num((0.45 - warp.max_num(weft)) * 0.003).min_num(oval + 0.005);
    let foot = super::cylinder(p - Vec3::Z * 0.012, 0.045, 0.024);
    S::new(shade.min_num(foot), || {
        (
            if shade < foot { Material::Fabric } else { Material::Metal },
            if shade < foot {
                vec3(0.82, 0.70, 0.47) * (0.58 + 0.42 * warp.max_num(weft))
            } else {
                vec3(0.22, 0.19, 0.15)
            },
        )
    })
}
#[inline(never)]
pub(super) fn table_flowers<S: Sample>(p: Vec3) -> S {
    let pot =
        super::cylinder(p - Vec3::Z * 0.027, 0.033, 0.054).max_num(-super::cylinder(p - Vec3::Z * 0.038, 0.027, 0.045));
    let mut leaves = super::FAR;
    let mut petals = super::FAR;
    let mut i = 0;
    while i < 7 {
        let phase = i as f32 * 2.399_963;
        let (sin, cos) = phase.sin_cos();
        let center = vec3(cos * 0.033, sin * 0.033, 0.09 + i as f32 * 0.003);
        let q = super::unrotate_z(p - center, vec2(sin, cos));
        leaves = leaves.min_num(leaf(q - vec3(0.0, 0.0, -0.025), 0.026, 0.012));
        leaves = leaves.min_num(super::capsule(p, vec3(0.0, 0.0, 0.035), center, 0.0015));
        petals = petals.min_num(flower(q, 0.017));
        i += 1;
    }
    let distance = pot.min_num(leaves).min_num(petals);
    let (material, tint) = if petals < pot.min_num(leaves) {
        (Material::Fabric, vec3(0.72, 0.25, 0.39))
    } else if leaves < pot {
        (Material::Foliage, vec3(0.21, 0.34, 0.15))
    } else {
        (Material::Ceramic, vec3(0.63, 0.62, 0.57))
    };
    S::new(distance, || (material, tint))
}
#[inline(never)]
pub(super) fn sill_arrangement<S: Sample>(p: Vec3, size: Vec3) -> S {
    // A single narrow amber vase with grasses and flowering stems, distinct
    // from both the ficus and the floor-standing cherry blossom arrangement.
    let bottom = -size.z * 0.5;
    let top = bottom + size.z * 0.32;
    let vase = super::taper(p, vec3(0.0, 0.0, bottom), vec3(0.0, 0.0, top), 0.032, 0.039)
        .max_num(bottom - p.z)
        .max_num(p.z - top)
        .max_num(-super::cylinder(p - Vec3::Z * (top - 0.055), 0.031, 0.14));
    let mut grasses = super::FAR;
    let mut blossoms = super::FAR;
    let mut i = 0;
    while i < 12 {
        let phase = i as f32 * 2.399_963;
        let (sin, cos) = phase.sin_cos();
        let start = vec3(cos * 0.018, sin * 0.018, top - 0.03);
        let middle = vec3(cos * 0.035, sin * 0.035, top + size.z * 0.24);
        let end = vec3(cos * 0.085, sin * 0.055, size.z * (0.37 + 0.07 * phase.sin()));
        grasses = grasses
            .min_num(super::taper(p, start, middle, 0.0025, 0.002))
            .min_num(super::taper(p, middle, end, 0.002, 0.0005));
        if i < 6 {
            let q = super::unrotate_z(p - end, vec2(sin, cos));
            blossoms = blossoms.min_num(flower(q, 0.016));
            grasses = grasses.min_num(leaf(q - vec3(0.0, 0.0, -0.05), 0.031, 0.006));
        }
        i += 1;
    }
    let distance = vase.min_num(grasses).min_num(blossoms);
    let (material, tint) = if blossoms < vase.min_num(grasses) {
        (Material::Fabric, vec3(0.71, 0.33, 0.52))
    } else if grasses < vase {
        (Material::Foliage, vec3(0.49, 0.54, 0.22))
    } else {
        (Material::Ceramic, vec3(0.66, 0.57, 0.18))
    };
    S::new(distance, || (material, tint))
}
/// Signed triangle profile extruded into a thin wooden silhouette.
fn triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let edge0 = b - a;
    let edge1 = c - b;
    let edge2 = a - c;
    let v0 = p - a;
    let v1 = p - b;
    let v2 = p - c;
    let d0 = v0 - edge0 * (v0.dot(edge0) / edge0.length_squared()).max_num(0.0).min_num(1.0);
    let d1 = v1 - edge1 * (v1.dot(edge1) / edge1.length_squared()).max_num(0.0).min_num(1.0);
    let d2 = v2 - edge2 * (v2.dot(edge2) / edge2.length_squared()).max_num(0.0).min_num(1.0);
    let winding = if edge0.perp_dot(c - a) >= 0.0 { 1.0 } else { -1.0 };
    let inside =
        (winding * edge0.perp_dot(v0)).min_num(winding * edge1.perp_dot(v1)).min_num(winding * edge2.perp_dot(v2));
    d0.length_squared().min_num(d1.length_squared()).min_num(d2.length_squared()).sqrt()
        * if inside >= 0.0 { -1.0 } else { 1.0 }
}
/// A pointed, swept feather profile with a curved centerline and tapered width.
fn wing(p: Vec2, root: Vec2, tip: Vec2, width: f32, bend: f32) -> f32 {
    let axis = tip - root;
    let length = axis.length();
    let along = (p - root).dot(axis) / length;
    let fraction = (along / length).max_num(0.0).min_num(1.0);
    let bulge = 4.0 * fraction * (1.0 - fraction);
    let across = axis.perp_dot(p - root) / length - bend * bulge;
    (across.abs() - width * bulge * (1.1 - 0.7 * fraction)).max_num((-along).max_num(along - length)) * 0.5
}
fn swallow(p: Vec3) -> f32 {
    let profile = p.xz();
    let wings = wing(profile, vec2(-0.012, 0.015), vec2(-0.21, 0.15), 0.044, -0.032).min_num(wing(
        profile,
        vec2(0.012, 0.015),
        vec2(0.19, 0.08),
        0.037,
        0.027,
    ));
    let tail = wing(profile, vec2(-0.006, -0.045), vec2(-0.065, -0.19), 0.013, 0.006).min_num(wing(
        profile,
        vec2(0.006, -0.045),
        vec2(0.07, -0.17),
        0.014,
        -0.006,
    ));
    let head = triangle(profile, vec2(-0.02, 0.06), vec2(0.047, 0.105), vec2(0.025, 0.037));
    let outline = wings.min_num(tail).min_num(head);
    let shell = vec2(outline, p.y.abs() - 0.009);
    let carving = shell.max_num(Vec2::ZERO).length() + shell.max_element().min_num(0.0);
    let feather = super::capsule(p, vec3(-0.055, -0.009, 0.025), vec3(-0.145, -0.009, 0.105), 0.002)
        .min_num(super::capsule(p, vec3(0.055, -0.009, 0.025), vec3(0.135, -0.009, 0.09), 0.002));
    let carving = carving.max_num(-feather);
    let body = super::taper(p, vec3(0.0, -0.004, -0.09), vec3(0.0, -0.004, 0.065), 0.009, 0.014);
    super::blend(carving, body, 0.009)
}
fn elven_blade<S: Sample>(p: Vec3, length: f32, width: f32) -> S {
    let t = (p.x / length).max_num(0.0).min_num(1.0);
    let center = 0.006 - 0.004 * (t * PI).sin() + 0.030 * t.powi(5);
    let half_width = width * (1.0 + 0.18 * (t * PI).sin()) * (1.0 - t.powi(8));
    let edge = (p.z - center).abs() - half_width;
    let profile = edge.max_num(-p.x).max_num(p.x - length);
    let ridge = (1.0 - (p.z - center).abs() / half_width.max_num(0.001)).max_num(0.0);
    let depth = 0.002 + 0.004 * ridge;
    let distance = profile.max_num(p.y.abs() - depth) * 0.65;
    let scroll = center + half_width * (0.20 + 0.25 * (t * 9.0).sin());
    let engraving = (p.z - scroll).abs() < half_width * 0.42 && t < 0.93;
    let bevel = (1.0 - edge.abs() / 0.004).max_num(0.0);
    let blue = vec3(0.12, 0.14, 0.25).lerp(vec3(0.37, 0.39, 0.48), bevel * 0.7);
    S::new(distance, || (Material::Metal, if engraving { vec3(0.45, 0.40, 0.28) } else { blue }))
}

fn elven_fitting<S: Sample>(p: Vec3) -> S {
    // One rounded, swept bow instead of several pointed fins.
    let curve = |t: f32| vec3(-0.035 + 0.105 * t * t, 0.0, -0.025 + 0.060 * t);
    let mut guard = super::FAR;
    let mut i = 0;
    while i < 8 {
        let t = i as f32 / 8.0;
        guard =
            guard.min_num(super::taper(p, curve(t), curve(t + 0.125), 0.006 - t * 0.003, 0.006 - (t + 0.125) * 0.003));
        i += 1;
    }
    S::new(guard, || (Material::Metal, vec3(0.43, 0.38, 0.25)))
}

fn weapon<S: Sample>(p: Vec3) -> S {
    let guard = -0.29;
    let q = p - vec3(guard, 0.0, 0.0);
    let grip = super::taper(p, vec3(-0.56, 0.0, -0.006), vec3(-0.43, 0.0, 0.008), 0.010, 0.012).min_num(super::taper(
        p,
        vec3(-0.43, 0.0, 0.008),
        vec3(-0.32, 0.0, 0.0),
        0.012,
        0.009,
    ));
    let pommel = super::capsule(p, vec3(-0.565, 0.0, -0.012), vec3(-0.565, 0.0, 0.012), 0.007);
    let inlay = (p.z - 0.009).abs() < 0.0015;
    elven_blade::<S>(q, 0.87, 0.014)
        .union(elven_fitting(q))
        .union(S::new(grip, || (Material::Metal, if inlay { vec3(0.38, 0.34, 0.25) } else { vec3(0.12, 0.11, 0.13) })))
        .union(S::new(pommel, || (Material::Metal, vec3(0.20, 0.18, 0.20))))
}

fn naginata<S: Sample>(p: Vec3) -> S {
    let gold = vec3(0.43, 0.38, 0.25);
    let blue = vec3(0.10, 0.13, 0.27);
    let head = p - vec3(0.12, 0.0, 0.0);
    let shaft = super::taper(p, vec3(-1.02, 0.0, 0.0), vec3(0.12, 0.0, 0.0), 0.012, 0.010);
    let grip = p.x < -0.66;
    let wrap = (p.x * 77.0 + p.z.atan2(p.y)).sin() > 0.96;
    let stripe = p.z.abs() < 0.0018;
    let shaft_tint = if grip {
        if wrap { gold } else { vec3(0.095, 0.085, 0.08) }
    } else if stripe {
        gold
    } else {
        blue
    };
    let pommel = super::capsule(p, vec3(-1.015, 0.0, -0.009), vec3(-1.015, 0.0, 0.009), 0.014);
    elven_blade::<S>(head, 0.78, 0.019)
        .union(elven_fitting(head))
        .union(S::new(shaft, || (if grip { Material::Fabric } else { Material::Metal }, shaft_tint)))
        .union(S::new(pommel, || (Material::Metal, blue)))
}
#[inline(never)]
pub(super) fn wall_art<S: Sample>(p: Vec3, _size: Vec3) -> S {
    // Both tips face the kitchen; the lower weapon has a long pole and a separate blade.
    let upper = p - vec3(-0.08, 0.0, -0.10);
    let upper = vec3(-upper.x, upper.y, upper.z);
    let lower = p - vec3(-0.04, 0.0, -0.30);
    let lower = vec3(-lower.x, lower.y, lower.z);
    let blade = weapon::<S>(upper).union(naginata(lower));
    let mut birds = super::FAR;
    let mut i = 0;
    while i < 5 {
        let bird = p - vec3(-0.92 + i as f32 * 0.43, 0.0, 0.72 - i as f32 * 0.16);
        let angle = 0.95 + (i as f32 - 2.0) * 0.24;
        let (sin, cos) = angle.sin_cos();
        let q = vec3(bird.x * cos + bird.z * sin, bird.y, -bird.x * sin + bird.z * cos);
        if super::box_sdf(q, vec3(0.45, 0.05, 0.44)).max_num(0.0) * 0.3 <= birds.min_num(blade.distance()) {
            birds = birds.min_num(swallow(q / 0.72) * 0.72);
        }
        i += 1;
    }
    S::new(birds, || (Material::Wood, vec3(0.60, 0.37, 0.20))).union(blade)
}
