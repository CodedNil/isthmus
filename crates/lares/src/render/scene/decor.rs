use super::{Hit, Kind, Material};
use core::f32::consts::{PI, TAU};
use isthmus::prelude::*;

/// Pointed leaf blade: a tapered lens extruded along a cupped, drooping sheet.
/// Normalize the deformation's gradient bound to keep sphere steps conservative.
fn leaf(p: Vec3, length: f32, width: f32) -> f32 {
    let t = (p.x / length).clamp(-1.0, 1.0);
    let profile = width * (1.0 - t * t).max(0.0);
    let edge = (p.y.abs() - profile).max(p.x.abs() - length);
    let sheet = p.z + 0.18 * p.x * t - 0.32 * p.y.abs();
    let d = vec2(edge, sheet.abs() - 0.0025);
    (d.max(Vec2::ZERO).length() + d.max_element().min(0.0)) * 0.55
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
    let petal = shell.max(Vec2::ZERO).length() + shell.max_element().min(0.0);
    petal.min(p.length() - radius * 0.18) * 0.85
}
pub(super) fn plant(kind: Kind, p: Vec3, size: Vec3) -> Hit {
    let blossom = kind == Kind::Blossom;
    let scale = size.x;
    let bottom = -size.z * 0.5;
    let top = bottom + size.z * 0.16;
    let pot = super::taper(p, vec3(0.0, 0.0, bottom), vec3(0.0, 0.0, top), scale * 0.14, scale * 0.18)
        .max(bottom - p.z)
        .max(p.z - top)
        .max(-super::taper(p, vec3(0.0, 0.0, bottom + 0.025), vec3(0.0, 0.0, top + 0.03), scale * 0.11, scale * 0.155));
    let soil = super::cylinder(p - Vec3::Z * (top - 0.025), scale * 0.145, 0.025);
    let hit = Hit::new(pot, Material::Ceramic, if blossom { vec3(0.19, 0.14, 0.12) } else { vec3(0.78, 0.75, 0.67) })
        .union(Hit::new(soil, Material::Soil, vec3(0.22, 0.17, 0.10)));
    if kind == Kind::Palm || kind == Kind::SillPlant {
        return Hit::new(pot, Material::Ceramic, vec3(0.10, 0.115, 0.11)).union(Hit::new(
            soil,
            Material::Soil,
            vec3(0.22, 0.17, 0.10),
        ));
    }
    let wood = vec3(0.36, 0.28, 0.19);
    let trunk =
        super::taper(p, vec3(0.0, 0.0, top - 0.01), vec3(0.025, -0.025, size.z * 0.38), scale * 0.022, scale * 0.003);
    hit.union(Hit::new(trunk, Material::Wood, wood))
}
pub(super) fn leaf_frame(p: Vec3, size: Vec3) -> Vec3 {
    let lean = -(size.z / size.x).clamp(-0.95, 0.95);
    let upright = (1.0 - lean * lean).sqrt();
    vec3(p.x * upright + p.z * lean, p.y, p.z * upright - p.x * lean)
}
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
    let lean = -(size.z / size.x).clamp(-0.95, 0.95);
    let upright = (1.0 - lean * lean).sqrt();
    vec3(gradient.x * upright - gradient.z * lean, gradient.y, gradient.z * upright + gradient.x * lean).normalize()
}
pub(super) fn blade(kind: Kind, p: Vec3, size: Vec3) -> Hit {
    if kind == Kind::Stem {
        return Hit::new(
            super::capsule(p, -size * 0.5, size * 0.5, super::stem_radius(size)),
            Material::Wood,
            vec3(0.36, 0.28, 0.19),
        );
    }
    if kind == Kind::Petal {
        return Hit::new(
            flower(vec3(p.x * 0.8 + p.z * 0.6, p.y, p.z * 0.8 - p.x * 0.6), size.x * 0.5),
            Material::Fabric,
            vec3(0.93, 0.80, 0.82),
        );
    }
    let q = leaf_frame(p, size);
    let length = size.x * 0.5;
    let width = size.y * 0.5;
    let profile = width * (1.0 - (q.x / length).powi(2)).max(0.02);
    let margin = (q.y.abs() / profile).saturate();
    let vein = (1.0 - q.y.abs() / 0.0015).saturate();
    let tint = vec3(0.25, 0.36, 0.18).lerp(vec3(0.61, 0.65, 0.43), margin.powf(5.0) * 0.55 + vein * 0.08);
    Hit::new(leaf(q, length, width), Material::Foliage, tint)
}

