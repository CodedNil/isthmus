//! Entry furniture and the five-tier lounge display, authored as whole objects.
use crate::scene::{
    FAR, Material, Sample, box_sdf, capsule, cylinder, materials, rounded_box, scaled_box, taper, unrotate_z,
};
use core::f32::consts::TAU;
use isthmus::prelude::*;

const IRON: Vec3 = vec3(0.11, 0.12, 0.13);
const WALNUT: Vec3 = vec3(0.33, 0.25, 0.19);

#[inline(never)]
pub fn shoe_storage<S: Sample>(p: Vec3, size: Vec3) -> S {
    // STÄLL: four flush tilt-out fronts on a shallow carcass with two front feet, back clear of the skirting.
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
pub fn mirror<S: Sample>(p: Vec3, size: Vec3) -> S {
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
    let mut leaves = FAR;
    let mut i = 0;
    while i < 7 {
        let (sn, cs) = (i as f32 * 2.399_963).sin_cos();
        let r = unrotate_z(q - Vec3::Z * 0.08, vec2(sn, cs));
        let blade = vec3((r.x - 0.026) * 0.8 - r.z * 0.6, r.y, (r.x - 0.026) * 0.6 + r.z * 0.8);
        leaves = leaves.min_num(((blade / vec3(0.018, 0.012, 0.070)).length() - 1.0) * 0.012);
        i += 1;
    }
    S::new(pot * scale, || (Material::Ceramic, vec3(0.86, 0.85, 0.79)))
        .union(S::new(leaves * scale, || (Material::Foliage, vec3(0.34, 0.44, 0.20))))
}

/// A few book spines, stacked volumes and planters on the five tiers.
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
pub fn shelf<S: Sample>(p: Vec3, size: Vec3) -> S {
    // The back stays on the wall and each tier is shallower than the one below.
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
        let bounds = box_sdf(q - Vec3::Z * 0.16, vec3(0.74, 0.33, 0.32));
        if bounds <= hit.distance() {
            hit = hit.union(display(q, tier));
        }
        tier += 1;
    }
    hit
}

