use super::{Kind, Material, Sample};
use isthmus::prelude::*;

#[inline(never)]
pub(super) fn bed<S: Sample>(local: Vec3, size: Vec3) -> S {
    let p = local + Vec3::Z * size.z * 0.5;
    let base = super::rounded_box(p - vec3(0.0, 0.0, 0.20), vec3(size.x, size.y, 0.36), 0.035);
    let mattress = super::rounded_box(p - vec3(0.0, 0.0, 0.45), vec3(size.x * 0.98, size.y * 0.97, 0.23), 0.075);
    // Centimetre-scale cloth folds, with a conservative distance correction.
    let fold = 0.009 * (p.x * 21.0 + (p.y * 9.0).sin()).sin() + 0.006 * (p.y * 25.0 + p.x * 6.0).sin();
    let duvet =
        super::rounded_box(p - vec3(0.0, 0.10, 0.50 + fold), vec3(size.x + 0.06, size.y - 0.08, 0.25), 0.095) * 0.7;
    let throw =
        super::rounded_box(p - vec3(0.0, 0.02, 0.636 + fold), vec3(size.x + 0.07, size.y * 0.64, 0.032), 0.013) * 0.7;
    let sheet = super::rounded_box(p - vec3(0.0, -size.y * 0.30, 0.627), vec3(size.x, 0.34, 0.043), 0.019);
    let pillow_point = vec3(p.x.abs() - size.x * 0.245, p.y + size.y * 0.34, p.z - 0.704 - 0.055 * p.y);
    let pillows = super::rounded_box(pillow_point, vec3(size.x * 0.47, 0.43, 0.17), 0.070);
    let head_point = p - vec3(0.0, -size.y * 0.48, 0.69);
    let head = super::rounded_box(head_point, vec3(size.x + 0.06, 0.11, 1.18), 0.035);
    let panel_point = vec3(
        head_point.x,
        head_point.y - 0.045,
        head_point.z - (head_point.z / 0.255).round().clamp_num(-1.0, 1.0) * 0.255 - 0.21,
    );
    let panels = super::rounded_box(panel_point, vec3(size.x - 0.07, 0.065, 0.249), 0.023);
    S::new(base, || (Material::Fabric, vec3(0.66, 0.63, 0.58)))
        .union(S::new(mattress.min_num(sheet).min_num(pillows), || (Material::Fabric, vec3(0.96, 0.96, 0.94))))
        .union(S::new(duvet, || (Material::Fabric, vec3(0.20, 0.29, 0.48))))
        .union(S::new(throw, || (Material::Knit, vec3(0.65, 0.65, 0.67))))
        .union(S::new(head.min_num(panels), || {
            let warp = super::materials::noise(vec2(p.x * 9.0, p.z * 2.0));
            let vein = (p.z * 95.0 + warp * 12.0 + super::materials::noise(p.xz() * 27.0) * 2.0).sin();
            (Material::Fabric, Vec3::splat(0.57 + 0.065 * vein))
        }))
}

#[inline(never)]
pub(super) fn cabinet<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let p = local + Vec3::Z * size.z * 0.5;
    let wardrobe = kind == Kind::Wardrobe;
    let columns = if matches!(kind, Kind::Nightstand | Kind::Vanity) { 1.0 } else { 2.0 };
    let rows = if wardrobe {
        1.0
    } else if kind == Kind::Dresser {
        3.0
    } else {
        2.0
    };
    let pitch = vec2((size.x - 0.032) / columns, (size.z - 0.065) / rows);
    let body = super::rounded_box(p - Vec3::Z * (size.z * 0.5 + 0.018), size - vec3(0.0, 0.025, 0.036), 0.003);
    let front_point = p - vec3(0.0, size.y * 0.5 + 0.006, size.z * 0.5 + 0.018);
    let repeated = vec3(
        front_point.x
            - ((front_point.x / pitch.x - 0.5).round() + 0.5).clamp_num(-(columns - 1.0) * 0.5, (columns - 1.0) * 0.5)
                * pitch.x,
        front_point.y,
        front_point.z
            - ((front_point.z / pitch.y - (rows - 1.0) * 0.5).round() + (rows - 1.0) * 0.5)
                .clamp_num(-(rows - 1.0) * 0.5, (rows - 1.0) * 0.5)
                * pitch.y,
    );
    let fronts = super::rounded_box(repeated, vec3(pitch.x - 0.004, 0.018, pitch.y - 0.006), 0.002);
    let color = if kind == Kind::Vanity {
        vec3(0.90, 0.89, 0.86)
    } else if wardrobe {
        Vec3::ONE
    } else {
        vec3(0.105, 0.095, 0.087)
    };
    let material = if kind == Kind::Vanity {
        Material::Paint
    } else if wardrobe {
        Material::Maple
    } else {
        Material::Wood
    };
    let handle_point = vec3(p.x.abs() - 0.035, p.y - size.y * 0.5 - 0.035, p.z - size.z * 0.48);
    let handles = super::capsule(handle_point, vec3(0.0, 0.0, -0.10), vec3(0.0, 0.0, 0.10), 0.007);
    S::new(body, || (material, color * 0.8))
        .union(S::new(fronts, || (material, color)))
        .union(S::new(if wardrobe { handles } else { super::FAR }, || (Material::Metal, Vec3::splat(0.75))))
}

#[inline(never)]
pub(super) fn accessory<S: Sample>(kind: Kind, p: Vec3, size: Vec3) -> S {
    if kind == Kind::TowerFan {
        let base = super::cylinder(p + Vec3::Z * (size.z * 0.5 - 0.018), 0.125, 0.035);
        let body = super::rounded_box(p, vec3(0.13, 0.14, size.z - 0.04), 0.058);
        let vent = super::rounded_box(p - vec3(0.0, 0.067, -0.08), vec3(0.065, 0.012, size.z * 0.68), 0.017);
        let control = super::cylinder(vec3(p.x, p.z - size.z * 0.36, p.y - 0.070), 0.035, 0.009);
        return S::new(body.min_num(base), || (Material::Paint, Vec3::splat(0.07)))
            .union(S::new(vent, || (Material::Metal, Vec3::splat(0.12))))
            .union(S::new(control, || (Material::Metal, Vec3::splat(0.55))));
    }
    let disc = super::cylinder(vec3(p.x, p.z - 0.015, p.y), size.x * 0.48, size.y * 0.70);
    let legs = super::capsule(p, vec3(-0.04, 0.0, -0.08), vec3(-0.075, 0.015, -size.z * 0.5), 0.012)
        .min_num(super::capsule(p, vec3(0.04, 0.0, -0.08), vec3(0.075, 0.015, -size.z * 0.5), 0.012));
    S::new(disc, || (Material::Fabric, Vec3::splat(0.44))).union(S::new(legs, || (Material::Paint, Vec3::splat(0.90))))
}
