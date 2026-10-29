//! Automatically projected SDF surface samples: no meshes or authored UVs.
use super::Probe;
use crate::home::{Kind, Object};
#[cfg(not(target_arch = "spirv"))]
use crate::render::{WALL_HEIGHT, acceleration::Node};
#[cfg(not(target_arch = "spirv"))]
use core::array::from_fn;
use isthmus::prelude::*;

const SPACING: f32 = 0.20;

fn architectural(item: Object) -> bool {
    item.kind == Kind::Box || item.kind == Kind::Ceiling
}
fn dimensions(item: Object) -> UVec3 {
    if architectural(item) {
        let lo = ((item.pos - item.size * 0.5) / SPACING).floor();
        let hi = ((item.pos + item.size * 0.5) / SPACING).ceil();
        (hi - lo + 5.0).as_uvec3()
    } else {
        (item.size / SPACING).ceil().max_num(Vec3::ONE).as_uvec3()
    }
}
fn plane(v: Vec3, axis: u32) -> Vec2 {
    if axis == 0 {
        v.yz()
    } else if axis == 1 {
        v.xz()
    } else {
        v.xy()
    }
}
fn cell_origin(item: Object, axis: u32) -> Vec2 {
    (plane(item.pos - item.size * 0.5, axis) / SPACING).floor() - 2.0
}
fn face(item: Object, axis: u32, negative: bool) -> (u32, UVec2) {
    let size = dimensions(item);
    let (offset, shape) = if axis == 0 {
        (0, size.yz())
    } else if axis == 1 {
        (2 * size.y * size.z, size.xz())
    } else {
        (2 * (size.y * size.z + size.x * size.z), size.xy())
    };
    (item.surface_cache + offset + u32::from(negative) * shape.x * shape.y, shape)
}

#[derive(Clone, Copy, Default)]
pub(super) struct Lookup {
    indices: UVec4,
    weights: Vec4,
}
impl Lookup {
    fn sample_normal(self, probes: &[Probe], delta: Vec3) -> Vec4 {
        let light = |index: u32| {
            let field = probes[index as usize].light;
            (field.constant + field.x * delta.x + field.y * delta.y + field.z * delta.z).max_num(Vec3::ZERO)
        };
        let weight = self.weights.element_sum();
        let color = light(self.indices.x) * self.weights.x
            + light(self.indices.y) * self.weights.y
            + light(self.indices.z) * self.weights.z
            + light(self.indices.w) * self.weights.w;
        (color / weight.max_num(1.0e-10)).extend(weight.min_num(1.0))
    }

    #[cfg(not(target_arch = "spirv"))]
    pub(super) fn confidence(self) -> f32 {
        self.weights.element_sum()
    }

    #[cfg(not(target_arch = "spirv"))]
    pub(super) fn taps(self) -> [(u32, f32); 4] {
        from_fn(|i| (self.indices[i], self.weights[i]))
    }

