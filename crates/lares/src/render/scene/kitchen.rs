use super::{Hit, Material};
use isthmus::prelude::*;

pub(super) fn counter(local: Vec3, size: Vec3) -> Hit {
    let body = super::scaled_box(local, size, vec3(0.0, 0.0, -0.04), vec3(0.96, 0.96, 0.92), 0.02);
    let top = super::scaled_box(local, size, vec3(0.0, 0.0, 0.46), vec3(1.0, 1.0, 0.08), 0.03);
    let distance = super::blend(body, top, 0.02);
    Hit { distance, material: Material::Stone, tint: vec3(0.42, 0.42, 0.43) }
}

pub(super) fn cupboard(local: Vec3, size: Vec3) -> Hit {
    let body = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02);
    let seam = super::box_sdf(local - vec3(0.0, size.y * 0.46, 0.0), vec3(size.x * 0.04, size.y * 0.08, size.z * 0.9));
    let distance = body.max(-seam);
    Hit { distance, material: Material::Wood, tint: vec3(0.48, 0.55, 0.62) }
}

pub(super) fn fridge(local: Vec3, size: Vec3) -> Hit {
    let body = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02);
    let seam = super::box_sdf(local - vec3(0.0, size.y * 0.46, 0.0), vec3(size.x * 0.04, size.y * 0.08, size.z * 0.9));
    let distance = body.max(-seam);
    Hit { distance, material: Material::Metal, tint: vec3(0.8, 0.8, 0.78) }
}

pub(super) fn oven(local: Vec3, size: Vec3) -> Hit {
    let body = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02);
    let window = super::scaled_box(local, size, vec3(0.0, 0.46, 0.05), vec3(0.7, 0.08, 0.4), 0.03);
    let distance = body.max(-window);
    Hit { distance, material: Material::Metal, tint: vec3(0.8, 0.8, 0.78) }
}
