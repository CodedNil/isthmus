//! Sparse directional irradiance; geometry transport is cached on the CPU.
use crate::{home::Object, render::WALL_HEIGHT};
#[cfg(not(target_arch = "spirv"))]
pub use baked::Controls;
#[cfg(not(target_arch = "spirv"))]
pub use cache::Cache;
#[cfg(not(target_arch = "spirv"))]
pub use cache::bake_cached;
#[cfg(not(target_arch = "spirv"))]
mod baked;
use isthmus::prelude::*;

const SPACING: f32 = 0.50;
#[derive(Clone, Copy, ShaderData)]
pub struct Grid {
    pub origin: Vec3,
    pub size: UVec3,
}
/// First-order directional irradiance. Volume sites use SH moments; projected
/// surfaces store the cosine integral and its world-space normal derivative.
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
    /// Zero for volume sites, geometric receiver normal for surface sites.
    pub normal: Vec3,
    /// Clearance to the nearest surface along each positive and negative axis.
    pub positive: Vec3,
    pub negative: Vec3,
    /// Geometry depth moments in six overlapping directional cones. These
    /// include furniture, unlike architectural clearances, and never relight.
    pub mean_positive: Vec3,
    pub mean_negative: Vec3,
    pub square_positive: Vec3,
    pub square_negative: Vec3,
}
impl Probe {
    fn receiver_weight(self, delta: Vec3, normal: Vec3) -> f32 {
        let distance = delta.length().max_num(0.0001);
        let direction = delta / distance;
        // A receiver should gather from its exposed hemisphere, rather than
        // averaging dark air underneath a cushion onto the cushion's top.
        let facing = (0.5 - 0.5 * direction.dot(normal)).max_num(0.0);
        // Never borrow an irradiance sample from behind this opaque receiver.
        // Axis clearances alone cannot reject a diagonal path around a wall
        // corner; the old nonzero back-hemisphere floor admitted adjacent-room
        // light exactly where surface-cache coverage dropped at trim/tile edges.
        let exposed = ((0.003 - delta.dot(normal)) / 0.03).clamp_num(0.0, 1.0);
        let hemisphere = (0.12 + 0.88 * facing * facing) * exposed;
        let axes = direction.abs();
        let axes = axes * axes;
        let axes = axes * axes;
        let weights = axes / axes.element_sum().max_num(0.0001);
        let mean = Vec3::select(direction.cmpge(Vec3::ZERO), self.mean_positive, self.mean_negative).dot(weights);
        if mean <= 0.0 || distance <= mean + 0.02 {
            return hemisphere;
        }
        let square = Vec3::select(direction.cmpge(Vec3::ZERO), self.square_positive, self.square_negative).dot(weights);
        let variance = (square - mean * mean).max_num(0.0004);
        let beyond = distance - mean - 0.02;
        let visibility = variance / (variance + beyond * beyond);
        hemisphere * visibility * visibility
    }
}
mod surface;

impl Grid {
    pub fn surface(probes: &[Probe], item: Object, local: Vec3, normal: Vec3) -> Vec4 {
        surface::sample(probes, item, local, normal)
    }

    pub(super) fn surface_normal(probes: &[Probe], item: Object, local: Vec3, normal: Vec3, shading: Vec3) -> Vec4 {
        surface::sample_normal(probes, item, local, normal, shading - normal)
    }

