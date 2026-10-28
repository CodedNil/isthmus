use super::*;
use crate::{
    home::Room,
    render::{Scene, SunSettings},
};
use isthmus::glam::vec2;
const MIDNIGHT: f64 = 1_782_000_000.0;

fn room(position: Vec2) -> Room {
    Room::new(position, Vec2::splat(3.0), Material::Carpet)
}

#[test]
fn fixtures_bounce_onto_ceiling_and_dimming_does_not_accumulate_energy() {
    let room = room(Vec2::ZERO).downlights("light.test", &[(0.0, 0.0)]);
    let mut scene = Scene::new_at(&[room], MIDNIGHT, SunSettings::default());
    let sample = |scene: &Scene| scene.grid.sample(&scene.probes, vec3(0.0, 0.0, 2.25), -Vec3::Z);
    let surface = scene
        .probes
        .iter()
        .enumerate()
        .filter(|(_, probe)| probe.normal.length_squared() > 0.5)
        .map(|(index, probe)| (index, probe.light.constant))
        .collect::<Vec<_>>();
    assert!(surface.iter().any(|(_, light)| light.max_element() > 0.0), "surface receivers must receive bounced light");
    let lit = sample(&scene);
    scene.set_light_state("light.test", 0.0);
    let dark = sample(&scene);
    let surface_dark: Vec<_> = surface.iter().map(|(index, _)| scene.probes[*index].light.constant).collect();
    assert!(lit.x > dark.x + 0.001, "ceiling should receive bounced light: {lit:?}, {dark:?}");
    assert!(lit.x > lit.z, "warm emitters should tint the bounce");
    for brightness in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
        scene.set_light_state("light.test", brightness);
        let expected = dark + (lit - dark) * brightness;
        assert!((sample(&scene) - expected).length() < 0.00001, "light transport must be linear and have no feedback");
        for ((index, lit), dark) in surface.iter().zip(&surface_dark) {
            let expected = *dark + (*lit - *dark) * brightness;
            assert!(
                (scene.probes[*index].light.constant - expected).length() < 0.00001,
                "surface radiosity must also remain linear"
            );
        }
    }
}

#[test]
fn source_colour_changes_both_bounces_without_geometry_rebuilds() {
    let room = room(Vec2::ZERO).downlights("light.test", &[(0.0, 0.0)]);
    let mut scene = Scene::new_at(&[room], MIDNIGHT, SunSettings::default());
    let sample = |scene: &Scene| scene.grid.sample(&scene.probes, vec3(0.0, 0.0, 2.25), -Vec3::Z);
    scene.set_light_state("light.test", 0.0);
    let dark = sample(&scene);
    let mut bases = [Vec3::ZERO; 3];
    for (index, color) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        scene.set_light_color("light.test", color);
        scene.set_light_state("light.test", 1.0);
        bases[index] = sample(&scene) - dark;
    }
    for color in [Vec3::ZERO, Vec3::ONE, vec3(0.2, 0.7, 0.4), vec3(0.9, 0.1, 0.5)] {
        scene.set_light_color("light.test", color);
        for brightness in [0.0, 0.2, 0.5, 1.0] {
            scene.set_light_state("light.test", brightness);
            let expected = dark + (bases[0] * color.x + bases[1] * color.y + bases[2] * color.z) * brightness;
            assert!((sample(&scene) - expected).length() < 0.00001);
        }
    }
    let revision = scene.lighting_revision;
    assert!(!scene.set_light_color("missing", Vec3::Y));
    assert_eq!(revision, scene.lighting_revision);
}

