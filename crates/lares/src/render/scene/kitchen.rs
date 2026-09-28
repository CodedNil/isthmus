//! Modular shaker joinery and appliances. Local +Y is the room-facing side.
use super::{Hit, Kind, Material, capsule, cylinder, rounded_box};
use isthmus::prelude::*;
const MAPLE: Vec3 = vec3(0.98, 0.98, 0.96);
const STEEL: Vec3 = vec3(0.64, 0.66, 0.65);
const BLACK: Vec3 = vec3(0.055, 0.065, 0.07);

fn pull(p: Vec3, width: f32) -> f32 {
    capsule(p, vec3(-width * 0.5, 0.025, 0.0), vec3(width * 0.5, 0.025, 0.0), 0.006).min(capsule(
        vec3(p.x.abs(), p.y, p.z),
        vec3(width * 0.36, 0.0, 0.0),
        vec3(width * 0.36, 0.025, 0.0),
        0.005,
    ))
}
fn door(p: Vec3, width: f32, height: f32) -> Hit {
    let slab = rounded_box(p, vec3(width, 0.022, height), 0.002);
    let recess = rounded_box(p - Vec3::Y * 0.013, vec3(width - 0.095, 0.018, height - 0.095), 0.002);
    Hit::new(slab.max(-recess), Material::Maple, MAPLE)
}
fn fronts(p: Vec3, size: Vec3, drawers: bool) -> Hit {
    let count = (size.x / 0.55).round().max(1.0);
    let width = size.x / count;
    let cell = ((p.x + size.x * 0.5) / width).floor().clamp(0.0, count - 1.0);
    let q = p - vec3(-size.x * 0.5 + (cell + 0.5) * width, size.y * 0.5, 0.0);
    let mut hit = door(q, width - 0.006, size.z - 0.008);
    let handle = if drawers {
        let drawer_z = size.z * 0.5 - 0.08;
        // A separate shallow drawer above each framed cupboard door.
        hit = door(q + Vec3::Z * 0.083, width - 0.006, size.z - 0.174).union(door(
            q - Vec3::Z * drawer_z,
            width - 0.006,
            0.15,
        ));
        pull(q - vec3(0.0, 0.015, drawer_z), width * 0.62)
            .min(pull(vec3(q.x, q.y, q.z - size.z * 0.5 + 0.225), width * 0.62))
    } else {
        let q = q - vec3(width * 0.30, 0.015, -size.z * 0.12);
        pull(vec3(q.z, q.y, q.x), (size.z * 0.40).min(0.30))
    };
    hit.union(Hit::new(handle, Material::Metal, STEEL))
}
pub(super) fn counter(p: Vec3, size: Vec3) -> Hit {
    let bottom = -size.z * 0.5;
    let plinth = rounded_box(p - vec3(0.0, -0.035, bottom + 0.07), vec3(size.x - 0.035, size.y - 0.09, 0.14), 0.002);
    let body = rounded_box(p - vec3(0.0, -0.018, 0.05), size - vec3(0.014, 0.036, 0.14), 0.002);
    let top = rounded_box(p - Vec3::Z * (size.z * 0.5 - 0.016), vec3(size.x, size.y + 0.025, 0.032), 0.004);
    Hit::new(body.min(plinth), Material::Maple, MAPLE)
        .union(fronts(p - Vec3::Z * 0.048, vec3(size.x - 0.01, size.y - 0.006, size.z - 0.17), true))
        .union(Hit::new(top, Material::Stone, vec3(0.105, 0.115, 0.12)))
}
pub(super) fn cabinet(kind: Kind, p: Vec3, size: Vec3) -> Hit {
    let body = rounded_box(p + Vec3::Y * 0.012, size - vec3(0.008, 0.024, 0.012), 0.003);
    let mut hit = Hit::new(body, Material::Maple, MAPLE);
    if kind == Kind::Fridge {
        // Integrated fridge/freezer: two tall wood fronts, not a stainless door.
        let split = -size.z * 0.12;
        let upper = size.z * 0.5 - split;
        let lower = size.z * 0.5 + split;
        for i in 0..2 {
            let height = if i == 0 { upper } else { lower };
            let z = if i == 0 { split + upper * 0.5 } else { -size.z * 0.5 + lower * 0.5 };
            hit = hit.union(fronts(p - Vec3::Z * z, vec3(size.x, size.y, height - 0.007), false));
        }
    } else {
        hit = hit.union(fronts(p, size, false));
    }
    let moulding =
        rounded_box(vec3(p.x, p.y, p.z.abs() - size.z * 0.5), vec3(size.x + 0.025, size.y + 0.025, 0.024), 0.010);
    hit.union(Hit::new(moulding, Material::Maple, MAPLE))
}
pub(super) fn oven(p: Vec3, size: Vec3) -> Hit {
    let body = rounded_box(p, size - vec3(0.01, 0.018, 0.03), 0.004);
    let front = p - Vec3::Y * (size.y * 0.5);
    let glass = rounded_box(front - vec3(0.0, 0.006, -0.08), vec3(size.x - 0.09, 0.017, size.z * 0.54), 0.007);
    let handle = pull(front - vec3(0.0, 0.018, size.z * 0.23), size.x * 0.80);
    let dial = cylinder(vec3(front.x.abs() - size.x * 0.35, front.z - size.z * 0.36, front.y - 0.015), 0.018, 0.023);
    let display = rounded_box(front - vec3(0.0, 0.012, size.z * 0.36), vec3(0.12, 0.008, 0.030), 0.001);
    let hob = rounded_box(p - Vec3::Z * (size.z * 0.5 + 0.006), vec3(size.x, size.y, 0.012), 0.003);
    let ring_p = vec2(p.x.abs() - size.x * 0.24, p.y.abs() - size.y * 0.23);
    let rings = (ring_p.length() - 0.073).abs() < 0.0018 && p.z > size.z * 0.5;
    Hit::new(body.min(handle).min(dial), Material::Metal, STEEL)
        .union(Hit::new(glass.min(hob), Material::Ceramic, if rings { vec3(0.23, 0.24, 0.24) } else { BLACK }))
        .union(Hit::new(display, Material::Ceramic, vec3(0.08, 0.20, 0.06)))
}
pub(super) fn microwave(p: Vec3, size: Vec3) -> Hit {
    let body = rounded_box(p, size, 0.012);
    let front = p - Vec3::Y * (size.y * 0.5);
    let window = rounded_box(front - vec3(-0.045, 0.006, 0.0), vec3(size.x - 0.14, 0.010, size.z - 0.065), 0.009);
    let handle = pull(vec3(front.z, front.y - 0.02, front.x - size.x * 0.24), size.z * 0.68);
    let buttons = rounded_box(
        vec3(front.x - size.x * 0.39, front.y - 0.009, front.z - (front.z / 0.023).round().clamp(-3.0, 1.0) * 0.023),
        vec3(0.035, 0.009, 0.010),
        0.003,
    );
    let display = rounded_box(front - vec3(size.x * 0.38, 0.012, size.z * 0.32), vec3(0.058, 0.007, 0.025), 0.002);
    Hit::new(body.min(handle), Material::Metal, vec3(0.30, 0.31, 0.30))
        .union(Hit::new(window, Material::Ceramic, BLACK))
        .union(Hit::new(buttons, Material::Paint, vec3(0.20, 0.21, 0.20)))
        .union(Hit::new(display, Material::Ceramic, vec3(0.06, 0.27, 0.08)))
}
pub(super) fn sink(p: Vec3, size: Vec3) -> Hit {
    let q = p - vec3(0.0, 0.0, size.z * 0.5);
    let cavity = rounded_box(q - vec3(0.18, 0.0, -0.045), vec3(0.37, 0.39, 0.19), 0.045);
    let mut hit = counter(p, size);
    hit.distance = hit.distance.max(-cavity);
    let bowl = rounded_box(q - vec3(0.18, 0.0, -0.05), vec3(0.39, 0.41, 0.19), 0.045).max(-cavity);
    let rim = rounded_box(q + Vec3::Z * 0.002, vec3(0.88, 0.46, 0.012), 0.015).max(-cavity);
    let drainer = q - vec3(-0.25, 0.0, 0.004);
    let ribs = rounded_box(
        vec3(drainer.x - (drainer.x / 0.034).round().clamp(-4.0, 4.0) * 0.034, drainer.y, drainer.z),
        vec3(0.007, 0.35, 0.005),
        0.002,
    );
    hit = hit.union(Hit::new(bowl.min(rim).min(ribs), Material::Metal, STEEL));
    // Continuous gooseneck in the YZ plane, with a vertical riser and spout.
    let tap = q - vec3(0.05, -0.20, 0.0);
    let arch = vec2(vec2(tap.y - 0.075, tap.z - 0.22).length() - 0.075, tap.x).length() - 0.013;
    let arch = arch.max(0.22 - tap.z);
    let riser = capsule(tap, Vec3::ZERO, Vec3::Z * 0.22, 0.013);
    let spout = capsule(tap, vec3(0.0, 0.15, 0.22), vec3(0.0, 0.15, 0.18), 0.014);
    hit.union(Hit::new(arch.min(riser).min(spout), Material::Metal, BLACK))
}
pub(super) fn hood(p: Vec3, size: Vec3) -> Hit {
    let z = p.z + size.z * 0.5;
    let t = (z / 0.23).clamp(0.0, 1.0);
    let half = size.xy() * 0.5 - vec2(0.22, 0.13) * t;
    let canopy = (p.xy().abs() - half).max_element().max(-z).max(z - 0.23) * 0.65;
    let chimney = rounded_box(p - vec3(0.0, -0.07, 0.13), vec3(0.20, 0.20, size.z - 0.23), 0.004);
    let lip = rounded_box(p + Vec3::Z * (size.z * 0.5 - 0.013), vec3(size.x, size.y, 0.026), 0.003);
    let grille = p.x.abs() < 0.24 && p.y.abs() < 0.13 && z < 0.008 && (p.x * 280.0).sin() > 0.2;
    Hit::new(canopy.min(chimney).min(lip), Material::Metal, if grille { BLACK } else { STEEL })
}
pub(super) fn splashback(p: Vec3, size: Vec3) -> Hit {
    let backing = rounded_box(p, size, 0.001);
    let cell = vec2(0.15, 0.15);
    let uv = p.xz() - (p.xz() / cell).round() * cell;
    let tile = rounded_box(vec3(uv.x, p.y - size.y * 0.5, uv.y), vec3(0.147, 0.010, 0.147), 0.003)
        .max(super::box_sdf(p, size + Vec3::Y * 0.02));
    Hit::new(backing, Material::Paint, vec3(0.66, 0.65, 0.61)).union(Hit::new(
        tile,
        Material::Ceramic,
        vec3(0.78, 0.81, 0.80),
    ))
}
pub(super) fn blind(p: Vec3, size: Vec3) -> Hit {
    let row = (p.z / 0.025).round().clamp(-(size.z * 0.5 / 0.025).floor(), (size.z * 0.5 / 0.025).floor());
    let q = p - Vec3::Z * (row * 0.025);
    let slat =
        rounded_box(vec3(q.x, q.y * 0.94 - q.z * 0.34, q.y * 0.34 + q.z * 0.94), vec3(size.x, 0.023, 0.002), 0.0008);
    let cords = capsule(
        vec3(p.x.abs() - size.x * 0.32, p.y - 0.012, p.z),
        vec3(0.0, 0.0, -size.z * 0.5),
        vec3(0.0, 0.0, size.z * 0.5),
        0.0015,
    );
    Hit::new(slat.min(cords), Material::Paint, vec3(0.86, 0.87, 0.85))
}