    pub fn sample(self, probes: &[Probe], point: Vec3, normal: Vec3) -> Vec3 {
        if point.z >= WALL_HEIGHT {
            return Vec3::ZERO;
        }
        let cell = ((point - self.origin) / SPACING).clamp_num(Vec3::ZERO, (self.size - UVec3::ONE).as_vec3() - 0.001);
        let base = cell.floor().as_uvec3();
        let fraction = cell.fract();
        let mut color = Vec3::ZERO;
        let mut total = 0.0;
        let mut coverage = 0.0;
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
            let visibility = ((clearance + 0.003) / 0.08).clamp_num(0.0, 1.0);
            let spatial = w.x * w.y * w.z * visibility;
            coverage += spatial;
            let weight = spatial * probe.receiver_weight(delta, normal);
            let light = probe.light;
            color += (light.constant + light.x * normal.x + light.y * normal.y + light.z * normal.z) * weight;
            total += weight;
        }
        if total <= 1.0e-20 {
            coverage = 0.0;
        }
        let primary = color.max_num(Vec3::ZERO) / total.max_num(1.0e-20);
        if coverage >= 0.05 && total > 1.0e-20 {
            return primary;
        }
        // Fade to a wider, compact cubic kernel where relocated/disabled
        // lattice sites leave a hole. Its support vanishes at cell boundaries;
        // neither changing the candidate set nor invoking fallback makes a seam.
        color = Vec3::ZERO;
        total = 0.0;
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
            let visibility = ((clearance + 0.003) / 0.08).clamp_num(0.0, 1.0);
            let lattice_delta = (offset.as_vec3() - fraction).abs();
            let cubic = |distance: f32| {
                if distance < 1.0 {
                    (4.0 - 6.0 * distance * distance + 3.0 * distance * distance * distance) / 6.0
                } else {
                    let remaining = (2.0 - distance).max_num(0.0);
                    remaining * remaining * remaining / 6.0
                }
            };
            let spatial = cubic(lattice_delta.x) * cubic(lattice_delta.y) * cubic(lattice_delta.z);
            let weight = spatial * visibility * probe.receiver_weight(delta, normal);
            let light = probe.light;
            color += (light.constant + light.x * normal.x + light.y * normal.y + light.z * normal.z) * weight;
            total += weight;
        }
        let fallback = color.max_num(Vec3::ZERO) / total.max_num(1.0e-20);
        fallback.lerp(primary, (coverage / 0.05).clamp_num(0.0, 1.0))
    }

    /// Fixed geometric weights used by CPU bounce paths; no per-update geometry arithmetic.
    #[cfg(not(target_arch = "spirv"))]
    pub(super) fn stencil(self, probes: &[Probe], point: Vec3, normal: Vec3) -> Vec<(u32, f32)> {
        if point.z >= WALL_HEIGHT {
            return Vec::new();
        }
        let cell = ((point - self.origin) / SPACING).clamp_num(Vec3::ZERO, (self.size - UVec3::ONE).as_vec3() - 0.001);
        let base = cell.floor().as_uvec3();
        let fraction = cell.fract();
        let mut taps = Vec::new();
        let mut total = 0.0;
        let mut coverage = 0.0;
        for corner in 0..8 {
            let offset = uvec3(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let index = base + offset;
            let slot = index.x + self.size.x * (index.y + self.size.y * index.z);
            let probe = probes[slot as usize];
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
            let visibility = ((clearance + 0.003) / 0.08).clamp_num(0.0, 1.0);
            let spatial = w.x * w.y * w.z * visibility;
            coverage += spatial;
            let weight = spatial * probe.receiver_weight(delta, normal);
            if weight > 0.0 {
                taps.push((slot, weight));
            }
            total += weight;
        }
        if total <= 1.0e-20 {
            coverage = 0.0;
        }
        let blend = (coverage / 0.05).clamp_num(0.0, 1.0);
        let mut primary: Vec<_> = taps.iter().map(|&(index, weight)| (index, weight / total.max(1.0e-20))).collect();
        if coverage >= 0.05 && total > 1.0e-20 {
            return primary;
        }
        // Fade to a wider, compact cubic kernel where relocated/disabled
        // lattice sites leave a hole. Its support vanishes at cell boundaries;
        // neither changing the candidate set nor invoking fallback makes a seam.
        taps.clear();
        total = 0.0;
        for neighbour in 0..64 {
            let offset = ivec3(neighbour & 3, (neighbour >> 2) & 3, (neighbour >> 4) & 3) - IVec3::ONE;
            let index = base.as_ivec3() + offset;
            if index.cmplt(IVec3::ZERO).any() || index.cmpge(self.size.as_ivec3()).any() {
                continue;
            }
            let index = index.as_uvec3();
            let slot = index.x + self.size.x * (index.y + self.size.y * index.z);
            let probe = probes[slot as usize];
            let delta = point - probe.position;
            if delta.cmpgt(probe.positive + Vec3::splat(0.003)).any()
                || (-delta).cmpgt(probe.negative + Vec3::splat(0.003)).any()
            {
                continue;
            }
            let clearance = (probe.positive - delta).min(probe.negative + delta).min_element();
            // Restore the 2 mm bake guard at receivers on adjoining surfaces.
            let visibility = ((clearance + 0.003) / 0.08).clamp_num(0.0, 1.0);
            let lattice_delta = (offset.as_vec3() - fraction).abs();
            let cubic = |distance: f32| {
                if distance < 1.0 {
                    (4.0 - 6.0 * distance * distance + 3.0 * distance * distance * distance) / 6.0
                } else {
                    let remaining = (2.0 - distance).max_num(0.0);
                    remaining * remaining * remaining / 6.0
                }
            };
            let spatial = cubic(lattice_delta.x) * cubic(lattice_delta.y) * cubic(lattice_delta.z);
            let weight = spatial * visibility * probe.receiver_weight(delta, normal);
            if weight > 0.0 {
                taps.push((slot, weight));
            }
            total += weight;
        }
        for (_, weight) in &mut primary {
            *weight *= blend;
        }
        primary.extend(taps.into_iter().map(|(index, weight)| (index, weight / total.max(1.0e-20) * (1.0 - blend))));
        primary
    }
}

