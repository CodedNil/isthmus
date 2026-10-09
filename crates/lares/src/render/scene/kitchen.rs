//! Modular shaker joinery and appliances. Local +Y is the room-facing side.
use super::{Kind, Material, Sample, capsule, cylinder, rounded_box};
use isthmus::prelude::*;
const MAPLE: Vec3 = vec3(0.98, 0.98, 0.96);
const STEEL: Vec3 = vec3(0.64, 0.66, 0.65);
const BLACK: Vec3 = vec3(0.055, 0.065, 0.07);

fn pull(p: Vec3, width: f32) -> f32 {
    capsule(p, vec3(-width * 0.5, 0.025, 0.0), vec3(width * 0.5, 0.025, 0.0), 0.006).min_num(capsule(
        vec3(p.x.abs(), p.y, p.z),
        vec3(width * 0.36, 0.0, 0.0),
        vec3(width * 0.36, 0.025, 0.0),
        0.005,
    ))
}
fn door<S: Sample>(p: Vec3, width: f32, height: f32) -> S {
    let slab = rounded_box(p, vec3(width, 0.022, height), 0.002);
    let recess = rounded_box(p - Vec3::Y * 0.013, vec3(width - 0.095, 0.018, height - 0.095), 0.002);
    let material = if p.z.abs() > height * 0.5 - 0.046 { Material::MapleHorizontal } else { Material::Maple };
    S::new(slab.max_num(-recess), || (material, MAPLE))
}
#[inline(never)]
fn fronts<S: Sample>(p: Vec3, size: Vec3, drawers: bool) -> S {
    let count = (size.x / 0.55).round().max_num(1.0);
    let width = size.x / count;
    let cell = ((p.x + size.x * 0.5) / width).floor().max_num(0.0).min_num(count - 1.0);
    let q = p - vec3(-size.x * 0.5 + (cell + 0.5) * width, size.y * 0.5, 0.0);
    let mut hit = door(q, width - 0.006, size.z - 0.008);
    let handle = if drawers {
        let drawer_z = size.z * 0.5 - 0.08;
        // A separate shallow drawer above each framed cupboard door.
        hit = door::<S>(q + Vec3::Z * 0.083, width - 0.006, size.z - 0.174).union(door(
            q - Vec3::Z * drawer_z,
            width - 0.006,
            0.15,
        ));
        pull(q - vec3(0.0, 0.015, drawer_z), width * 0.62)
            .min_num(pull(vec3(q.z + 0.055, q.y - 0.015, q.x - width * 0.32), 0.23))
    } else {
        let hinge = if count > 1.0 && cell % 2.0 > 0.5 { -1.0 } else { 1.0 };
        let q = q - vec3(hinge * width * 0.30, 0.015, -size.z * 0.12);
        pull(vec3(q.z, q.y, q.x), (size.z * 0.40).min_num(0.30))
    };
    hit.union(S::new(handle, || (Material::Metal, STEEL)))
}
#[inline(never)]
pub(super) fn counter<S: Sample>(p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p - vec3(0.0, -0.018, 0.05), size - vec3(0.014, 0.036, 0.14), 0.002);
    let top = rounded_box(p - Vec3::Z * (size.z * 0.5 - 0.016), vec3(size.x, size.y + 0.025, 0.032), 0.004);
    S::new(body, || (Material::Maple, MAPLE))
        .union(fronts(p - Vec3::Z * 0.048, vec3(size.x - 0.01, size.y - 0.006, size.z - 0.17), true))
        .union(S::new(top, || (Material::Stone, vec3(0.065, 0.072, 0.078))))
}
#[inline(never)]
pub(super) fn cabinet<S: Sample>(kind: Kind, p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p + Vec3::Y * 0.012, size - vec3(0.008, 0.024, 0.012), 0.003);
    let mut hit = S::new(body, || (Material::Maple, MAPLE));
    if kind == Kind::Fridge {
        // Integrated fridge/freezer: two tall wood fronts, not a stainless door.
        let split = -size.z * 0.12;
        let upper = size.z * 0.5 - split;
        let lower = size.z * 0.5 + split;
        let mut i = 0;
        while i < 2 {
            let height = if i == 0 { upper } else { lower };
            let z = if i == 0 { split + upper * 0.5 } else { -size.z * 0.5 + lower * 0.5 };
            hit = hit.union(fronts(p - Vec3::Z * z, vec3(size.x, size.y, height - 0.007), false));
            i += 1;
        }
    } else {
        hit = hit.union(fronts(p, size, false));
    }
    let moulding =
        rounded_box(vec3(p.x, p.y, p.z.abs() - size.z * 0.5), vec3(size.x + 0.045, size.y + 0.045, 0.044), 0.018);
    hit.union(S::new(moulding, || (Material::Maple, MAPLE)))
}
#[inline(never)]
pub(super) fn oven<S: Sample>(p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p - Vec3::Z * 0.05, size - vec3(0.01, 0.018, 0.14), 0.004);
    let front = p - Vec3::Y * (size.y * 0.5);
    let glass = rounded_box(front - vec3(0.0, 0.012, -0.025), vec3(size.x - 0.09, 0.022, size.z * 0.47), 0.007);
    let handle = pull(front - vec3(0.0, 0.024, size.z * 0.24), size.x * 0.80);
    let dial = cylinder(vec3(front.x.abs() - size.x * 0.35, front.z - size.z * 0.36, front.y - 0.015), 0.018, 0.023);
    let display = rounded_box(front - vec3(0.0, 0.012, size.z * 0.36), vec3(0.12, 0.008, 0.030), 0.001);
    let top = rounded_box(p - Vec3::Z * (size.z * 0.5 - 0.016), vec3(size.x, size.y + 0.025, 0.032), 0.004);
    let hob = rounded_box(p - Vec3::Z * (size.z * 0.5 + 0.004), vec3(size.x - 0.018, size.y - 0.018, 0.008), 0.003);
    let ring_p = vec2(p.x.abs() - size.x * 0.24, p.y.abs() - size.y * 0.23);
    let rings = (ring_p.length() - 0.073).abs() < 0.0025 && p.z > size.z * 0.5;
    S::new(body.min_num(handle).min_num(dial), || (Material::Metal, STEEL))
        .union(S::new(top, || (Material::Stone, vec3(0.065, 0.072, 0.078))))
        .union(S::new(glass.min_num(hob), || (Material::Ceramic, if rings { vec3(0.23, 0.24, 0.24) } else { BLACK })))
        .union(S::new(display, || (Material::Ceramic, vec3(0.08, 0.20, 0.06))))
}
#[inline(never)]
pub(super) fn microwave<S: Sample>(p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p, size, 0.012);
    let front = p - Vec3::Y * (size.y * 0.5);
    let window = rounded_box(front - vec3(-0.045, 0.006, 0.0), vec3(size.x - 0.14, 0.010, size.z - 0.065), 0.009);
    let handle = pull(vec3(front.z, front.y - 0.02, front.x - size.x * 0.24), size.z * 0.68);
    let buttons = rounded_box(
        vec3(
            front.x - size.x * 0.39,
            front.y - 0.009,
            front.z - (front.z / 0.023).round().max_num(-3.0).min_num(1.0) * 0.023,
        ),
        vec3(0.035, 0.009, 0.010),
        0.003,
    );
    let display = rounded_box(front - vec3(size.x * 0.38, 0.012, size.z * 0.32), vec3(0.058, 0.007, 0.025), 0.002);
    S::new(body.min_num(handle), || (Material::Metal, vec3(0.30, 0.31, 0.30)))
        .union(S::new(window, || (Material::Ceramic, BLACK)))
        .union(S::new(buttons, || (Material::Paint, vec3(0.20, 0.21, 0.20))))
        .union(S::new(display, || (Material::Ceramic, vec3(0.06, 0.27, 0.08))))
}
#[inline(never)]
pub(super) fn sink<S: Sample>(p: Vec3, size: Vec3) -> S {
    let q = p - vec3(0.0, 0.0, size.z * 0.5);
    // The liner sits inside a larger worktop cut, so wood cannot win the CSG seam.
    let bowl = |point: Vec3, center: Vec3, width: f32, depth: f32| {
        rounded_box(point - center, vec3(width, depth, 0.34), 0.018)
    };
    let large = vec3(0.24, 0.0, 0.025);
    let small = vec3(-0.045, 0.0, 0.045);
    let cavity = bowl(q, large, 0.35, 0.36).min_num(bowl(q, small, 0.17, 0.28));
    let cut = bowl(q + Vec3::Z * 0.012, large, 0.37, 0.38).min_num(bowl(q + Vec3::Z * 0.012, small, 0.19, 0.30));
    let mut hit: S = counter(p, size);
    hit = hit.subtract(cut);
    let liner = cut.max_num(-cavity).max_num(q.z - 0.006);
    let deck = rounded_box(q - Vec3::Z * 0.002, vec3(0.98, 0.47, 0.012), 0.005).max_num(-cavity);
    let drain = vec2(q.x - 0.24, q.y).length();
    let plug = cylinder(q - vec3(0.24, 0.0, -0.140), 0.025, 0.008);
    let drainer = q - vec3(-0.30, 0.0, 0.010);
    let ribs = rounded_box(
        vec3(drainer.x - (drainer.x / 0.026).round().max_num(-5.0).min_num(4.0) * 0.026, drainer.y, drainer.z),
        vec3(0.004, 0.34, 0.004),
        0.001,
    );
    let steel_inside = STEEL * (0.58 + 0.42 * ((q.z + 0.15) / 0.15).max_num(0.0).min_num(1.0));
    hit = hit
        .union(S::new(liner, || (Material::Metal, steel_inside)))
        .union(S::new(deck.min_num(ribs), || (Material::Metal, STEEL * 1.10)))
        .union(S::new(plug, || (Material::Metal, if drain < 0.017 { BLACK } else { STEEL })));
    // Continuous gooseneck in the YZ plane, with a vertical riser and spout.
    let tap = q - vec3(0.05, -0.20, 0.0);
    let arch = vec2(vec2(tap.y - 0.075, tap.z - 0.22).length() - 0.075, tap.x).length() - 0.013;
    let arch = arch.max_num(0.22 - tap.z);
    let riser = capsule(tap, Vec3::ZERO, Vec3::Z * 0.22, 0.013);
    let spout = capsule(tap, vec3(0.0, 0.15, 0.22), vec3(0.0, 0.15, 0.18), 0.014);
    hit.union(S::new(arch.min_num(riser).min_num(spout), || (Material::Metal, BLACK)))
}
#[inline(never)]
pub(super) fn hood<S: Sample>(p: Vec3, size: Vec3) -> S {
    let z = p.z + size.z * 0.5;
    let t = (z / 0.24).max_num(0.0).min_num(1.0);
    let half = size.xy() * 0.5 - vec2(0.18, 0.12) * t;
    let canopy = (p.xy().abs() - half).max_element().max_num(-z).max_num(z - 0.24) * 0.65;
    let chimney = rounded_box(p - vec3(0.0, -0.09, 0.12), vec3(0.20, 0.18, size.z - 0.24), 0.004);
    let lip = rounded_box(p + Vec3::Z * (size.z * 0.5 - 0.010), vec3(size.x, size.y, 0.020), 0.003);
    let rail = rounded_box(p - vec3(0.0, -0.105, size.z * 0.5 - 0.06), vec3(0.615, 0.345, 0.044), 0.010);
    S::new(canopy.min_num(chimney).min_num(lip), || (Material::Metal, STEEL))
        .union(S::new(rail, || (Material::MapleHorizontal, MAPLE)))
}

