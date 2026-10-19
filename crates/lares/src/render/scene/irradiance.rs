//! Sparse directional irradiance; geometry transport is cached on the CPU.
#[cfg(not(target_arch = "spirv"))]
pub use cache::{Cache, bake};
use isthmus::prelude::*;

const SPACING: f32 = 0.5;
#[derive(Clone, Copy, ShaderData)]
pub struct Grid {
    pub origin: Vec3,
    pub size: UVec3,
}
/// First-order directional irradiance.
#[derive(Clone, Copy, Default, ShaderData)]
pub struct Irradiance {
    pub constant: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}
#[derive(Clone, Copy, Default, ShaderData)]
pub struct Probe {
    pub light: Irradiance,
    /// Clearance to the nearest surface along each positive and negative axis.
    pub positive: Vec3,
    pub negative: Vec3,
}
impl Grid {
    pub fn sample(self, probes: &[Probe], point: Vec3, normal: Vec3) -> Vec3 {
        let cell = ((point - self.origin) / SPACING).clamp(Vec3::ZERO, (self.size - UVec3::ONE).as_vec3() - 0.001);
        let base = cell.floor().as_uvec3();
        let fraction = cell.fract();
        let mut color = Vec3::ZERO;
        let mut total = 0.0;
        for corner in 0..8 {
            let offset = uvec3(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let index = base + offset;
            let probe = probes[(index.x + self.size.x * (index.y + self.size.y * index.z)) as usize];
            let delta = point - (self.origin + index.as_vec3() * SPACING);
            // Axis clearance suppresses interpolation across nearby walls/floors.
            if delta.cmpgt(probe.positive).any() || (-delta).cmpgt(probe.negative).any() {
                continue;
            }
            let w = Vec3::ONE - (offset.as_vec3() - fraction).abs();
            let weight = w.x * w.y * w.z;
            let light = probe.light;
            color += (light.constant + light.x * normal.x + light.y * normal.y + light.z * normal.z)
                .max_num(Vec3::ZERO)
                * weight;
            total += weight;
        }
        if total > 0.0001 { color / total } else { Vec3::ZERO }
    }
}

#[cfg(not(target_arch = "spirv"))]
mod cache;
