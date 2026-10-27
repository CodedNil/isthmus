use super::*;
use crate::render::WALL_HEIGHT;

#[test]
fn rejected_lights_have_no_visible_emitter_samples_at_receiver_bounds() {
    let scene = Scene::new_at(&home::rooms(), 1_782_043_200.0, SunSettings::default());
    let mut rejected = 0;
    for node in scene.nodes.iter().filter(|node| node.object != BRANCH) {
        let mask = scene.light_masks[node.object as usize];
        for (slot, &index) in scene.lights.iter().take(32).enumerate() {
            if mask & (1 << slot) != 0 {
                continue;
            }
            rejected += 1;
            let light = scene.objects[index as usize];
            for corner in 0..8 {
                let point = vec3(
                    if corner & 1 == 0 { node.min.x - 0.003 } else { node.max.x + 0.003 },
                    if corner & 2 == 0 { node.min.y - 0.003 } else { node.max.y + 0.003 },
                    if corner & 4 == 0 { node.min.z - 0.003 } else { node.max.z + 0.003 },
                );
                for sample in 0..4 {
                    let emitter = subpixel(sample) * if light.kind == Kind::Downlight { 0.055 } else { 0.035 };
                    let (direction, radiance, distance) = fixtures::incident(light, point, emitter);
                    if radiance.max_element() <= 0.0 {
                        continue;
                    }
                    let visible =
                        cast::<true, false>(point, direction, &scene.nodes, &scene.objects, 0.0, distance - 0.002).0;
                    assert_eq!(
                        visible, 0.0,
                        "object {}, light slot {slot}, corner {corner}, sample {sample}",
                        node.object
                    );
                }
            }
        }
    }
    assert!(rejected > 0, "fixture masks must reject some lights");
}

#[test]
fn acceleration_agrees_with_exhaustive_intersection_and_normals_stay_finite() {
    let scene = Scene::new_at(&home::rooms(), 1_782_043_200.0, SunSettings::default());
    for origin in [vec3(0.0, 0.0, 4.0), vec3(-4.0, 0.0, 1.5), vec3(4.0, 0.0, 1.5)] {
        for ray in 0..64 {
            let z = 1.0 - 2.0 * (ray as f32 + 0.5) / 64.0;
            let angle = ray as f32 * 2.399_963;
            let radius = (1.0 - z * z).sqrt();
            let direction = vec3(radius * angle.cos(), radius * angle.sin(), z);
            let accelerated = cast::<false, false>(origin, direction, &scene.nodes, &scene.objects, 0.0, FAR);
            let bounds = trace::Ray::new(origin, direction);
            let mut exhaustive = FAR;
            for item in &scene.objects {
                let extent = item.extent();
                let (sin, cos) = (item.rotation.x.abs(), item.rotation.y.abs());
                let extent = vec3(cos * extent.x + sin * extent.y, sin * extent.x + cos * extent.y, extent.z);
                let (enter, exit) = bounds.bounds(item.pos - extent, item.pos + extent);
                if enter > exit || enter >= exhaustive {
                    continue;
                }
                let distance = if matches!(item.kind, Kind::Box | Kind::Ceiling) && item.rotation == Vec2::Y {
                    if enter > 0.0 { enter } else { exit }
                } else {
                    intersect(*item, origin, direction, enter, exit.min(exhaustive), 0.0)
                };
                exhaustive = exhaustive.min(distance);
            }
            assert!((accelerated.0 - exhaustive).abs() < 0.002, "BVH missed a nearer surface");
            if accelerated.0 < FAR {
                let item = scene.objects[accelerated.1 as usize];
                let normal =
                    object_normal(item, unrotate_z(origin + direction * accelerated.0 - item.pos, item.rotation));
                assert!(normal.is_finite() && (normal.length() - 1.0).abs() < 0.01);
            }
        }
    }
}

#[test]
fn enclosures_block_light_except_through_openings_and_have_only_indoor_probes() {
    for width in [2.5, 4.0, 6.0] {
        let half = width * 0.5;
        for (position, rotation, outward) in [
            (vec2(0.0, -half), 0.0, Vec3::Y),
            (vec2(half, 0.0), -90.0, Vec3::X),
            (vec2(0.0, half), 180.0, -Vec3::Y),
            (vec2(-half, 0.0), 90.0, -Vec3::X),
        ] {
            let room = home::Room::new(Vec2::ZERO, Vec2::splat(width), Material::Paint).window(position, rotation);
            let scene = Scene::new_at(&[room], 1_782_043_200.0, SunSettings::default());
            let start = outward.cross(Vec3::Z) * 0.25 + Vec3::Z * 1.5;
            let visible = |p, d| cast::<true, false>(p, d, &scene.nodes, &scene.objects, 0.0, FAR).0;
            assert!(visible(start, outward) > 0.5, "window must transmit light");
            assert!(visible(start - Vec3::Z, outward) < 0.5, "wall below window must be solid");
            assert!(visible(start, Vec3::Z) < 0.5, "roof must block light");
            let cutaway = cast::<false, true>(vec3(0.2, 0.0, 4.0), -Vec3::Z, &scene.nodes, &scene.objects, 0.0, FAR).0;
            assert!(cutaway > 3.9, "cutaway must affect only camera visibility");
            for probe in &scene.probes {
                if probe.positive.x < 0.0 {
                    assert!(probe.light.constant == Vec3::ZERO, "disabled probes must not carry light");
                } else {
                    assert!(probe.position.z > 0.0 && probe.position.z < WALL_HEIGHT);
                    let (distance, ceiling) = architectural_cast(probe.position, Vec3::Z, &scene.nodes, &scene.objects);
                    assert!(distance < FAR && scene.objects[ceiling as usize].kind == Kind::Ceiling);
                }
            }
        }
    }
}