#[inline(never)]
pub(super) fn splashback<S: Sample>(p: Vec3, size: Vec3) -> S {
    let backing = rounded_box(p, size, 0.001);
    let cell = vec2(0.15, 0.15);
    let from_edge = p.xz() + size.xz() * 0.5;
    let uv = from_edge - ((from_edge / cell).floor() + Vec2::splat(0.5)) * cell;
    let tile = rounded_box(vec3(uv.x, p.y - size.y * 0.5, uv.y), vec3(0.147, 0.010, 0.147), 0.003)
        .max_num(super::box_sdf(p, size + Vec3::Y * 0.02));
    S::new(backing, || (Material::Paint, vec3(0.66, 0.65, 0.61)))
        .union(S::new(tile, || (Material::Ceramic, vec3(0.78, 0.81, 0.80))))
}
#[inline(never)]
pub(super) fn blind<S: Sample>(p: Vec3, size: Vec3) -> S {
    let end = (size.z * 0.5 / 0.023).floor();
    let row = (p.z / 0.023).round().max_num(-end).min_num(end);
    let q = p - Vec3::Z * (row * 0.023);
    let slat =
        rounded_box(vec3(q.x, q.y * 0.60 - q.z * 0.80, q.y * 0.80 + q.z * 0.60), vec3(size.x, 0.026, 0.002), 0.0008);
    let cords = capsule(
        vec3(p.x.abs() - size.x * 0.32, p.y - 0.012, p.z),
        vec3(0.0, 0.0, -size.z * 0.5),
        vec3(0.0, 0.0, size.z * 0.5),
        0.0015,
    );
    S::new(slat.min_num(cords), || (Material::Paint, vec3(0.86, 0.87, 0.85)))
}

