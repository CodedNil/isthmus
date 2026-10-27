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
    let lit = sample(&scene);
    scene.set_light_state("light.test", 0.0);
    let dark = sample(&scene);
    assert!(lit.x > dark.x + 0.001, "ceiling should receive bounced light: {lit:?}, {dark:?}");
    assert!(lit.x > lit.z, "warm emitters should tint the bounce");
    for brightness in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
        scene.set_light_state("light.test", brightness);
        let expected = dark + (lit - dark) * brightness;
        assert!((sample(&scene) - expected).length() < 0.00001, "light transport must be linear and have no feedback");
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
    let sample = |scene: &Scene| scene.grid.sample(&scene.probes, vec3(1.6, 0.0, 1.0), -Vec3::X);
    let lit = sample(&scene);
    scene.set_light_state("light.left", 0.0);
    assert!((lit - sample(&scene)).length() < 0.0001, "GI should respect the partition");
}

#[test]
fn budgeted_updates_converge_to_a_full_relight() {
    let room = Room::new(Vec2::ZERO, Vec2::splat(3.0), Material::Carpet).downlights("light.test", &[(0.0, 0.0)]);
    let scene = Scene::new_at(&[room], MIDNIGHT, SunSettings::default());
    let (_, mut probes, mut cache) =
        bake(&scene.objects, &scene.nodes, &scene.lights, scene.nodes[0].min, scene.nodes[0].max, scene.sun);
    let noon = Sun::at_utc(MIDNIGHT + 12.0 * 3600.0, SunSettings::default());
    cache.begin_update();
    assert_eq!(cache.remaining, cache.active.len() * (BOUNCES + 1));
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
                    &scene.objects,
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
