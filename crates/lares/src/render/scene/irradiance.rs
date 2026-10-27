//! Sparse directional irradiance; geometry transport is cached on the CPU.
use crate::render::WALL_HEIGHT;
#[cfg(not(target_arch = "spirv"))]
pub use cache::{Cache, bake};
use isthmus::prelude::*;

const SPACING: f32 = 1.0 / 3.0;
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
    /// Actual baked position after moving a blocked lattice site into free space.
    pub position: Vec3,
    /// Clearance to the nearest surface along each positive and negative axis.
    pub positive: Vec3,
    pub negative: Vec3,
}
impl Grid {
    pub fn sample(self, probes: &[Probe], point: Vec3, normal: Vec3) -> Vec3 {
        if point.z >= WALL_HEIGHT {
            return Vec3::ZERO;
        }
        let cell = ((point - self.origin) / SPACING).clamp(Vec3::ZERO, (self.size - UVec3::ONE).as_vec3() - 0.001);
        let base = cell.floor().as_uvec3();
        let fraction = cell.fract();
        let mut color = Vec3::ZERO;
        let mut total = 0.0;
        for corner in 0..8 {
            let offset = uvec3(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let index = base + offset;
            let probe = probes[(index.x + self.size.x * (index.y + self.size.y * index.z)) as usize];
            let delta = point - probe.position;
            // Axis clearance suppresses interpolation across nearby walls/floors.
            if delta.cmpgt(probe.positive + Vec3::splat(0.003)).any()
                || (-delta).cmpgt(probe.negative + Vec3::splat(0.003)).any()
            {
                continue;
            }
            let w = Vec3::ONE - (offset.as_vec3() - fraction).abs();
            let clearance = (probe.positive - delta).min(probe.negative + delta).min_element();
            // Restore the 2 mm bake guard at receivers on adjoining surfaces.
            let visibility = ((clearance + 0.003) / 0.08).clamp(0.0, 1.0);
            let weight = w.x * w.y * w.z * visibility;
            let light = probe.light;
            color += (light.constant + light.x * normal.x + light.y * normal.y + light.z * normal.z) * weight;
            total += weight;
        }
        if total > 1.0e-10 {
            return color.max_num(Vec3::ZERO) / total;
        }
        // A surface corner can have all eight lattice neighbours in walls,
        // furniture or above the ceiling. Search one cell farther only in
        // that case, retaining the same wall-clearance rejection.
        for neighbour in 0..64 {
            let offset = ivec3(neighbour & 3, (neighbour >> 2) & 3, (neighbour >> 4) & 3) - IVec3::ONE;
            let index = base.as_ivec3() + offset;
            if index.cmplt(IVec3::ZERO).any() || index.cmpge(self.size.as_ivec3()).any() {
                continue;
            }
            let index = index.as_uvec3();
            let probe = probes[(index.x + self.size.x * (index.y + self.size.y * index.z)) as usize];
            let delta = point - probe.position;
            if delta.cmpgt(probe.positive + Vec3::splat(0.003)).any()
                || (-delta).cmpgt(probe.negative + Vec3::splat(0.003)).any()
            {
                continue;
            }
            let clearance = (probe.positive - delta).min(probe.negative + delta).min_element();
            // Restore the 2 mm bake guard at receivers on adjoining surfaces.
            let visibility = ((clearance + 0.003) / 0.08).clamp(0.0, 1.0);
            let weight = visibility / delta.length_squared().max_num(0.01);
            let light = probe.light;
            color += (light.constant + light.x * normal.x + light.y * normal.y + light.z * normal.z) * weight;
            total += weight;
        }
        if total > 1.0e-10 { color.max_num(Vec3::ZERO) / total } else { Vec3::ZERO }
    }
}

#[cfg(not(target_arch = "spirv"))]
mod cache;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_receivers_preserve_constant_light_without_crossing_partitions() {
        let grid = Grid { origin: Vec3::ZERO, size: UVec3::splat(2) };
        let mut probes = [Probe::default(); 8];
        for (corner, probe) in probes.iter_mut().enumerate() {
            let offset = vec3((corner & 1) as f32, ((corner >> 1) & 1) as f32, ((corner >> 2) & 1) as f32);
            let position = Vec3::splat(0.08) + offset * SPACING;
            *probe = Probe {
                light: Irradiance { constant: vec3(0.3, 0.7, 1.0), ..Irradiance::default() },
                position,
                positive: Vec3::ONE - position - Vec3::splat(0.002),
                negative: position - Vec3::splat(0.002),
            };
        }
        for x in [0.0, 0.0001, 0.001, 0.01, 0.1] {
            for y in [0.0, 0.0001, 0.001, 0.01, 0.1] {
                for normal in [Vec3::X, Vec3::Y, Vec3::Z] {
                    let light = grid.sample(&probes, vec3(x, y, 0.0), normal);
                    assert!((light - vec3(0.3, 0.7, 1.0)).length() < 1.0e-5, "boundary lost indirect light: {light:?}");
                }
            }
        }
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            assert_eq!(grid.sample(&probes, -axis * 0.01, axis), Vec3::ZERO);
        }
    }
}
