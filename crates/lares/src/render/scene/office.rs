use super::{Material, Sample, capsule, rounded_box, taper};
use core::f32::consts::TAU;
use isthmus::prelude::*;

const BLACK: Vec3 = vec3(0.075, 0.085, 0.095);
const OAK: Vec3 = vec3(0.78, 0.65, 0.48);

#[inline(never)]
pub(super) fn chair<S: Sample>(p: Vec3, _size: Vec3) -> S {
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
    let column = super::cylinder(p - vec3(0.0, 0.0, -0.39), 0.038, 0.35);
    let a = p.y.atan2(p.x);
    let turn = (a / (TAU / 5.0)).round() * (TAU / 5.0);
    let (sn, cs) = turn.sin_cos();
    let q = super::unrotate_z(p, vec2(sn, cs));
    let foot = taper(q, vec3(0.02, 0.0, -0.49), vec3(0.32, 0.0, -0.57), 0.028, 0.018);
    let wheels = super::cylinder(vec3(q.x - 0.32, q.z + 0.60, q.y), 0.04, 0.055);
    S::new(seat.min_num(back).min_num(arms), || (Material::Fabric, BLACK))
        .union(S::new(supports.min_num(column).min_num(foot).min_num(wheels), || (Material::Metal, BLACK)))
}

#[inline(never)]
pub(super) fn dining_chair<S: Sample>(p: Vec3, size: Vec3) -> S {
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
pub(super) fn table<S: Sample>(p: Vec3, size: Vec3) -> S {
    let top = super::scaled_box(p, size, vec3(0.0, 0.0, 0.46), vec3(1.0, 1.0, 0.08), 0.03);
    let q = vec3(p.x.abs(), p.y.abs(), p.z);
    let corner = size.xy() * 0.5 - 0.065;
    let leg = taper(q, corner.extend(-size.z * 0.5 + 0.025), corner.extend(size.z * 0.42), 0.025, 0.032);
    let apron = rounded_box(p - Vec3::Z * size.z * 0.36, size * vec3(0.90, 0.86, 0.14), 0.005)
        .max_num(-super::box_sdf(p - Vec3::Z * size.z * 0.36, size * vec3(0.84, 0.78, 0.20)));
    S::new(top.min_num(leg).min_num(apron), || (Material::Wood, OAK))
}

fn screen<S: Sample>(p: Vec3, size: Vec2) -> S {
    let casing = rounded_box(p, size.extend(0.03).xzy(), 0.007);
    let face = rounded_box(p - Vec3::Y * 0.017, vec3(size.x - 0.014, 0.003, size.y - 0.014), 0.003);
    S::new(casing, || (Material::Metal, BLACK)).union(S::new(face, || (Material::Emissive, Vec3::ONE)))
}

/// Animated aurora wallpaper: evaluated only after an emissive screen is hit.
#[inline(never)]
pub(super) fn wallpaper(local: Vec3, time: f32) -> Vec3 {
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
pub(super) fn workstation<S: Sample>(p: Vec3, _size: Vec3) -> S {
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
    let cell = key - (key / 0.019).round().clamp(vec2(-9.0, -3.0), vec2(9.0, 3.0)) * 0.019;
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
pub(super) fn pegboard<S: Sample>(p: Vec3, size: Vec3) -> S {
    let board = rounded_box(p - Vec3::Y * -0.07, vec3(size.x, 0.014, size.z), 0.012);
    let grid = p.xz() - (p.xz() / 0.025).round() * 0.025;
    let slot = rounded_box(vec3(grid.x, p.y + 0.07, grid.y), vec3(0.005, 0.04, 0.011), 0.002);
    let board = board.max_num(-slot);
    let tray = rounded_box(p - vec3(0.0, 0.0, -0.17), vec3(size.x * 0.88, 0.14, 0.025), 0.006);
    let bin = rounded_box(p - vec3(0.05, 0.0, 0.11), vec3(0.27, 0.13, 0.14), 0.016).max_num(-rounded_box(
        p - vec3(0.05, 0.0, 0.13),
        vec3(0.25, 0.11, 0.14),
        0.01,
    ));
    S::new(board.min_num(tray).min_num(bin), || (Material::Paint, vec3(0.86, 0.85, 0.80)))
}

#[inline(never)]
pub(super) fn poster<S: Sample>(p: Vec3, size: Vec3, night: bool) -> S {
    let uv = p.xz() / size.xz();
    let mut color = if night { vec3(0.035, 0.08, 0.12) } else { vec3(0.62, 0.62, 0.63) };
    let paper = super::materials::noise(uv * 67.0) - 0.5;
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
