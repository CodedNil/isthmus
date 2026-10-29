//! Shared ceiling fittings and the woven lounge table lamp, in metres.
use super::{Material, Sample, cylinder};
use crate::{
    home::DEFAULT_LIGHT_COLOR,
    render::photometry::{GU10_BEAM_MIX, GU10_PEAK_CANDELA, LIGHT_RANGE, UNITS},
};
use core::f32::consts::PI;
use isthmus::prelude::*;

pub(super) const WARM: Vec3 = DEFAULT_LIGHT_COLOR;

pub(super) fn downlight<S: Sample>(p: Vec3, size: Vec3) -> S {
    let rim = cylinder(p, size.x * 0.5, size.z);
    let recess = cylinder(p + Vec3::Z * 0.010, size.x * 0.26, size.z);
    let lens = cylinder(p + Vec3::Z * 0.008, size.x * 0.225, 0.009);
    S::new(rim.max_num(-recess), || (Material::Paint, vec3(0.94, 0.93, 0.90)))
        .union(S::new(lens, || (Material::Emissive, WARM)))
}

pub(super) fn lamp<S: Sample>(p: Vec3, size: Vec3) -> S {
    super::decor::table_lamp(p + Vec3::Z * size.z * 0.5)
}

/// A finite, downward-facing emitter shared by direct light and cached bounce.
/// Returns direction, incident radiance and distance; visibility is traced separately.
pub(super) fn incident(light: super::Object, point: Vec3, emitter: Vec2) -> (Vec3, Vec3, f32) {
    let downlight = light.kind == super::Kind::Downlight;
    let source = light.pos + emitter.extend(0.0) + Vec3::Z * if downlight { -0.025 } else { 0.005 };
    let delta = source - point;
    let distance = delta.length();
    if distance < 0.001 || distance >= LIGHT_RANGE {
        return (Vec3::Z, Vec3::ZERO, distance);
    }
    let direction = delta / distance;
    // A normalized polynomial lobe preserves rated lumens and 40° FWHM.
    let intensity = if downlight {
        let c = direction.z.max_num(0.0);
        let c2 = c * c;
        let c4 = c2 * c2;
        let c8 = c4 * c4;
        let beam = c8 * c2 * c * (1.0 + GU10_BEAM_MIX * (c - 1.0));
        beam * (GU10_PEAK_CANDELA / UNITS)
    } else {
        // The woven lamp's bulb is unknown: explicit 470 lm isotropic assumption.
        470.0 / (4.0 * PI * UNITS)
    };
    let fade = (LIGHT_RANGE - distance).clamp_num(0.0, 1.0);
    let radiance = light.emission_color * (light.emission * intensity * fade / (distance * distance).max_num(0.0025));
    (direction, radiance, distance)
}
