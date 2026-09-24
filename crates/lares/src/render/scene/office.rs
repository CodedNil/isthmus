use super::{Hit, Material};
use isthmus::prelude::*;

pub(super) fn chair(local: Vec3, _size: Vec3) -> Hit {
    let seat = super::rounded_box(local - vec3(0.0, 0.03, -0.15), vec3(0.5, 0.48, 0.09), 0.05);
    let back = super::rounded_box(local - vec3(0.0, -0.26, 0.24), vec3(0.44, 0.1, 0.6), 0.09);
    let column = super::taper(local, vec3(0.0, 0.0, -0.62), vec3(0.0, 0.0, -0.2), 0.05, 0.035);
    let foot = super::rounded_box(local - vec3(0.0, 0.0, -0.62), vec3(0.55, 0.08, 0.06), 0.02).min(super::rounded_box(
        local - vec3(0.0, 0.0, -0.62),
        vec3(0.08, 0.55, 0.06),
        0.02,
    ));
    let distance = super::blend(seat, back, 0.08).min(column).min(foot);
    Hit { distance, material: Material::Fabric, tint: vec3(0.16, 0.16, 0.18) }
}

pub(super) fn table(local: Vec3, size: Vec3) -> Hit {
    let top = super::scaled_box(local, size, vec3(0.0, 0.0, 0.44), vec3(1.0, 1.0, 0.1), 0.06);
    let leg = super::capsule(local, vec3(-0.4, -0.4, -0.5) * size, vec3(-0.4, -0.4, 0.42) * size, 0.035);
    let leg_2 = super::capsule(local, vec3(0.4, -0.4, -0.5) * size, vec3(0.4, -0.4, 0.42) * size, 0.035);
    let leg_3 = super::capsule(local, vec3(-0.4, 0.4, -0.5) * size, vec3(-0.4, 0.4, 0.42) * size, 0.035);
    let leg_4 = super::capsule(local, vec3(0.4, 0.4, -0.5) * size, vec3(0.4, 0.4, 0.42) * size, 0.035);
    let distance = super::blend(super::blend(top, leg, 0.03), leg_2, 0.03).min(leg_3).min(leg_4);
    Hit { distance, material: Material::Wood, tint: Vec3::ONE }
}

pub(super) fn workstation(local: Vec3, _size: Vec3) -> Hit {
    let top = super::rounded_box(local - vec3(0.0, 0.0, 0.05), vec3(1.7, 0.9, 0.04), 0.02);
    let cabinet = super::rounded_box(local - vec3(0.5, 0.0, -0.35), vec3(0.6, 0.85, 0.7), 0.03);
    let leg = super::taper(local, vec3(-0.76, -0.38, -0.7), vec3(-0.76, -0.38, 0.0), 0.028, 0.04);
    let panel = super::rounded_box(local - vec3(0.0, -0.16, 0.35), vec3(0.62, 0.035, 0.5), 0.02);
    let keyboard = super::rounded_box(local - vec3(0.0, 0.06, 0.09), vec3(0.44, 0.15, 0.03), 0.012);
    let distance = top.min(cabinet).min(leg).min(panel).min(keyboard);
    Hit { distance, material: Material::Wood, tint: Vec3::ONE }
}
