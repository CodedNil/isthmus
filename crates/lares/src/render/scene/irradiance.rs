//! Static architectural diffuse bounce, baked once; no per-pixel scene search.
use super::{FAR, lighting, trace};
use crate::home::{Kind, Material, Object};
use core::f32::consts::PI;
use isthmus::prelude::*;

const SPACING: f32 = 0.5;
#[derive(Clone, Copy, ShaderData)]
pub struct Grid {
    pub origin: Vec3,
    pub size: UVec3,
}
#[derive(Clone, Copy, Default, ShaderData)]
pub struct Probe {
    pub constant: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
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
            color += (probe.constant + probe.x * normal.x + probe.y * normal.y + probe.z * normal.z)
                .max_num(Vec3::ZERO)
                * weight;
            total += weight;
        }
        if total > 0.0001 { color / total } else { vec3(0.12, 0.13, 0.15) }
    }
}

#[cfg(not(target_arch = "spirv"))]
pub fn bake(
    objects: &[Object],
    nodes: &[super::super::acceleration::Node],
    min: Vec3,
    max: Vec3,
) -> (Grid, Box<[Probe]>) {
    // Furniture retains dynamic primary/shadow tracing. Only room transport is
    // cached: this intentionally approximates diffuse GI, not mirror reflections.
    let architecture: Vec<_> = objects.iter().filter(|o| o.kind == Kind::Box).copied().collect();
    let origin = vec3(min.x, min.y, 0.10);
    let size = ((vec3(max.x, max.y, 2.6) - origin) / SPACING).ceil().as_uvec3() + UVec3::ONE;
    let grid = Grid { origin, size };
    let raycast = |p: Vec3, d: Vec3| {
        let ray = trace::Ray::new(p, d);
        let mut nearest = FAR;
        let mut found = None;
        let mut index = 0;
        while index < nodes.len() {
            let node = nodes[index];
            let (enter, exit) = ray.bounds(node.min, node.max);
            if enter > exit || enter >= nearest {
                index = node.skip as usize;
                continue;
            }
            index += 1;
            if node.object != super::super::acceleration::BRANCH {
                let object = objects[node.object as usize];
                if object.kind == Kind::Box {
                    nearest = enter;
                    found = Some(object);
                }
            }
        }
        (nearest, found)
    };
    let mut probes = Vec::with_capacity((size.x * size.y * size.z) as usize);
    for z in 0..size.z {
        for y in 0..size.y {
            for x in 0..size.x {
                let p = origin + uvec3(x, y, z).as_vec3() * SPACING;
                let mut probe = Probe::default();
                if architecture.iter().any(|o| (p - o.pos).abs().cmplt(o.size * 0.5).all()) {
                    probe.positive = Vec3::splat(-FAR);
                    probe.negative = Vec3::splat(-FAR);
                    probes.push(probe);
                    continue;
                }
                for axis in 0..3 {
                    let mut direction = Vec3::ZERO;
                    direction[axis] = 1.0;
                    probe.positive[axis] = (raycast(p, direction).0 - 0.002).max_num(0.0);
                    probe.negative[axis] = (raycast(p, -direction).0 - 0.002).max_num(0.0);
                }
                for sample in 0..32 {
                    let h = 1.0 - 2.0 * (sample as f32 + 0.5) / 32.0;
                    let (sin, cos) = (sample as f32 * 2.399_963).sin_cos();
                    let r = (1.0 - h * h).sqrt();
                    let direction = vec3(r * cos, r * sin, h);
                    let (distance, object) = raycast(p, direction);
                    let radiance = object.map_or_else(
                        || vec3(0.18, 0.15, 0.12).lerp(vec3(0.72, 0.83, 1.0), direction.z.max_num(0.0).sqrt()),
                        |object| {
                            let hit = p + direction * distance;
                            let normal = super::object_normal(object, hit - object.pos);
                            let visibility =
                                if raycast(hit + normal * 0.002, lighting::KEY).1.is_none() { 1.0 } else { 0.0 };
                            let albedo = match object.material {
                                Material::Paint => vec3(0.70, 0.67, 0.60),
                                Material::Floorboards => vec3(0.36, 0.20, 0.095),
                                Material::Tile => vec3(0.07, 0.08, 0.09),
                                _ => vec3(0.65, 0.61, 0.53),
                            };
                            albedo
                                * (vec3(0.28, 0.30, 0.34)
                                    + lighting::RADIANCE * (normal.dot(lighting::KEY).max_num(0.0) * visibility / PI))
                        },
                    );
                    probe.constant += radiance / 32.0;
                    probe.x += radiance * (direction.x / 16.0);
                    probe.y += radiance * (direction.y / 16.0);
                    probe.z += radiance * (direction.z / 16.0);
                }
                probes.push(probe);
            }
        }
    }
    (grid, probes.into_boxed_slice())
}
