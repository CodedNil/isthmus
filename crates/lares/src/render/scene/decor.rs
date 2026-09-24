use super::{Hit, Material};
use isthmus::prelude::*;

pub(super) fn plant(local: Vec3, _size: Vec3) -> Hit {
    let pot = super::taper(local, vec3(0.0, 0.0, -0.95), vec3(0.0, 0.0, -0.64), 0.14, 0.19);
    let soil = super::taper(local, vec3(0.0, 0.0, -0.68), vec3(0.0, 0.0, -0.62), 0.17, 0.17);
    let crown = vec3(0.0, 0.0, -0.5);
    let frond = super::frond_of(local, crown, Vec2::X, 0.4)
        .min(super::frond_of(local, crown, Vec2::Y, 0.4))
        .min(super::frond_of(local, crown, vec2(-1.0, 0.0), 0.4))
        .min(super::frond_of(local, crown, vec2(0.0, -1.0), 0.4))
        .min(super::frond_of(local, crown, vec2(0.71, 0.71), 0.34));
    let distance = pot.min(soil).min(frond);
    let (material, tint) = if frond < pot.min(soil) {
        (Material::Foliage, vec3(0.25, 0.4, 0.22))
    } else if soil < pot {
        (Material::Soil, vec3(0.24, 0.15, 0.09))
    } else {
        (Material::Ceramic, vec3(0.56, 0.34, 0.22))
    };
    Hit { distance, material, tint }
}

pub(super) fn wall_art(local: Vec3, _size: Vec3) -> Hit {
    // Reuse one bird shape for three swallows.
    let bird = local - vec3(0.0, 0.0, 0.26);
    let slot = (bird.x / 0.55).round().clamp(-1.0, 1.0);
    let bird = bird - vec3(slot * 0.55, 0.0, 0.0);
    let body = super::rounded_box(bird, vec3(0.14, 0.045, 0.09), 0.04);
    let wing = super::capsule(bird, vec3(-0.05, 0.0, 0.0), vec3(-0.14, 0.0, 0.06), 0.024);
    let wing_2 = super::capsule(bird, vec3(0.05, 0.0, 0.0), vec3(0.14, 0.0, 0.06), 0.024);
    let tail = super::capsule(bird, vec3(-0.01, 0.0, -0.02), vec3(-0.11, 0.0, -0.08), 0.02);
    let swallows = super::blend(super::blend(body, wing, 0.03), wing_2, 0.03).min(tail);
    let spear = super::capsule(local, vec3(-0.88, 0.0, -0.12), vec3(0.88, 0.0, -0.12), 0.018).min(super::rounded_box(
        local - vec3(0.74, 0.0, -0.12),
        vec3(0.28, 0.04, 0.08),
        0.02,
    ));
    let spear_2 = super::capsule(local, vec3(-0.8, 0.0, -0.26), vec3(0.8, 0.0, -0.26), 0.018).min(super::rounded_box(
        local - vec3(0.68, 0.0, -0.26),
        vec3(0.26, 0.04, 0.07),
        0.02,
    ));
    let distance = swallows.min(spear).min(spear_2);
    Hit { distance, material: Material::Paint, tint: vec3(0.45, 0.26, 0.12) }
}