#[inline(never)]
pub(super) fn kettle<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = -size.z * 0.5;
    let body = super::taper(p, vec3(0.0, 0.0, base + 0.03), vec3(0.0, 0.0, size.z * 0.28), 0.074, 0.063)
        .max_num(base + 0.015 - p.z)
        .max_num(p.z - size.z * 0.30);
    let lid = cylinder(p - Vec3::Z * (size.z * 0.30), 0.062, 0.012);
    let foot = cylinder(p - Vec3::Z * (base + 0.012), 0.078, 0.024);
    let spout = super::taper(p, vec3(0.0, 0.05, 0.015), vec3(0.0, 0.10, 0.07), 0.032, 0.018);
    let h = p - vec3(0.0, -0.075, 0.0);
    let handle = vec2((h.yz() / vec2(0.045, 0.09)).length() - 1.0, h.x / 0.045).length() * 0.045 - 0.008;
    S::new(body.min_num(spout), || (Material::Ceramic, vec3(0.81, 0.80, 0.73)))
        .union(S::new(lid.min_num(foot).min_num(handle), || (Material::Metal, STEEL)))
}
#[inline(never)]
pub(super) fn entry_door<S: Sample>(p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p, size, 0.002);
    let q = p - Vec3::Y * (size.y * 0.5);
    let letter = rounded_box(q - vec3(0.0, 0.006, -0.22), vec3(0.29, 0.012, 0.065), 0.003);
    let flap = rounded_box(q - vec3(0.0, 0.014, -0.22), vec3(0.267, 0.008, 0.045), 0.002);
    let escutcheon = cylinder(vec3(q.x - size.x * 0.38, q.z - 0.02, q.y - 0.008), 0.025, 0.014);
    let handle = pull(vec3(q.z - 0.07, q.y - 0.018, q.x - size.x * 0.38), 0.16);
    let peephole = cylinder(vec3(q.x, q.z - size.z * 0.27, q.y - 0.004), 0.006, 0.010);
    S::new(body, || (Material::Maple, vec3(0.75, 0.69, 0.60)))
        .union(S::new(letter.min_num(peephole), || (Material::Metal, BLACK)))
        .union(S::new(flap.min_num(escutcheon).min_num(handle), || (Material::Metal, STEEL)))
}