#[test]
fn bounced_fixture_light_does_not_cross_a_closed_room_wall() {
    let left = room(vec2(-1.6, 0.0)).downlights("light.left", &[(0.0, 0.0)]);
    let right = room(vec2(1.6, 0.0));
    let mut scene = Scene::new_at(&[left, right], MIDNIGHT, SunSettings::default());
    // Include floor/wall and ceiling/wall junctions, where relocated volume
    // probes and projected surface footprints used to cross partitions.
    let receivers = [
        (vec3(1.6, 0.0, 1.0), -Vec3::X),
        (vec3(0.103, 0.0, 0.003), Vec3::Z),
        (vec3(0.103, 1.49, 0.003), -Vec3::Y),
        (vec3(0.103, 0.0, 2.397), -Vec3::Z),
    ];
    let sample = |scene: &Scene| receivers.map(|(point, normal)| scene.grid.sample(&scene.probes, point, normal));
    let lit = sample(&scene);
    let surfaces: Vec<_> = scene
        .probes
        .iter()
        .enumerate()
        .filter(|(_, probe)| probe.normal != Vec3::ZERO && probe.position.x > 0.101)
        .map(|(index, probe)| (index, probe.light.constant))
        .collect();
    scene.set_light_state("light.left", 0.0);
    for (lit, dark) in lit.into_iter().zip(sample(&scene)) {
        assert!((lit - dark).length() < 0.0001, "GI should respect the partition: {:?}", lit - dark);
    }
    for (index, lit) in surfaces {
        assert!(
            (lit - scene.probes[index].light.constant).length() < 0.0001,
            "surface filtering must not transfer light through a partition"
        );
    }
}

#[test]
fn budgeted_updates_converge_to_a_full_relight() {
    let room = Room::new(Vec2::ZERO, Vec2::splat(3.0), Material::Carpet).downlights("light.test", &[(0.0, 0.0)]);
    let scene = Scene::new_at(&[room], MIDNIGHT, SunSettings::default());
    let (_, mut probes, mut cache) = bake(
        &mut scene.objects.clone(),
        &scene.nodes,
        &scene.lights,
        scene.nodes[0].min,
        scene.nodes[0].max,
        scene.sun,
    );
    let noon = Sun::at_utc(MIDNIGHT + 12.0 * 3600.0, SunSettings::default());
    cache.begin_update();
    assert!(cache.updating());
    assert!(cache.active.len() < cache.direct.len(), "exterior/solid sites must be excluded from transport work");
    let old = cache.remaining;
    cache.step(noon, &scene.objects, &scene.nodes, &mut probes);
    assert!(old - cache.remaining <= UPDATE_BUDGET);
    while cache.updating() {
        cache.step(noon, &scene.objects, &scene.nodes, &mut probes);
    }
    let incremental = probes.clone();
    cache.relight(noon, &scene.objects, &scene.nodes, &mut probes);
    for (a, b) in incremental.iter().zip(&probes) {
        assert!((a.light.constant - b.light.constant).length() < 1.0e-5);
        assert!((a.light.x - b.light.x).length() < 1.0e-5);
    }
}

#[test]
fn directional_reconstruction_stays_nonnegative_without_changing_mean_energy() {
    let mut field = Irradiance::default();
    // A few intense, differently coloured incoming directions are the hardest
    // case for low-order SH: the old projection produced negative RGB lobes.
    for direction in [Vec3::X, Vec3::Y, Vec3::Z, vec3(-1.0, 0.2, 0.3).normalize()] {
        field.project(direction, vec3(10.0, 1.0, 0.2));
    }
    let mut average = Vec3::ZERO;
    for sample in 0..2048 {
        let z = 1.0 - 2.0 * (sample as f32 + 0.5) / 2048.0;
        let turn = sample as f32 * 2.399_963;
        let radius = (1.0 - z * z).sqrt();
        let direction = vec3(radius * turn.cos(), radius * turn.sin(), z);
        let radiance = field.constant + field.x * direction.x + field.y * direction.y + field.z * direction.z;
        assert!(radiance.min_element() >= -1e-6);
        average += radiance / 2048.0;
    }
    assert!((average - field.constant).abs().max_element() < 0.0001);
}

