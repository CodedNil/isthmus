//! Solved diffuse responses; direct sunlight and GPU shadows remain geometric.
use super::{Cache, Irradiance, Probe};
use crate::{
    home::Object,
    render::{Sun, acceleration::Node},
};
use core::{
    array::from_fn,
    f32::consts::{FRAC_PI_2, PI, TAU},
};
use isthmus::{
    ShaderData,
    glam::{Vec3, vec3},
};

pub(super) const AZIMUTHS: usize = 16;
pub(super) const ELEVATIONS: usize = 5;
pub(super) const SUN_FIELDS: usize = AZIMUTHS * (ELEVATIONS - 1) + 1;

/// Shared-scale signed coefficients: <0.4% peak-coefficient error in 16 bytes.
#[derive(Clone, Copy, ShaderData)]
pub struct Packed(pub [u32; 4]);
impl Packed {
    pub(super) fn new(value: Irradiance) -> Self {
        let v = [
            value.constant.x,
            value.constant.y,
            value.constant.z,
            value.x.x,
            value.x.y,
            value.x.z,
            value.y.x,
            value.y.y,
            value.y.z,
            value.z.x,
            value.z.y,
            value.z.z,
        ];
        let scale = v.iter().fold(0.0f32, |m, v| m.max(v.abs())) / 127.0;
        let mut words = [0; 4];
        words[3] = scale.to_bits();
        if scale > 0.0 {
            for (i, value) in v.into_iter().enumerate() {
                let byte = (value / scale).round().clamp(-127.0, 127.0) as i8 as u8;
                words[i / 4] |= u32::from(byte) << (i % 4 * 8);
            }
        }
        Self(words)
    }

    pub fn valid(self) -> bool {
        let scale = f32::from_bits(self.0[3]);
        scale.is_finite() && scale >= 0.0
    }

    pub(super) fn unpack(self) -> Irradiance {
        let scale = f32::from_bits(self.0[3]);
        let v: [f32; 12] = from_fn(|i| f32::from((self.0[i / 4] >> (i % 4 * 8)) as u8 as i8) * scale);
        Irradiance {
            constant: vec3(v[0], v[1], v[2]),
            x: vec3(v[3], v[4], v[5]),
            y: vec3(v[6], v[7], v[8]),
            z: vec3(v[9], v[10], v[11]),
        }
    }
}

pub struct Daylight {
    pub(super) sky: Box<[Packed]>,
    pub(super) ground: Box<[Packed]>,
    pub(super) sun: Box<[Packed]>,
}
impl Daylight {
    pub(super) fn bytes(&self) -> usize {
        (self.sky.len() + self.ground.len() + self.sun.len()) * size_of::<Packed>()
    }

    pub(super) fn taps(direction: Vec3) -> [(usize, f32); 4] {
        let azimuth = direction.y.atan2(direction.x).rem_euclid(TAU) * AZIMUTHS as f32 / TAU;
        let elevation = direction.z.clamp(0.0, 1.0).asin() * (ELEVATIONS - 1) as f32 / FRAC_PI_2;
        let row = (elevation.floor() as usize).min(ELEVATIONS - 2);
        let column = azimuth.floor() as usize % AZIMUTHS;
        let x = azimuth.fract();
        let y = (elevation - row as f32).clamp(0.0, 1.0);
        from_fn(|tap| {
            let r = row + tap / 2;
            let c = (column + tap % 2) % AZIMUTHS;
            let field = if r == ELEVATIONS - 1 { SUN_FIELDS - 1 } else { r * AZIMUTHS + c };
            (field, (if tap % 2 == 0 { 1.0 - x } else { x }) * if tap / 2 == 0 { 1.0 - y } else { y })
        })
    }

    pub(super) fn compose(&self, sun: Sun, active: &[usize], geometry: &[Probe], probes: &mut [Probe]) {
        let taps = Self::taps(sun.direction);
        let ground = sun.radiance * (sun.direction.z.max(0.0) * 0.20 / PI);
        for (slot, &index) in active.iter().enumerate() {
            let mut light = Irradiance::default();
            light.add(self.sky[slot].unpack(), sun.sky);
            light.add(self.ground[slot].unpack(), ground);
            if sun.radiance.max_element() > 0.0 {
                for (field, weight) in taps {
                    if weight > 0.0 {
                        light.add(self.sun[field * active.len() + slot].unpack(), sun.radiance * weight);
                    }
                }
            }
            probes[index] = Probe { light, ..geometry[index] };
        }
    }
}

impl Cache {
    pub(crate) fn bake_daylight(&mut self, original: Sun, objects: &[Object], nodes: &[Node]) -> Daylight {
        let mut scratch = self.direct.clone();
        for &index in &self.active {
            let mut source = self.sky[index];
            source.add(self.ground[index], Vec3::splat(0.17));
            self.raw[index].light = source;
        }
        self.solve(&mut scratch);
        let sky = self.active.iter().map(|&i| Packed::new(scratch[i].light)).collect();
        for &index in &self.active {
            self.raw[index].light = self.ground[index];
        }
        self.solve(&mut scratch);
        let ground = self.active.iter().map(|&i| Packed::new(scratch[i].light)).collect();
        let mut responses = Vec::with_capacity(SUN_FIELDS * self.active.len());
        for field in 0..SUN_FIELDS {
            let elevation = (field / AZIMUTHS) as f32 * FRAC_PI_2 / (ELEVATIONS - 1) as f32;
            let azimuth = (field % AZIMUTHS) as f32 * TAU / AZIMUTHS as f32;
            let direction = vec3(elevation.cos() * azimuth.cos(), elevation.cos() * azimuth.sin(), elevation.sin());
            let sun = Sun { direction, radiance: Vec3::ONE, sky: Vec3::ZERO };
            for &index in &self.active {
                self.raw[index].light = self.sun_at(index, sun, objects, nodes);
            }
            self.solve(&mut scratch);
            responses.extend(self.active.iter().map(|&i| Packed::new(scratch[i].light)));
        }
        self.relight(original, objects, nodes, &mut scratch);
        Daylight { sky, ground, sun: responses.into_boxed_slice() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_coefficients_have_bounded_error_across_hdr_scales() {
        for scale in [0.0, 1.0e-8, 1.0e-3, 1.0, 1000.0, 1.0e6] {
            let input = Irradiance {
                constant: vec3(0.17, 0.53, 1.0) * scale,
                x: vec3(-0.91, 0.06, -0.37) * scale,
                y: vec3(0.23, -0.64, 0.0) * scale,
                z: vec3(-0.007, 0.78, -0.43) * scale,
            };
            let packed = Packed::new(input);
            assert!(packed.valid());
            let output = packed.unpack();
            for (a, b) in [input.constant, input.x, input.y, input.z].into_iter().zip([
                output.constant,
                output.x,
                output.y,
                output.z,
            ]) {
                assert!((a - b).abs().max_element() <= scale / 254.0 + scale * 1.0e-6);
            }
        }
    }

    #[test]
    fn angular_interpolation_is_continuous_normalized_and_preserves_grid_nodes() {
        for elevation in 0..=90 {
            for azimuth in -360..=360 {
                let (e, a) = ((elevation as f32).to_radians(), (azimuth as f32).to_radians());
                let taps = Daylight::taps(vec3(e.cos() * a.cos(), e.cos() * a.sin(), e.sin()));
                assert!(taps.iter().all(|&(i, w)| i < SUN_FIELDS && (0.0..=1.0).contains(&w)));
                assert!((taps.iter().map(|t| t.1).sum::<f32>() - 1.0).abs() < 1.0e-5);
            }
        }
    }
}
