use super::{Hit, Material};
use isthmus::prelude::*;

pub(super) fn ottoman(local: Vec3, _size: Vec3) -> Hit {
    let base = super::rounded_box(local - vec3(0.0, 0.0, -0.15), vec3(0.78, 0.58, 0.16), 0.03);
    let cushion = super::rounded_box(local - vec3(0.0, 0.0, 0.02), vec3(0.8, 0.6, 0.36), 0.1);
    let throw = super::rounded_box(local - vec3(0.08, 0.04, 0.19), vec3(0.44, 0.3, 0.05), 0.02);
    let body = super::blend(base, cushion, 0.08);
    let distance = body.min(throw);
    let tint = if throw < body { vec3(0.12, 0.3, 0.72) } else { vec3(0.7, 0.6, 0.47) };
    Hit { distance, material: Material::Fabric, tint }
}

pub(super) fn rug(local: Vec3, _size: Vec3) -> Hit {
    Hit {
        distance: super::rounded_box(local - Vec3::ZERO, vec3(2.6, 2.0, 0.05), 0.02),
        material: Material::Carpet,
        tint: Vec3::ONE,
    }
}

pub(super) fn sofa(local: Vec3, size: Vec3) -> Hit {
    let fabric = vec3(0.7, 0.6, 0.47);
    let blue = vec3(0.12, 0.3, 0.72);
    let seat = super::scaled_box(local, size, vec3(0.0, 0.05, -0.2), vec3(1.0, 0.85, 0.55), 0.16);
    let back = super::scaled_box(local, size, vec3(0.0, -0.42, 0.22), vec3(1.0, 0.16, 0.55), 0.3);
    let arm = super::scaled_box(local, size, vec3(-0.44, 0.05, 0.0), vec3(0.12, 0.9, 0.6), 0.4);
    let arm_2 = super::scaled_box(local, size, vec3(0.44, 0.05, 0.0), vec3(0.12, 0.9, 0.6), 0.4);
    let cushion = super::scaled_box(local, size, vec3(-0.22, -0.34, 0.2), vec3(0.4, 0.14, 0.5), 0.3);
    let cushion_2 = super::scaled_box(local, size, vec3(0.22, -0.34, 0.2), vec3(0.4, 0.14, 0.5), 0.3);
    let pillow = super::scaled_box(local, size, vec3(-0.3, -0.24, 0.02), vec3(0.24, 0.12, 0.36), 0.4);
    let pillow_2 = super::scaled_box(local, size, vec3(0.3, -0.24, 0.02), vec3(0.24, 0.12, 0.36), 0.4);
    let blanket = super::scaled_box(local, size, vec3(0.0, 0.32, 0.18), vec3(0.72, 0.3, 0.1), 0.08);
    let body = super::blend(
        super::blend(super::blend(super::blend(seat, back, 0.14), arm, 0.1), arm_2, 0.1),
        super::blend(cushion, cushion_2, 0.05),
        0.05,
    );
    let pillows = pillow.min(pillow_2).min(blanket);
    let distance = body.min(pillows);
    let tint = if pillows < body { blue } else { fabric };
    Hit { distance, material: Material::Fabric, tint }
}
