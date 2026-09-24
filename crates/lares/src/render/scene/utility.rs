use super::{Hit, Material};
use isthmus::prelude::*;

pub(super) fn appliance(local: Vec3, size: Vec3) -> Hit {
    Hit {
        distance: super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02),
        material: Material::Metal,
        tint: vec3(0.8, 0.8, 0.78),
    }
}

pub(super) fn radiator(local: Vec3, size: Vec3) -> Hit {
    let body = super::scaled_box(local, size, Vec3::ZERO, vec3(1.0, 1.0, 0.9), 0.06);
    let fin = super::box_sdf(local - vec3(0.0, size.y * 0.3, 0.0), vec3(size.x * 0.94, size.y * 0.1, size.z * 0.86));
    let distance = body.max(-fin);
    Hit { distance, material: Material::Metal, tint: vec3(0.8, 0.8, 0.78) }
}
