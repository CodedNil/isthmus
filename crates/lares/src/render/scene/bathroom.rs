use super::{Hit, Material};
use isthmus::prelude::*;

pub(super) fn bath(local: Vec3, size: Vec3) -> Hit {
    let shell = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.05);
    let hollow = super::scaled_box(local, size, vec3(0.0, 0.0, 0.15), vec3(0.78, 0.86, 0.9), 0.08);
    let distance = shell.max(-hollow);
    Hit { distance, material: Material::Ceramic, tint: vec3(0.8, 0.8, 0.78) }
}

pub(super) fn shower(local: Vec3, size: Vec3) -> Hit {
    let tray = super::scaled_box(local, size, vec3(0.0, 0.0, -0.44), vec3(1.0, 1.0, 0.12), 0.03);
    let glass = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02);
    let inside = super::scaled_box(local, size, Vec3::ZERO, vec3(0.93, 0.93, 1.2), 0.02);
    let distance = tray.min(glass.max(-inside));
    Hit { distance, material: Material::Ceramic, tint: vec3(0.8, 0.8, 0.78) }
}

pub(super) fn sink(local: Vec3, size: Vec3) -> Hit {
    let rim = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.03);
    let bowl = super::scaled_box(local, size, vec3(0.0, 0.0, 0.05), vec3(0.7, 0.7, 0.8), 0.2);
    let distance = rim.max(-bowl);
    Hit { distance, material: Material::Ceramic, tint: vec3(0.8, 0.8, 0.78) }
}

pub(super) fn toilet(local: Vec3, size: Vec3) -> Hit {
    let pan = super::scaled_box(local, size, vec3(0.0, 0.2, -0.3), vec3(0.7, 0.7, 0.5), 0.1);
    let bowl = (local - vec3(0.0, 0.2, -0.08) * size).length() - size.x * 0.3;
    let tank = super::scaled_box(local, size, vec3(0.0, -0.42, 0.1), vec3(0.9, 0.16, 0.7), 0.06);
    let distance = super::blend(super::blend(pan, bowl, 0.04), tank, 0.05);
    Hit { distance, material: Material::Ceramic, tint: vec3(0.8, 0.8, 0.78) }
}