pub(super) fn kettle(p: Vec3, size: Vec3) -> Hit {
    let base = -size.z * 0.5;
    let body = super::taper(p, vec3(0.0, 0.0, base + 0.03), vec3(0.0, 0.0, size.z * 0.28), 0.074, 0.063)
        .max(base + 0.015 - p.z)
        .max(p.z - size.z * 0.30);
    let lid = cylinder(p - Vec3::Z * (size.z * 0.30), 0.062, 0.012);
    let foot = cylinder(p - Vec3::Z * (base + 0.012), 0.078, 0.024);
    let spout = super::taper(p, vec3(0.0, 0.05, 0.015), vec3(0.0, 0.10, 0.07), 0.032, 0.018);
    let h = p - vec3(0.0, -0.075, 0.0);
    let handle = vec2((h.yz() / vec2(0.045, 0.09)).length() - 1.0, h.x / 0.045).length() * 0.045 - 0.008;
    Hit::new(body.min(spout), Material::Ceramic, vec3(0.81, 0.80, 0.73)).union(Hit::new(
        lid.min(foot).min(handle),
        Material::Metal,
        STEEL,
    ))
}
pub(super) fn entry_door(p: Vec3, size: Vec3) -> Hit {
    let body = rounded_box(p, size, 0.002);
    let q = p - Vec3::Y * (size.y * 0.5);
    let letter = rounded_box(q - vec3(0.0, 0.006, -0.22), vec3(0.29, 0.012, 0.065), 0.003);
    let flap = rounded_box(q - vec3(0.0, 0.014, -0.22), vec3(0.267, 0.008, 0.045), 0.002);
    let escutcheon = cylinder(vec3(q.x - size.x * 0.38, q.z - 0.02, q.y - 0.008), 0.025, 0.014);
    let handle = pull(vec3(q.z - 0.07, q.y - 0.018, q.x - size.x * 0.38), 0.16);
    let peephole = cylinder(vec3(q.x, q.z - size.z * 0.27, q.y - 0.004), 0.006, 0.010);
    Hit::new(body, Material::Maple, vec3(0.75, 0.69, 0.60))
        .union(Hit::new(letter.min(peephole), Material::Metal, BLACK))
        .union(Hit::new(flap.min(escutcheon).min(handle), Material::Metal, STEEL))
}
