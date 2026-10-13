//! Shared ceiling fittings and the bedside table lamp, in metres.
use super::{Material, Sample, cylinder};
use isthmus::prelude::*;

pub(super) const WARM: Vec3 = vec3(1.0, 0.78, 0.50);

pub(super) fn downlight<S: Sample>(p: Vec3, size: Vec3) -> S {
    let rim = cylinder(p, size.x * 0.5, size.z);
    let recess = cylinder(p + Vec3::Z * 0.010, size.x * 0.37, size.z);
    let lens = cylinder(p + Vec3::Z * 0.008, size.x * 0.34, 0.009);
    S::new(rim.max_num(-recess), || (Material::Paint, vec3(0.94, 0.93, 0.90)))
        .union(S::new(lens, || (Material::Emissive, WARM)))
}

pub(super) fn lamp<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = cylinder(p + Vec3::Z * (size.z * 0.5 - 0.012), 0.085, 0.024);
    let stem = cylinder(p + Vec3::Z * 0.045, 0.009, 0.27);
    // A tapered linen shade, open at both ends, with a visible bulb inside.
    let shade_p = p - Vec3::Z * 0.09;
    let t = (shade_p.z / 0.18 + 0.5).clamp(0.0, 1.0);
    let radius = 0.12 - t * 0.035;
    let shell = vec2((shade_p.xy().length() - radius).abs() - 0.0025, shade_p.z.abs() - 0.09);
    let shade = (shell.max_num(Vec2::ZERO).length() + shell.max_element().min_num(0.0)) * 0.9;
    let bulb = (p - Vec3::Z * 0.065).length() - 0.025;
    S::new(base.min_num(stem), || (Material::Metal, vec3(0.15, 0.13, 0.10)))
        .union(S::new(shade, || (Material::Fabric, vec3(0.92, 0.82, 0.64))))
        .union(S::new(bulb, || (Material::Emissive, WARM)))
}

/// A finite, downward-facing emitter shared by direct light and cached bounce.
/// Returns direction, incident radiance and distance; visibility is traced separately.
pub(super) fn incident(light: super::Object, point: Vec3, emitter: Vec2) -> (Vec3, Vec3, f32) {
    let source =
        light.pos + emitter.extend(0.0) + Vec3::Z * if light.kind == super::Kind::Downlight { -0.025 } else { 0.065 };
    let delta = source - point;
    let distance = delta.length();
    if distance < 0.001 || distance >= 4.0 {
        return (Vec3::Z, Vec3::ZERO, distance);
    }
    let direction = delta / distance;
    let beam = if light.kind == super::Kind::Downlight {
        ((direction.z - 0.35) / 0.45).max_num(0.0).min_num(1.0)
    } else {
        1.0
    };
    let falloff = (1.0 - distance * distance / 16.0).max_num(0.0);
    let power = if light.kind == super::Kind::Downlight { 3.0 } else { 0.8 };
    let radiance = WARM * (light.emission * power * beam * falloff * falloff / (distance * distance + 0.05));
    (direction, radiance, distance)
}