#[inline(never)]
pub fn swords<S: Sample>(p: Vec3, _size: Vec3) -> S {
    let mut hit = S::new(FAR, || (Material::Metal, vec3(0.65, 0.68, 0.69)));
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

const BLACK: Vec3 = vec3(0.075, 0.085, 0.095);
const OAK: Vec3 = vec3(0.78, 0.65, 0.48);

#[inline(never)]
pub fn chair<S: Sample>(p: Vec3, _size: Vec3) -> S {
    let seat = rounded_box(p - vec3(0.0, 0.025, -0.17), vec3(0.51, 0.48, 0.10), 0.045);
    let q = p - vec3(0.0, -0.25, 0.15);
    let back = rounded_box(vec3(q.x, q.y * 0.98 + q.z * 0.20, q.z * 0.98 - q.y * 0.20), vec3(0.43, 0.10, 0.56), 0.048);
    let side = vec3(p.x.abs(), p.y, p.z);
    let arms = rounded_box(side - vec3(0.30, 0.015, 0.045), vec3(0.075, 0.28, 0.045), 0.02);
    let supports = capsule(side, vec3(0.22, -0.10, -0.22), vec3(0.30, -0.10, 0.025), 0.023).min_num(capsule(
        p,
        vec3(0.0, -0.17, -0.23),
        vec3(0.0, -0.29, 0.05),
        0.027,
    ));
    let column = cylinder(p - vec3(0.0, 0.0, -0.39), 0.038, 0.35);
    let a = p.y.atan2(p.x);
    let turn = (a / (TAU / 5.0)).round() * (TAU / 5.0);
    let (sn, cs) = turn.sin_cos();
    let q = unrotate_z(p, vec2(sn, cs));
    let foot = taper(q, vec3(0.02, 0.0, -0.49), vec3(0.32, 0.0, -0.57), 0.028, 0.018);
    let wheels = cylinder(vec3(q.x - 0.32, q.z + 0.60, q.y), 0.04, 0.055);
    S::new(seat.min_num(back).min_num(arms), || (Material::Fabric, BLACK))
        .union(S::new(supports.min_num(column).min_num(foot).min_num(wheels), || (Material::Metal, BLACK)))
}

#[inline(never)]
pub fn dining_chair<S: Sample>(p: Vec3, size: Vec3) -> S {
    let seat = rounded_box(p - vec3(0.0, 0.0, -0.02), vec3(0.44, 0.43, 0.035), 0.012);
    let cushion = rounded_box(p - vec3(0.0, 0.01, 0.012), vec3(0.405, 0.395, 0.04), 0.018);
    let q = vec3(p.x.abs(), p.y, p.z);
    let rear = taper(q, vec3(0.19, -0.19, -size.z * 0.5), vec3(0.19, -0.24, size.z * 0.47), 0.016, 0.019);
    let front = taper(q, vec3(0.20, 0.19, -size.z * 0.5), vec3(0.18, 0.17, -0.02), 0.014, 0.022);
    let rail = rounded_box(p - vec3(0.0, -0.235, size.z * 0.45), vec3(0.42, 0.035, 0.055), 0.008);
    let x = p.x - (p.x / 0.068).round().max_num(-2.0).min_num(2.0) * 0.068;
    let spindle = rounded_box(vec3(x, p.y + 0.225 + p.z * 0.025, p.z - 0.22), vec3(0.022, 0.021, 0.35), 0.005);
    S::new(seat.min_num(rear).min_num(front).min_num(rail).min_num(spindle), || (Material::Wood, OAK))
        .union(S::new(cushion, || (Material::Fabric, vec3(0.72, 0.73, 0.72))))
}

#[inline(never)]
pub fn table<S: Sample>(p: Vec3, size: Vec3) -> S {
    let top = scaled_box(p, size, vec3(0.0, 0.0, 0.46), vec3(1.0, 1.0, 0.08), 0.03);
    let q = vec3(p.x.abs(), p.y.abs(), p.z);
    let corner = size.xy() * 0.5 - 0.065;
    let leg = taper(q, corner.extend(-size.z * 0.5 + 0.025), corner.extend(size.z * 0.42), 0.025, 0.032);
    let apron = rounded_box(p - Vec3::Z * size.z * 0.36, size * vec3(0.90, 0.86, 0.14), 0.005)
        .max_num(-box_sdf(p - Vec3::Z * size.z * 0.36, size * vec3(0.84, 0.78, 0.20)));
    S::new(top.min_num(leg).min_num(apron), || (Material::Wood, OAK))
}

fn screen<S: Sample>(p: Vec3, size: Vec2) -> S {
    let casing = rounded_box(p, size.extend(0.03).xzy(), 0.007);
    let face = rounded_box(p - Vec3::Y * 0.017, vec3(size.x - 0.014, 0.003, size.y - 0.014), 0.003);
    S::new(casing, || (Material::Metal, BLACK)).union(S::new(face, || (Material::Emissive, Vec3::ONE)))
}

/// Animated aurora wallpaper, evaluated only after a screen hit.
#[inline(never)]
pub fn wallpaper(local: Vec3, time: f32) -> Vec3 {
    let p = vec3(-local.x, local.y, local.z);
    let small = p.x < -0.33;
    let center = if small { vec2(-0.49, 0.16) } else { vec2(0.10, 0.38) };
    let size = if small { vec2(0.27, 0.18) } else { vec2(0.84, 0.49) };
    let uv = (p.xz() - center) / size;
    let ridge = -0.25 + 0.045 * (uv.x * 19.0).sin() + 0.025 * (uv.x * 37.0 + 2.0).sin();
    let aurora = uv.y - 0.12 * (uv.x * 8.0 + time * 0.22).sin() - 0.06 * (uv.x * 19.0 - time * 0.14).sin();
    let glow = (-aurora.abs() * 10.0).exp();
    let ribbon = vec3(0.045, 0.50, 0.34).lerp(vec3(0.22, 0.12, 0.60), (uv.x * 3.0 + time * 0.09).sin() * 0.5 + 0.5);
    let sky = vec3(0.006, 0.014, 0.045) + ribbon * glow;
    if uv.y < ridge { vec3(0.008, 0.025, 0.041) } else { sky }
}

#[inline(never)]
pub fn workstation<S: Sample>(p: Vec3, _size: Vec3) -> S {
    // Mirror the modular return toward the window in this room.
    let p = vec3(-p.x, p.y, p.z);
    // 141.7 x 120 cm Yaelle, local floor at -0.75 and desktop at 0.032.
    let top = rounded_box(p - vec3(-0.045, -0.29, 0.014), vec3(1.327, 0.62, 0.036), 0.017);
    let drawers = rounded_box(p - vec3(-0.045, -0.29, -0.07), vec3(1.30, 0.59, 0.14), 0.025);
    let return_top = rounded_box(p - vec3(-0.50, 0.21, -0.18), vec3(0.417, 0.78, 0.035), 0.014);
    let cabinet = rounded_box(p - vec3(-0.50, 0.21, -0.46), vec3(0.39, 0.75, 0.53), 0.01);
    let flute_x = p.x - (p.x / 0.018).round() * 0.018;
    let groove = rounded_box(vec3(flute_x, p.y - 0.012, p.z + 0.07), vec3(0.006, 0.012, 0.115), 0.003);
    let flute_y = p.y - (p.y / 0.018).round() * 0.018;
    let door_groove = rounded_box(vec3(p.x + 0.299, flute_y, p.z + 0.46), vec3(0.012, 0.006, 0.49), 0.003);
    let q = vec3(p.x, (p.y + 0.29).abs(), p.z);
    let legs = taper(q, vec3(0.59, 0.24, -0.72), vec3(0.53, 0.22, -0.12), 0.025, 0.046);
    let wood =
        top.min_num(drawers.max_num(-groove)).min_num(return_top).min_num(cabinet.max_num(-door_groove)).min_num(legs);
    let stand = rounded_box(p - vec3(0.10, -0.40, 0.043), vec3(0.29, 0.19, 0.02), 0.009).min_num(capsule(
        p,
        vec3(0.10, -0.43, 0.05),
        vec3(0.10, -0.43, 0.29),
        0.023,
    ));
    let mat = rounded_box(p - vec3(0.10, -0.14, 0.035), vec3(0.83, 0.32, 0.005), 0.012);
    let keyboard = rounded_box(p - vec3(0.08, -0.10, 0.054), vec3(0.37, 0.13, 0.029), 0.006);
    let key = vec2(p.x - 0.08, p.y + 0.10);
    let cell = key - (key / 0.019).round().clamp_num(vec2(-9.0, -3.0), vec2(9.0, 3.0)) * 0.019;
    let keys = rounded_box(vec3(cell.x, cell.y, p.z - 0.073), vec3(0.016, 0.016, 0.012), 0.002);
    let mouse = rounded_box(p - vec3(-0.19, -0.10, 0.059), vec3(0.06, 0.10, 0.04), 0.018);
    S::new(wood, || (Material::Wood, OAK))
        .union(S::new(stand.min_num(keyboard).min_num(mouse), || (Material::Metal, BLACK)))
        .union(S::new(keys, || (Material::Paint, vec3(0.76, 0.77, 0.72))))
        .union(S::new(mat, || (Material::Fabric, vec3(0.52, 0.29, 0.22))))
        .union(screen(p - vec3(0.10, -0.46, 0.38), vec2(0.84, 0.49)))
        .union(screen(p - vec3(-0.49, -0.33, 0.16), vec2(0.27, 0.18)))
}

#[inline(never)]
pub fn pegboard<S: Sample>(p: Vec3, size: Vec3) -> S {
    let board = rounded_box(p - Vec3::Y * -0.07, vec3(size.x, 0.014, size.z), 0.012);
    // SKÅDIS has staggered vertical slots, not a square perforation grid.
    let row = (p.z / 0.04).round();
    let x = p.x - (row % 2.0) * 0.02;
    let grid = vec2(x - (x / 0.04).round() * 0.04, p.z - row * 0.04);
    let slot = rounded_box(vec3(grid.x, p.y + 0.07, grid.y), vec3(0.005, 0.04, 0.014), 0.002);
    let board = board.max_num(-slot);
    // Two separate lower trays, a small upper cup and the wide upper bin.
    let q = p - vec3(if p.x < 0.0 { -0.137 } else { 0.137 }, 0.0, -0.19);
    let tray = rounded_box(q, vec3(0.25, 0.14, 0.035), 0.007).max_num(-rounded_box(
        q - Vec3::Z * 0.015,
        vec3(0.23, 0.12, 0.035),
        0.005,
    ));
    let q = p - vec3(0.075, 0.0, 0.115);
    let bin = rounded_box(q, vec3(0.27, 0.13, 0.14), 0.016).max_num(-rounded_box(
        q - Vec3::Z * 0.015,
        vec3(0.25, 0.11, 0.14),
        0.01,
    ));
    let q = p - vec3(-0.18, -0.005, 0.145);
    let cup = rounded_box(q, vec3(0.075, 0.105, 0.085), 0.01).max_num(-rounded_box(
        q - Vec3::Z * 0.01,
        vec3(0.061, 0.091, 0.085),
        0.007,
    ));
    S::new(board.min_num(tray).min_num(bin).min_num(cup), || (Material::Paint, vec3(0.92, 0.92, 0.90)))
}

#[inline(never)]
pub fn poster<S: Sample>(p: Vec3, size: Vec3, night: bool) -> S {
    let uv = p.xz() / size.xz();
    let mut color = if night { vec3(0.035, 0.08, 0.12) } else { vec3(0.62, 0.62, 0.63) };
    let paper = materials::noise(uv * 67.0) - 0.5;
    color += Vec3::splat(paper * 0.035);
    let mut i = 0;
    while i < 9 {
        let f = i as f32;
        let phase = if night { 2.3 } else { 0.0 };
        let ridge = 0.14 - f * 0.075
            + 0.065 * (uv.x * 14.0 + f * 2.0 + phase).sin()
            + 0.028 * (uv.x * 33.0 + f).sin()
            + 0.011 * (uv.x * 83.0 + f * 3.0).sin();
        if uv.y < ridge {
            color = if night {
                vec3(0.16, 0.28, 0.32).lerp(vec3(0.65, 0.62, 0.48), (f * 1.9).sin() * 0.5 + 0.5)
            } else if i & 1 == 0 {
                vec3(0.09, 0.18, 0.22)
            } else {
                vec3(0.63, 0.35, 0.22)
            };
            color *= 0.65 + f * 0.035 + paper * 0.10;
            if ridge - uv.y < 0.008 {
                color = color.lerp(vec3(0.82, 0.69, 0.43), 0.65);
            }
        }
        i += 1;
    }
    let sun = (uv - if night { vec2(-0.21, 0.34) } else { vec2(0.20, 0.31) }).length();
    if sun < 0.085 {
        color = if night { vec3(0.84, 0.83, 0.70) } else { vec3(0.75, 0.31, 0.18) } + Vec3::splat(paper * 0.1);
    }
    S::new(rounded_box(p, size, 0.002), || (Material::Metal, color))
}