#[test]
fn exterior_sky_and_ground_enter_only_through_apertures() {
    for width in [2.0, 3.0, 4.0] {
        for rotation in [0.0, 90.0, 180.0, 270.0] {
            let position = match rotation as i32 {
                0 => vec2(0.0, width * 0.5),
                90 => vec2(width * 0.5, 0.0),
                180 => vec2(0.0, -width * 0.5),
                _ => vec2(-width * 0.5, 0.0),
            };
            for open in [false, true] {
                let mut room = Room::new(Vec2::ZERO, Vec2::splat(width), Material::Paint);
                if open {
                    room = room.window(position, rotation);
                }
                let scene = Scene::new_at(&[room], MIDNIGHT + 43200.0, SunSettings::default());
                let (_, _, cache) = bake(
                    &mut scene.objects.clone(),
                    &scene.nodes,
                    &scene.lights,
                    scene.nodes[0].min,
                    scene.nodes[0].max,
                    scene.sun,
                );
                let sky: f32 = cache.sky.iter().map(|light| light.constant.element_sum()).sum();
                let ground: f32 = cache.ground.iter().map(|light| light.constant.element_sum()).sum();
                if open {
                    assert!(sky > 0.0 && ground > 0.0, "both exterior hemispheres should reach an open window");
                } else {
                    assert_eq!((sky, ground), (0.0, 0.0), "opaque structure must block exterior light");
                }
            }
        }
    }
}

#[test]
fn surface_gather_preserves_constant_environment_across_receiver_orientations() {
    for normal in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z, vec3(1.0, 2.0, 3.0).normalize()] {
        let mut result = Irradiance::default();
        for sample in 0..RAYS {
            let z = 1.0 - 2.0 * (sample as f32 + 0.5) / RAYS as f32;
            let angle = sample as f32 * 2.399_963;
            let radius = (1.0 - z * z).sqrt();
            let direction = vec3(radius * angle.cos(), radius * angle.sin(), z);
            result.project_receiver(direction, Vec3::ONE, normal);
        }
        assert!(
            (result.constant - Vec3::ONE).abs().max_element() < 0.015,
            "cosine gather must preserve unit radiance: {:?}",
            result.constant
        );
        for (actual, expected) in [(result.x, normal.x), (result.y, normal.y), (result.z, normal.z)] {
            assert!(
                (actual - Vec3::splat(expected)).abs().max_element() < 0.02,
                "surface directional derivative must match the cosine integral"
            );
        }
    }
}

#[test]
fn surface_normal_response_matches_directional_hemisphere_integration() {
    let light = vec3(0.8, 0.0, 0.6);
    let normals = [vec3(0.15, 0.0, 1.0).normalize(), vec3(-0.15, 0.0, 1.0).normalize()];
    let mut field = Irradiance::default();
    let mut reference = [Vec3::ZERO; 2];
    for sample in 0..RAYS {
        let z = 1.0 - 2.0 * (sample as f32 + 0.5) / RAYS as f32;
        let angle = sample as f32 * 2.399_963;
        let radius = (1.0 - z * z).sqrt();
        let direction = vec3(radius * angle.cos(), radius * angle.sin(), z);
        let radiance = Vec3::splat(direction.dot(light).max(0.0).powi(8));
        field.project_receiver(direction, radiance, Vec3::Z);
        for (value, normal) in reference.iter_mut().zip(normals) {
            *value += radiance * (4.0 * normal.dot(direction).max(0.0) / RAYS as f32);
        }
    }
    assert!(reference[0].x > reference[1].x * 1.2);
    for (normal, expected) in normals.into_iter().zip(reference) {
        let delta = normal - Vec3::Z;
        let actual = field.constant + field.x * delta.x + field.y * delta.y + field.z * delta.z;
        assert!(
            (actual - expected).length() < expected.length() * 0.02,
            "bumped cached irradiance must agree with directional integration: {actual:?} vs {expected:?}"
        );
    }
}