    pub(super) fn sample(self, probes: &[Probe]) -> Vec4 {
        let weight = self.weights.element_sum();
        let color = probes[self.indices.x as usize].light.constant * self.weights.x
            + probes[self.indices.y as usize].light.constant * self.weights.y
            + probes[self.indices.z as usize].light.constant * self.weights.z
            + probes[self.indices.w as usize].light.constant * self.weights.w;
        (color / weight.max_num(1.0e-10)).extend(weight.min_num(1.0))
    }
}
pub(super) fn sample(probes: &[Probe], item: Object, local: Vec3, normal: Vec3) -> Vec4 {
    sample_normal(probes, item, local, normal, Vec3::ZERO)
}
pub(super) fn sample_normal(probes: &[Probe], item: Object, local: Vec3, normal: Vec3, delta: Vec3) -> Vec4 {
    // Blend projections continuously through rounded edges. The old dominant
    // axis selection made an otherwise smooth SDF look flat-shaded.
    let n = super::super::unrotate_z(normal, item.rotation).abs();
    let weights = n * n * n * n;
    let mut color = Vec3::ZERO;
    let mut total = 0.0;
    for axis in 0..3 {
        let weight = if axis == 0 {
            weights.x
        } else if axis == 1 {
            weights.y
        } else {
            weights.z
        };
        if weight < 0.001 {
            continue;
        }
        let value = lookup_axis(probes, item, local, normal, axis).sample_normal(probes, delta);
        let confidence = (value.w * 4.0).min_num(1.0);
        color += value.truncate() * (weight * confidence);
        total += weight * confidence;
    }
    (color / total.max_num(1.0e-10)).extend((total / weights.element_sum().max_num(1.0e-10)).min_num(1.0))
}
pub(super) fn lookup(probes: &[Probe], item: Object, local: Vec3, normal: Vec3) -> Lookup {
    let local_normal = super::super::unrotate_z(normal, item.rotation);
    let magnitude = local_normal.abs();
    let axis =
        if magnitude.z >= magnitude.x && magnitude.z >= magnitude.y { 2 } else { u32::from(magnitude.x < magnitude.y) };
    lookup_axis(probes, item, local, normal, axis)
}
fn lookup_axis(probes: &[Probe], item: Object, local: Vec3, normal: Vec3, axis: u32) -> Lookup {
    let local_normal = super::super::unrotate_z(normal, item.rotation);
    let negative = if axis == 0 {
        local_normal.x < 0.0
    } else if axis == 1 {
        local_normal.y < 0.0
    } else {
        local_normal.z < 0.0
    };
    let (offset, shape) = face(item, axis, negative);
    let uv = plane(local, axis);
    let cell = if architectural(item) {
        (uv + plane(item.pos, axis)) / SPACING - cell_origin(item, axis)
    } else {
        (uv / plane(item.size, axis) + 0.5) * shape.as_vec2() - 0.5
    };
    let cell = cell.max_num(Vec2::ZERO).min_num((shape - UVec2::ONE).as_vec2());
    let base = cell.floor().as_uvec2();
    let fraction = cell.fract();
    let point = item.pos + super::super::rotate_z(local, item.rotation);
    let mut result = Lookup::default();
    for corner in 0..4 {
        let step = uvec2(corner & 1, (corner >> 1) & 1);
        let index = (base + step).min(shape - UVec2::ONE);
        let slot = offset + index.x + shape.x * index.y;
        let probe = probes[slot as usize];
        if probe.normal.length_squared() < 0.5 {
            continue;
        }
        // Padded world-aligned charts can span a perpendicular partition.
        // Use the same architectural bounds as the volume reconstruction;
        // matching normals/planes alone does not establish visibility.
        let delta = point - probe.position;
        if delta.cmpgt(probe.positive + 0.003).any() || (-delta).cmpgt(probe.negative + 0.003).any() {
            continue;
        }
        let normal_weight = ((probe.normal.dot(normal) - 0.45) / 0.55).max_num(0.0);
        // Prevent a top projection from crossing a step onto a lower cushion.
        let separation = (point - probe.position).dot(normal).abs();
        let plane_weight = (1.0 - separation / 0.12).max_num(0.0);
        let w = Vec2::ONE - (step.as_vec2() - fraction).abs();
        let weight = w.x * w.y * normal_weight * plane_weight;
        if corner == 0 {
            result.indices.x = slot;
            result.weights.x = weight;
        } else if corner == 1 {
            result.indices.y = slot;
            result.weights.y = weight;
        } else if corner == 2 {
            result.indices.z = slot;
            result.weights.z = weight;
        } else {
            result.indices.w = slot;
            result.weights.w = weight;
        }
    }
    result
}

#[cfg(not(target_arch = "spirv"))]
pub(super) struct Sites {
    pub probes: Vec<Probe>,
    pub footprints: Vec<[Vec3; 2]>,
    pub neighbours: Vec<[u32; 9]>,
}