#[cfg(not(target_arch = "spirv"))]
mod cache;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_reconstruction_rejects_light_behind_opaque_receivers_and_cached_weights_match() {
        let grid = Grid { origin: Vec3::splat(-0.75), size: UVec3::splat(4) };
        for normal in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
            let probes: Vec<_> = (0..64)
                .map(|index| {
                    let position = grid.origin + uvec3(index % 4, index / 4 % 4, index / 16).as_vec3() * SPACING;
                    let color = if position.dot(normal) < 0.0 { Vec3::splat(1000.0) } else { vec3(0.3, 0.7, 1.0) };
                    Probe {
                        position,
                        positive: Vec3::splat(4.0),
                        negative: Vec3::splat(4.0),
                        light: Irradiance { constant: color, ..Irradiance::default() },
                        ..Probe::default()
                    }
                })
                .collect();
            for offset in [-0.02, 0.0, 0.02] {
                let point = normal * offset;
                let actual = grid.sample(&probes, point, normal);
                assert!(
                    (actual - vec3(0.3, 0.7, 1.0)).length() < 1.0e-5,
                    "back-side light leaked: {normal:?}, {actual:?}"
                );
                let cached = grid
                    .stencil(&probes, point, normal)
                    .iter()
                    .fold(Vec3::ZERO, |sum, &(index, weight)| sum + probes[index as usize].light.constant * weight);
                assert!((cached - actual).length() < 1.0e-5, "CPU transport stencil must match GPU reconstruction");
            }
        }
    }

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
                ..Probe::default()
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
    #[test]
    fn missing_sites_preserve_constant_fields_across_interpolation_boundaries() {
        let grid = Grid { origin: Vec3::ZERO, size: UVec3::splat(4) };
        let color = vec3(0.3, 0.7, 1.0);
        let probes: Vec<_> = (0..64)
            .map(|index| {
                let coordinate = uvec3(index % 4, index / 4 % 4, index / 16);
                let position = coordinate.as_vec3() * SPACING + Vec3::splat(0.04);
                let missing = coordinate.cmpge(UVec3::ONE).all() && coordinate.cmple(UVec3::splat(2)).all();
                Probe {
                    light: Irradiance { constant: color, ..Irradiance::default() },
                    position,
                    positive: if missing { Vec3::splat(-100.0) } else { Vec3::splat(3.0) - position },
                    negative: if missing { Vec3::splat(-100.0) } else { position },
                    ..Probe::default()
                }
            })
            .collect();
        for step in 0..800 {
            let point = vec3(0.15 + step as f32 * 0.001, 0.50, 0.50);
            for normal in [Vec3::X, Vec3::Y, Vec3::Z] {
                let light = grid.sample(&probes, point, normal);
                assert!(
                    (light - color).length() < 1.0e-5,
                    "missing sites must not create dark cubes: {point:?}, {light:?}"
                );
            }
        }
        let visible = Probe {
            mean_positive: Vec3::splat(2.0),
            mean_negative: Vec3::splat(2.0),
            square_positive: Vec3::splat(4.0),
            square_negative: Vec3::splat(4.0),
            ..Probe::default()
        };
        let blocked = Probe {
            mean_positive: Vec3::splat(0.1),
            mean_negative: Vec3::splat(0.1),
            square_positive: Vec3::splat(0.0101),
            square_negative: Vec3::splat(0.0101),
            ..Probe::default()
        };
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            assert!(blocked.receiver_weight(axis * 0.5, -axis) < visible.receiver_weight(axis * 0.5, -axis) * 0.001);
            assert!(visible.receiver_weight(axis * 0.5, axis) < visible.receiver_weight(-axis * 0.5, axis));
        }
    }
}