pub(super) fn table_lamp(p: Vec3) -> Hit {
    // Woven oval shade, solid warm inner diffuser and a small dark foot.
    let q = p - Vec3::Z * 0.18;
    let oval = ((q / vec3(0.065, 0.065, 0.16)).length() - 1.0) * 0.065;
    let angle = q.y.atan2(q.x);
    let warp = (angle * 36.0 + q.z * 24.0).sin().abs();
    let weft = (q.z * 210.0 - angle * 2.0).sin().abs();
    let weave = oval.abs() - 0.0035;
    let shade = weave.max((0.45 - warp.max(weft)) * 0.003).min(oval + 0.005);
    let foot = super::cylinder(p - Vec3::Z * 0.012, 0.045, 0.024);
    Hit::new(
        shade.min(foot),
        if shade < foot { Material::Fabric } else { Material::Metal },
        if shade < foot { vec3(0.82, 0.70, 0.47) * (0.58 + 0.42 * warp.max(weft)) } else { vec3(0.22, 0.19, 0.15) },
    )
}
pub(super) fn table_flowers(p: Vec3) -> Hit {
    let pot =
        super::cylinder(p - Vec3::Z * 0.027, 0.033, 0.054).max(-super::cylinder(p - Vec3::Z * 0.038, 0.027, 0.045));
    let mut leaves = super::FAR;
    let mut petals = super::FAR;
    for i in 0..7 {
        let phase = i as f32 * 2.399_963;
        let (sin, cos) = phase.sin_cos();
        let center = vec3(cos * 0.033, sin * 0.033, 0.09 + i as f32 * 0.003);
        let q = super::unrotate_z(p - center, vec2(sin, cos));
        leaves = leaves.min(leaf(q - vec3(0.0, 0.0, -0.025), 0.026, 0.012));
        leaves = leaves.min(super::capsule(p, vec3(0.0, 0.0, 0.035), center, 0.0015));
        petals = petals.min(flower(q, 0.017));
    }
    let distance = pot.min(leaves).min(petals);
    let (material, tint) = if petals < pot.min(leaves) {
        (Material::Fabric, vec3(0.72, 0.25, 0.39))
    } else if leaves < pot {
        (Material::Foliage, vec3(0.21, 0.34, 0.15))
    } else {
        (Material::Ceramic, vec3(0.63, 0.62, 0.57))
    };
    Hit::new(distance, material, tint)
}
pub(super) fn sill_arrangement(p: Vec3, size: Vec3) -> Hit {
    // A single narrow amber vase with grasses and flowering stems, distinct
    // from both the ficus and the floor-standing cherry blossom arrangement.
    let bottom = -size.z * 0.5;
    let top = bottom + size.z * 0.32;
    let vase = super::taper(p, vec3(0.0, 0.0, bottom), vec3(0.0, 0.0, top), 0.032, 0.039)
        .max(bottom - p.z)
        .max(p.z - top)
        .max(-super::cylinder(p - Vec3::Z * (top - 0.055), 0.031, 0.14));
    let mut grasses = super::FAR;
    let mut blossoms = super::FAR;
    for i in 0..12 {
        let phase = i as f32 * 2.399_963;
        let (sin, cos) = phase.sin_cos();
        let start = vec3(cos * 0.018, sin * 0.018, top - 0.03);
        let middle = vec3(cos * 0.035, sin * 0.035, top + size.z * 0.24);
        let end = vec3(cos * 0.085, sin * 0.055, size.z * (0.37 + 0.07 * phase.sin()));
        grasses =
            grasses.min(super::taper(p, start, middle, 0.0025, 0.002)).min(super::taper(p, middle, end, 0.002, 0.0005));
        if i < 6 {
            let q = super::unrotate_z(p - end, vec2(sin, cos));
            blossoms = blossoms.min(flower(q, 0.016));
            grasses = grasses.min(leaf(q - vec3(0.0, 0.0, -0.05), 0.031, 0.006));
        }
    }
    let distance = vase.min(grasses).min(blossoms);
    let (material, tint) = if blossoms < vase.min(grasses) {
        (Material::Fabric, vec3(0.71, 0.33, 0.52))
    } else if grasses < vase {
        (Material::Foliage, vec3(0.49, 0.54, 0.22))
    } else {
        (Material::Ceramic, vec3(0.66, 0.57, 0.18))
    };
    Hit::new(distance, material, tint)
}
/// Signed triangle profile extruded into a thin wooden silhouette.
fn triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let edge0 = b - a;
    let edge1 = c - b;
    let edge2 = a - c;
    let v0 = p - a;
    let v1 = p - b;
    let v2 = p - c;
    let d0 = v0 - edge0 * (v0.dot(edge0) / edge0.length_squared()).saturate();
    let d1 = v1 - edge1 * (v1.dot(edge1) / edge1.length_squared()).saturate();
    let d2 = v2 - edge2 * (v2.dot(edge2) / edge2.length_squared()).saturate();
    let winding = if edge0.perp_dot(c - a) >= 0.0 { 1.0 } else { -1.0 };
    let inside = (winding * edge0.perp_dot(v0)).min(winding * edge1.perp_dot(v1)).min(winding * edge2.perp_dot(v2));
    d0.length_squared().min(d1.length_squared()).min(d2.length_squared()).sqrt()
        * if inside >= 0.0 { -1.0 } else { 1.0 }
}
/// A pointed, swept feather profile with a curved centerline and tapered width.
fn wing(p: Vec2, root: Vec2, tip: Vec2, width: f32, bend: f32) -> f32 {
    let axis = tip - root;
    let length = axis.length();
    let along = (p - root).dot(axis) / length;
    let fraction = (along / length).saturate();
    let bulge = 4.0 * fraction * (1.0 - fraction);
    let across = axis.perp_dot(p - root) / length - bend * bulge;
    (across.abs() - width * bulge * (1.1 - 0.7 * fraction)).max((-along).max(along - length)) * 0.5
}
fn swallow(p: Vec3) -> f32 {
    let profile = p.xz();
    let wings = wing(profile, vec2(-0.012, 0.015), vec2(-0.21, 0.15), 0.044, -0.032).min(wing(
        profile,
        vec2(0.012, 0.015),
        vec2(0.19, 0.08),
        0.037,
        0.027,
    ));
    let tail = wing(profile, vec2(-0.006, -0.045), vec2(-0.065, -0.19), 0.013, 0.006).min(wing(
        profile,
        vec2(0.006, -0.045),
        vec2(0.07, -0.17),
        0.014,
        -0.006,
    ));
    let head = triangle(profile, vec2(-0.02, 0.06), vec2(0.047, 0.105), vec2(0.025, 0.037));
    let outline = wings.min(tail).min(head);
    let shell = vec2(outline, p.y.abs() - 0.009);
    let carving = shell.max(Vec2::ZERO).length() + shell.max_element().min(0.0);
    let feather = super::capsule(p, vec3(-0.055, -0.009, 0.025), vec3(-0.145, -0.009, 0.105), 0.002)
        .min(super::capsule(p, vec3(0.055, -0.009, 0.025), vec3(0.135, -0.009, 0.09), 0.002));
    let carving = carving.max(-feather);
    let body = super::taper(p, vec3(0.0, -0.004, -0.09), vec3(0.0, -0.004, 0.065), 0.009, 0.014);
    super::blend(carving, body, 0.009)
}
fn weapon(p: Vec3, length: f32) -> Hit {
    // Flattened leaf blade with a curved cutting edge and central ridge.
    let start = -length * 0.23;
    let span = length * 0.73;
    let t = ((p.x - start) / span).clamp(0.0, 1.0);
    let sweep = 0.065 * t * t;
    let width = (0.024 + 0.019 * (t * PI).sin()) * (1.0 - t);
    let cross = ((p.z - sweep).abs() - width).max(start - p.x).max(p.x - length * 0.5);
    let blade = vec2(cross, p.y.abs() - 0.005 * (1.0 - 0.7 * t)).max(Vec2::ZERO).length()
        + cross.max(p.y.abs() - 0.005 * (1.0 - 0.7 * t)).min(0.0);
    let grip = super::taper(p, vec3(-length * 0.49, 0.0, -0.004), vec3(start - 0.035, 0.0, 0.0), 0.014, 0.020);
    let wrap = (p.x * 180.0 + p.y.atan2(p.z) * 2.0).sin();
    let guard =
        wing(p.xz(), vec2(start - 0.03, -0.065), vec2(start + 0.085, 0.060), 0.015, -0.025).max(p.y.abs() - 0.009);
    let pommel = super::taper(p, vec3(-length * 0.50, 0.0, -0.035), vec3(-length * 0.475, 0.0, 0.022), 0.004, 0.012);
    let filigree = ((p.z - sweep - 0.009 * (t * 19.0).sin()).abs() - 0.0025).max(cross);
    let gold = vec3(0.57, 0.45, 0.25);
    let tint = if filigree < 0.0 { gold } else { vec3(0.09, 0.13, 0.24) };
    Hit::new(blade * 0.65, Material::Metal, tint)
        .union(Hit::new(grip, Material::Fabric, vec3(0.12, 0.105, 0.10) * (0.85 + 0.15 * wrap)))
        .union(Hit::new(guard.min(pommel), Material::Metal, gold))
}
pub(super) fn wall_art(p: Vec3, _size: Vec3) -> Hit {
    let upper = p - vec3(0.0, 0.0, -0.10);
    let lower = p - vec3(-0.04, 0.0, -0.26);
    let blade = weapon(upper, 1.55).union(weapon(lower, 2.00));
    let mut birds = super::FAR;
    for i in 0..5 {
        let bird = p - vec3(-0.92 + i as f32 * 0.43, 0.0, 0.72 - i as f32 * 0.16);
        let angle = 0.95 + (i as f32 - 2.0) * 0.24;
        let (sin, cos) = angle.sin_cos();
        let q = vec3(bird.x * cos + bird.z * sin, bird.y, -bird.x * sin + bird.z * cos);
        if super::box_sdf(q, vec3(0.45, 0.05, 0.44)).max(0.0) * 0.3 <= birds.min(blade.distance) {
            birds = birds.min(swallow(q / 0.72) * 0.72);
        }
    }
    Hit::new(birds, Material::Wood, vec3(0.60, 0.37, 0.20)).union(blade)
}