#[inline(never)]
pub(super) fn drawers<S: Sample>(p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p - vec3(0.0, -0.018, 0.05), size - vec3(0.014, 0.036, 0.14), 0.002);
    let top = rounded_box(p - Vec3::Z * (size.z * 0.5 - 0.016), vec3(size.x, size.y + 0.025, 0.032), 0.004);
    let mut hit =
        S::new(body, || (Material::Maple, MAPLE)).union(S::new(top, || (Material::Stone, vec3(0.065, 0.072, 0.078))));
    let mut row = 0;
    while row < 3 {
        let height = if row == 0 { 0.16 } else { 0.27 };
        let z = match row {
            0 => 0.33,
            1 => 0.105,
            _ => -0.17,
        };
        let front = p - vec3(0.0, size.y * 0.5, z);
        hit = hit
            .union(door(front, size.x - 0.006, height))
            .union(S::new(pull(front - vec3(0.0, 0.015, height * 0.24), size.x.min_num(0.58) * 0.65), || {
                (Material::Metal, STEEL)
            }));
        row += 1;
    }
    hit
}

#[inline(never)]
pub(super) fn air_fryer<S: Sample>(p: Vec3, size: Vec3) -> S {
    let body = rounded_box(p, size, 0.045);
    let front = p - Vec3::Y * (size.y * 0.5);
    let panel = rounded_box(front - vec3(0.0, 0.010, 0.10), vec3(size.x * 0.75, 0.014, 0.12), 0.012);
    let seam = rounded_box(front + Vec3::Z * 0.075, vec3(size.x * 0.88, 0.014, 0.006), 0.003);
    let handle = rounded_box(front - vec3(0.0, 0.032, -0.105), vec3(0.025, 0.045, 0.14), 0.011);
    let band = rounded_box(p - Vec3::Z * size.z * 0.44, vec3(size.x + 0.008, size.y + 0.008, 0.012), 0.006);
    let dial = cylinder(vec3(front.x, front.z - 0.10, front.y - 0.022), 0.022, 0.010);
    S::new(body.max_num(-seam), || (Material::Ceramic, BLACK))
        .union(S::new(panel, || (Material::Paint, vec3(0.09, 0.09, 0.10))))
        .union(S::new(handle.min_num(band), || (Material::Metal, vec3(0.65, 0.39, 0.19))))
        .union(S::new(dial, || (Material::Metal, STEEL)))
}

#[inline(never)]
pub(super) fn knife_block<S: Sample>(p: Vec3, size: Vec3) -> S {
    let block = rounded_box(p + Vec3::Z * size.z * 0.23, vec3(size.x, size.y, size.z * 0.54), 0.008);
    let mut handles = super::FAR;
    let mut i = 0;
    while i < 5 {
        let x = (i as f32 - 2.0) * size.x * 0.16;
        handles =
            handles.min_num(capsule(p, vec3(x, 0.0, 0.01), vec3(x, -0.018, size.z * (0.35 + 0.025 * i as f32)), 0.009));
        i += 1;
    }
    S::new(block, || (Material::Wood, vec3(0.62, 0.48, 0.29))).union(S::new(handles, || (Material::Paint, BLACK)))
}
