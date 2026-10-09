use super::{Kind, Material, Sample};
use core::f32::consts::TAU;
use isthmus::prelude::*;

fn radiator(p: Vec3, size: Vec3) -> f32 {
    let panel = super::rounded_box(p, size * vec3(0.96, 0.64, 0.88), 0.012);
    // Bounded domain repetition: vertical pressed channels and real top vents.
    let pitch = 0.027;
    let count = (size.x * 0.45 / pitch).floor();
    let x = p.x - (p.x / pitch).round().max_num(-count).min_num(count) * pitch;
    let groove = super::rounded_box(vec3(x, p.y.abs() - size.y * 0.32, p.z), vec3(0.007, 0.008, size.z * 0.78), 0.003);
    let panel = panel.max_num(-groove);
    let cap = super::rounded_box(p - Vec3::Z * size.z * 0.45, vec3(size.x * 0.98, size.y * 0.92, 0.025), 0.008);
    let vent = super::rounded_box(vec3(x, p.y, p.z - size.z * 0.45), vec3(0.015, size.y * 0.57, 0.04), 0.004);
    let cap = cap.max_num(-vent);
    let endcap =
        super::rounded_box(vec3(p.x.abs() - size.x * 0.48, p.y, p.z), vec3(0.025, size.y * 0.9, size.z * 0.90), 0.008);
    let valve =
        super::capsule(p, vec3(size.x * 0.49, 0.0, -size.z * 0.35), vec3(size.x * 0.54, 0.0, -size.z * 0.35), 0.025);
    panel.min_num(cap).min_num(endcap).min_num(valve)
}
fn window(p: Vec3, size: Vec3) -> f32 {
    let sill = super::rounded_box(p - vec3(0.0, 0.045, -size.z * 0.5 + 0.012), vec3(size.x + 0.06, 0.23, 0.035), 0.008);
    let frame = super::rounded_box(p - Vec3::Y * 0.13, vec3(size.x, 0.035, size.z), 0.008);
    let opening = super::box_sdf(p - Vec3::Y * 0.13, vec3(size.x - 0.08, 0.10, size.z - 0.08));
    let frame = frame.max_num(-opening);
    let mullion = super::rounded_box(p - Vec3::Y * 0.13, vec3(0.035, 0.035, size.z - 0.05), 0.004);
    sill.min_num(frame).min_num(mullion)
}
fn curtain<S: Sample>(p: Vec3, size: Vec3) -> S {
    // Broad vertical folds draw outward into a narrow gathered waist. Keep the
    // cloth continuous and give it a hem, rather than twisting an open sheet.
    let height = (p.z / size.z + 0.5).max_num(0.0).min_num(1.0);
    let t = ((height - 0.18) / 0.82).max_num(0.0).min_num(1.0);
    let spread = t * t * (3.0 - 2.0 * t);
    let width = 0.048 + 0.087 * spread + 0.020 * (1.0 - height / 0.18).max_num(0.0);
    let center = size.x * 0.5 - 0.095 - 0.05 * spread;
    let across = p.x.abs() - center;
    let depth = p.y + 0.025 - 0.035 * (1.0 - spread);
    let pleat = (across * TAU / 0.037).cos() * (0.010 + 0.007 * spread);
    let cloth = super::rounded_box(
        vec3(across, depth - pleat, p.z),
        vec3(width * 2.0, 0.024 + 0.028 * (1.0 - spread), size.z * 0.96),
        0.010,
    ) * 0.5;
    let rod =
        super::capsule(p, vec3(-size.x * 0.5, -0.025, size.z * 0.49), vec3(size.x * 0.5, -0.025, size.z * 0.49), 0.010);
    let tie = vec3(p.x.abs() - (size.x * 0.5 - 0.095), p.y - 0.01, p.z + size.z * 0.32);
    let hoop = vec2(tie.xy().length() - 0.067, tie.z).length() - 0.005;
    let mount = super::capsule(tie, vec3(0.0, 0.065, 0.0), vec3(0.0, 0.085, 0.0), 0.012);
    let finial = vec3(p.x.abs() - size.x * 0.49, p.y + 0.025, p.z - size.z * 0.49).length() - 0.020;
    S::new(cloth, || (Material::Fabric, vec3(0.68, 0.64, 0.57)))
        .union(S::new(rod.min_num(hoop).min_num(mount).min_num(finial), || (Material::Metal, vec3(0.65, 0.63, 0.58))))
}

#[inline(never)]
pub(super) fn model<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let distance = match kind {
        Kind::Appliance => super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02),
        Kind::Radiator => radiator(local, size),
        Kind::Window => window(local, size),
        Kind::Curtain => return curtain(local, size),
        _ => super::FAR,
    };
    let painted = kind != Kind::Appliance;
    S::new(distance, || {
        (
            if painted { Material::Paint } else { Material::Metal },
            if painted { vec3(0.93, 0.92, 0.89) } else { vec3(0.45, 0.45, 0.43) },
        )
    })
}
