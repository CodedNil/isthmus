//! Short-range SDF visibility. A static cell index avoids per-pixel BVH walks.
use crate::home::{Kind, Material, Object};
#[cfg(not(target_arch = "spirv"))]
use crate::render::{
    WALL_HEIGHT,
    acceleration::{BRANCH, Node},
};
use isthmus::prelude::*;

const CELL: f32 = 0.40;
const WIDE: f32 = 0.12;

/// One cosine-weighted visibility ray per existing accumulation sample. Unlike
/// a distance deficit, a conservative SDF only controls stepping, not darkness.
fn contact_hit(item: Object, point: Vec3, direction: Vec3) -> f32 {
    let local = super::unrotate_z(point - item.pos, item.rotation);
    let ray = super::unrotate_z(direction, item.rotation);
    let extent = item.extent();
    let (enter, exit) = super::trace::Ray::new(local, ray).bounds(-extent, extent);
    if enter > exit || enter >= WIDE {
        return WIDE;
    }
    if item.kind == Kind::Box {
        return enter;
    }
    // A 12 cm contact query needs neither the primary ray's subpixel
    // refinement nor its 192-step over-relaxation state. Unresolved grazing
    // rays remain visible rather than inventing an occluder at the budget cap.
    let end = exit.min_num(WIDE);
    let mut travelled = enter;
    for _ in 0..16 {
        let distance =
            super::render_furniture::<super::Distance>(item.kind, local + ray * travelled, item.size).0.abs();
        if distance < 0.0005 {
            return travelled;
        }
        travelled += distance;
        if travelled > end {
            return WIDE;
        }
    }
    WIDE
}

/// Ambient visibility only; direct lighting retains its own shadow visibility.
#[inline(never)]
pub(super) fn visibility(
    data: &[u32],
    objects: &[Object],
    receiver: u32,
    point: Vec3,
    normal: Vec3,
    sample: u32,
) -> f32 {
    let origin = vec3(f32::from_bits(data[0]), f32::from_bits(data[1]), f32::from_bits(data[2]));
    let size = uvec3(data[3], data[4], data[5]);
    let cell = ((point - origin) / CELL).floor().max_num(Vec3::ZERO).min_num((size - UVec3::ONE).as_vec3()).as_uvec3();
    let slot = (cell.x + size.x * (cell.y + size.y * cell.z)) as usize;
    let mut index = data[6 + slot] as usize;
    let end = data[7 + slot] as usize;
    let phase = (sample % 4) as f32;
    let radius = ((phase + 0.5) / 4.0).sqrt();
    let angle = phase * 2.399_963_1;
    let tangent =
        if normal.z.abs() < 0.9 { normal.cross(Vec3::Z).normalize() } else { normal.cross(Vec3::X).normalize() };
    let direction = tangent * (radius * angle.cos())
        + normal.cross(tangent) * (radius * angle.sin())
        + normal * (1.0 - radius * radius).sqrt();
    let start = point + normal * 0.0008;
    let mut closest = WIDE;
    while index < end {
        let object = data[index];
        index += 1;
        let item = objects[object as usize];
        // Architectural occlusion is already integrated into the GI field.
        // Applying it twice produces artificial dark wall-corner outlines.
        if object != receiver
            && !(item.kind == Kind::Ceiling || (item.kind == Kind::Box && item.material == Material::Paint))
        {
            closest = closest.min_num(contact_hit(item, start, direction));
        }
    }
    let contact = (1.0 - closest / WIDE).max_num(0.0);
    1.0 - 0.5 * contact * contact
}