#[cfg(not(target_arch = "spirv"))]
pub(super) fn sites(objects: &mut [Object], nodes: &[Node], base: u32) -> Sites {
    use crate::render::scene::{self, FAR};
    let mut result = Vec::new();
    let mut footprints = Vec::new();
    let mut neighbours = Vec::new();
    for index in 0..objects.len() {
        let mut item = objects[index];
        // A single depth projection covers architecture and the footstool.
        // Layered fronts, sofa seats/pillows and handles need multiple depth layers;
        // retain the directional volume there rather than switching layers.
        // Sub-grid-height trim uses the volume, just like other fine detail.
        // Its padded charts would otherwise consist almost entirely of holes.
        let trim = item.kind == Kind::Box && item.pos.z > 0.0 && item.size.z < SPACING;
        if trim || !matches!(item.kind, Kind::Box | Kind::Ceiling | Kind::Ottoman) {
            continue;
        }
        item.surface_cache = base + result.len() as u32;
        let size = dimensions(item);
        let count = 2 * (size.x * size.y + size.x * size.z + size.y * size.z);
        result.resize(result.len() + count as usize, Probe {
            positive: Vec3::splat(-FAR),
            negative: Vec3::splat(-FAR),
            ..Probe::default()
        });
        footprints.resize(result.len(), [Vec3::ZERO; 2]);
        neighbours.resize(result.len(), [u32::MAX; 9]);
        for axis in 0..3 {
            for negative in [false, true] {
                let direction = if axis == 0 {
                    Vec3::X
                } else if axis == 1 {
                    Vec3::Y
                } else {
                    Vec3::Z
                } * if negative { -1.0 } else { 1.0 };
                let (offset, shape) = face(item, axis, negative);
                for y in 0..shape.y {
                    for x in 0..shape.x {
                        let slot = (offset - base + x + shape.x * y) as usize;
                        for (tap, neighbour) in neighbours[slot].iter_mut().enumerate() {
                            let nx = x as i32 + (tap % 3) as i32 - 1;
                            let ny = y as i32 + (tap / 3) as i32 - 1;
                            if nx >= 0 && ny >= 0 && nx < shape.x as i32 && ny < shape.y as i32 {
                                *neighbour = offset + nx as u32 + shape.x * ny as u32;
                            }
                        }
                        let uv = if architectural(item) {
                            (cell_origin(item, axis) + vec2(x as f32, y as f32)) * SPACING - plane(item.pos, axis)
                        } else {
                            ((vec2(x as f32, y as f32) + 0.5) / shape.as_vec2() - 0.5) * plane(item.size, axis)
                        };
                        let local = if axis == 0 {
                            vec3(0.0, uv.x, uv.y)
                        } else if axis == 1 {
                            vec3(uv.x, 0.0, uv.y)
                        } else {
                            vec3(uv.x, uv.y, 0.0)
                        };
                        let extent = item.extent();
                        let distance = extent.dot(direction.abs()) + 0.025;
                        let origin = local + direction * distance;
                        let travelled = if architectural(item) {
                            // Padding gathers the neighbouring coplanar solid,
                            // so construction joints never become lighting seams.
                            let world = item.pos + origin;
                            let (depth, _) = scene::architectural_cast(world, -direction, nodes, objects);
                            if (depth - 0.025).abs() > 0.003 {
                                continue;
                            }
                            depth
                        } else {
                            scene::trace::march(origin, -direction, 0.0, distance * 2.0, 0.0, |p| {
                                scene::render_furniture::<scene::Distance>(item.kind, p, item.size).0
                            })
                        };
                        if travelled >= FAR {
                            continue;
                        }
                        let hit = origin - direction * travelled;
                        let normal = if architectural(item) { direction } else { scene::object_normal(item, hit) };
                        if normal.dot(direction) < 0.65 {
                            continue;
                        }
                        let planar = normal.dot(direction) > 0.995;
                        let normal = scene::rotate_z(normal, item.rotation);
                        let point = item.pos + scene::rotate_z(hit, item.rotation) + normal * 0.002;
                        if point.z <= 0.0 || point.z >= WALL_HEIGHT {
                            continue;
                        }
                        let (roof, object) = scene::architectural_cast(point, Vec3::Z, nodes, objects);
                        if roof >= FAR || objects[object as usize].kind != Kind::Ceiling {
                            continue;
                        }
                        // Integrate a planar texel's footprint, rather than
                        // magnifying a millimetre-wide ornament's occlusion
                        // when its centre happens to land on that ornament.
                        if planar {
                            let (u, v) = if axis == 0 {
                                (Vec3::Y * item.size.y, Vec3::Z * item.size.z)
                            } else if axis == 1 {
                                (Vec3::X * item.size.x, Vec3::Z * item.size.z)
                            } else {
                                (Vec3::X * item.size.x, Vec3::Y * item.size.y)
                            };
                            footprints[slot] = [
                                scene::rotate_z(
                                    if architectural(item) {
                                        u.normalize() * (SPACING * 0.5)
                                    } else {
                                        u / shape.x as f32 * 0.5
                                    },
                                    item.rotation,
                                ),
                                scene::rotate_z(
                                    if architectural(item) {
                                        v.normalize() * (SPACING * 0.5)
                                    } else {
                                        v / shape.y as f32 * 0.5
                                    },
                                    item.rotation,
                                ),
                            ];
                        }
                        result[slot] = Probe {
                            position: point,
                            normal,
                            positive: Vec3::splat(FAR),
                            negative: Vec3::splat(FAR),
                            ..Probe::default()
                        };
                    }
                }
            }
        }
        objects[index].surface_cache = item.surface_cache;
    }
    Sites { probes: result, footprints, neighbours }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        home::{Material, Room, Walls},
        render::{Scene, SunSettings},
    };

    #[test]
    fn projected_interpolation_does_not_borrow_light_across_partitions() {
        let rooms = [
            Room::new(vec2(-1.6, 0.0), Vec2::splat(3.0), Material::Floorboards),
            Room::new(vec2(1.6, 0.0), Vec2::splat(3.0), Material::Floorboards),
        ];
        let mut scene = Scene::new_at(&rooms, 1_782_043_200.0, SunSettings::default());
        for probe in &mut scene.probes {
            probe.light.constant = if probe.position.x < 0.0 { Vec3::splat(100.0) } else { Vec3::ZERO };
        }
        let floor = *scene
            .objects
            .iter()
            .find(|item| item.kind == Kind::Box && item.material == Material::Floorboards && item.pos.x > 0.0)
            .unwrap();
        for distance in [0.003, 0.01, 0.03, 0.08, 0.15] {
            for y in [-1.0, 0.0, 1.0] {
                let point = vec3(floor.pos.x - floor.size.x * 0.5 + distance, y, floor.pos.z + floor.size.z * 0.5);
                let value = sample(&scene.probes, floor, point - floor.pos, Vec3::Z);
                assert!(
                    value.truncate().max_element() < 0.0001,
                    "projected cache leaked across a wall at {point:?}: {value:?}"
                );
            }
        }
    }

    #[test]
    fn coplanar_construction_joints_preserve_continuous_irradiance() {
        let rooms = [
            Room::new(vec2(-0.54, 0.0), vec2(1.34, 2.0), Material::Floorboards).walls(Walls::empty()),
            Room::new(vec2(0.56, 0.0), vec2(0.86, 2.0), Material::Floorboards).walls(Walls::empty()),
        ];
        let mut scene = Scene::new_at(&rooms, 1_782_043_200.0, SunSettings::default());
        for probe in &mut scene.probes {
            probe.light.constant = Vec3::splat(2.0 + probe.position.x * 0.3 + probe.position.y * 0.4);
        }
        for ceiling in [false, true] {
            let mut pieces: Vec<_> = scene
                .objects
                .iter()
                .copied()
                .filter(|o| {
                    if ceiling {
                        o.kind == Kind::Ceiling
                    } else {
                        o.kind == Kind::Box && o.material == Material::Floorboards
                    }
                })
                .collect();
            pieces.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
            assert_eq!(pieces.len(), 2);
            let joint = pieces[0].pos.x + pieces[0].size.x * 0.5;
            let normal = if ceiling { -Vec3::Z } else { Vec3::Z };
            let z = pieces[0].pos.z + normal.z * pieces[0].size.z * 0.5;
            for y in [-0.63, -0.17, 0.24, 0.61] {
                let a = vec3(joint - 0.00001, y, z);
                let b = vec3(joint + 0.00001, y, z);
                let left = sample(&scene.probes, pieces[0], a - pieces[0].pos, normal);
                let right = sample(&scene.probes, pieces[1], b - pieces[1].pos, normal);
                assert!(left.w > 0.95 && right.w > 0.95, "joint lost surface coverage");
                assert!(
                    (left.truncate() - right.truncate()).length() < 0.001,
                    "construction seam: {left:?} vs {right:?}"
                );
            }
        }
    }
}