#[cfg(not(target_arch = "spirv"))]
pub fn build(objects: &[Object], nodes: &[Node], min: Vec3, max: Vec3) -> Box<[u32]> {
    let origin = min;
    let size = ((max.min(Vec3::new(max.x, max.y, WALL_HEIGHT)) - origin) / CELL).ceil().max(Vec3::ONE).as_uvec3();
    let count = (size.x * size.y * size.z) as usize;
    let mut cells = vec![Vec::new(); count];
    for node in nodes.iter().filter(|node| node.object != BRANCH) {
        let item = objects[node.object as usize];
        // Tiny foliage retains its material/fibre visibility. Contact gathering
        // concentrates on solid furnishings and the architectural envelope.
        if item.is_light()
            || matches!(item.kind, Kind::Leaf | Kind::Petal | Kind::Stem | Kind::Ceiling)
            || (item.kind == Kind::Box && item.material == Material::Paint)
        {
            continue;
        }
        let lo = ((node.min - Vec3::splat(WIDE * 2.0) - origin) / CELL).floor().as_ivec3().max(IVec3::ZERO).as_uvec3();
        let hi = ((node.max + Vec3::splat(WIDE * 2.0) - origin) / CELL)
            .floor()
            .as_ivec3()
            .max(IVec3::ZERO)
            .as_uvec3()
            .min(size - UVec3::ONE);
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    cells[(x + size.x * (y + size.y * z)) as usize].push(node.object);
                }
            }
        }
    }
    let mut data = vec![0; 6 + count + 1];
    data[..6].copy_from_slice(&[origin.x.to_bits(), origin.y.to_bits(), origin.z.to_bits(), size.x, size.y, size.z]);
    for (index, cell) in cells.iter().enumerate() {
        data[6 + index] = data.len() as u32;
        data.extend_from_slice(cell);
    }
    data[6 + count] = data.len() as u32;
    data.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::acceleration::Node;

    fn averaged(data: &[u32], objects: &[Object], receiver: u32, point: Vec3, normal: Vec3) -> f32 {
        (0..8).map(|sample| visibility(data, objects, receiver, point, normal, sample)).sum::<f32>() / 8.0
    }

    fn index(objects: &[Object]) -> Box<[u32]> {
        let nodes: Vec<_> = objects
            .iter()
            .enumerate()
            .map(|(id, item)| {
                let extent = item.extent();
                let spin = item.rotation.abs();
                let world =
                    vec3(extent.x * spin.y + extent.y * spin.x, extent.x * spin.x + extent.y * spin.y, extent.z);
                Node { min: item.pos - world, max: item.pos + world, object: id as u32, skip: id as u32 + 1 }
            })
            .collect();
        build(objects, &nodes, vec3(-2.0, -2.0, -0.12), vec3(2.0, 2.0, 2.4))
    }

    #[test]
    fn contacts_are_local_bounded_and_do_not_shadow_the_receiver_itself() {
        let floor = Object::new(Kind::Box, vec3(0.0, 0.0, -0.06), vec3(4.0, 4.0, 0.12), Material::Paint);
        let leg = Object::new(Kind::Box, vec3(0.0, 0.0, 0.25), vec3(0.10, 0.10, 0.50), Material::Wood);
        let objects = [floor, leg];
        let data = index(&objects);
        let close = averaged(&data, &objects, 0, vec3(0.052, 0.0, 0.0), Vec3::Z);
        let far = averaged(&data, &objects, 0, vec3(0.5, 0.0, 0.0), Vec3::Z);
        assert!((0.5..0.99).contains(&close));
        assert!((far - 1.0).abs() < 0.00001);
        assert_eq!(averaged(&index(&[floor]), &[floor], 0, Vec3::ZERO, Vec3::Z), 1.0);
        let raised = [floor, Object { pos: leg.pos + Vec3::Z * 0.20, ..leg }];
        assert!(averaged(&index(&raised), &raised, 0, vec3(0.052, 0.0, 0.0), Vec3::Z) > close);
    }

    #[test]
    fn cell_candidates_match_exhaustive_sdf_contacts_across_rotations_and_boundaries() {
        let mut objects = vec![Object::new(Kind::Box, vec3(0.0, 0.0, -0.06), vec3(4.0, 4.0, 0.12), Material::Paint)];
        for id in 0..12 {
            let (sin, cos) = (id as f32 * 0.71).sin_cos();
            objects.push(Object {
                rotation: vec2(sin, cos),
                ..Object::new(
                    Kind::Box,
                    vec3((id % 4) as f32 - 1.5, (id / 4) as f32 - 1.0, 0.20),
                    vec3(0.08 + id as f32 * 0.01, 0.16, 0.40),
                    Material::Wood,
                )
            });
        }
        let data = index(&objects);
        let mut exhaustive = vec![0, 0, 0, 1, 1, 1, 8, 8 + objects.len() as u32];
        exhaustive.extend(0..objects.len() as u32);
        for sample in 0..2000 {
            let seed = vec2(sample as f32, 137.0);
            let point = vec3(
                super::super::materials::hash(seed) * 4.0 - 2.0,
                super::super::materials::hash(seed + 37.0) * 4.0 - 2.0,
                super::super::materials::hash(seed + 83.0) * 0.6,
            );
            let normal = if sample % 3 == 0 {
                Vec3::X
            } else if sample % 3 == 1 {
                Vec3::Y
            } else {
                Vec3::Z
            };
            let actual = averaged(&data, &objects, 0, point, normal);
            let expected = averaged(&exhaustive, &objects, 0, point, normal);
            assert!((actual - expected).abs() < 0.00001, "contact index omitted geometry at {point:?}");
        }
    }
}
