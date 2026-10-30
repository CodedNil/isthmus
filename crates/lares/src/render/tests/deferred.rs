use super::*;
use crate::{camera::Camera, render::shadows::ShadowAtlas};

#[test]
fn half_payloads_survive_half_precision_storage_exactly() {
    for value in 0..32768 {
        let encoded = half_payload(value);
        let stored = F16x2::from_vec2(vec2(encoded, 0.0)).to_vec2().x;
        assert_eq!(read_half_payload(stored), value);
        assert!(stored.is_normal());
    }
    for direction in [Vec3::X, -Vec3::Y, Vec3::Z, -Vec3::Z, vec3(0.3, -0.7, 0.4).normalize()] {
        for height in [-0.12, 0.0, 0.34, 1.1, 2.4] {
            let record = encode_visibility(4238, SUN_VISIBLE | REFLECTED | GLAZED, height, direction, 0.73);
            let word = read_half_payload(record.x);
            let extra = read_half_payload(record.z);
            let decoded_height = (read_half_payload(record.y) | ((extra & 3) << 15)) as f32 * (3.2 / 131_071.0) - 0.2;
            assert_eq!(word & VISIBILITY_OBJECTS, 4238);
            assert_eq!(word & (SUN_VISIBLE | REFLECTED), SUN_VISIBLE | REFLECTED);
            assert_ne!(extra & 4, 0);
            assert!((height - decoded_height).abs() < 0.000_025);
            let normal = decode_normal((read_half_payload(record.w) & 16383) | ((extra >> 11) << 14));
            assert!(normal.dot(direction) > 0.9999);
            assert!((((extra >> 3) & 255) as f32 / 255.0 - 0.73).abs() < 0.002);
        }
    }
}

#[test]
fn deferred_pixels_match_the_direct_reference() {
    let room = home::Room::new(Vec2::ZERO, vec2(3.0, 3.0), Material::Floorboards)
        .furniture(Kind::Chair, (0.3, 0.0, 0.0), (0.72, 0.72, 1.3), 115.0)
        .furniture(Kind::Table, (-0.8, -0.5, 0.0), (0.6, 0.8, 0.7), 0.0)
        .downlights("test", &[(0.0, 0.0)]);
    let scene = Scene::new_at(&[room], 1_782_043_200.0, SunSettings::default());
    let atlas = ShadowAtlas::bake_cpu(&scene, 32);
    let size = vec2(48.0, 32.0);
    let camera = Camera { distance: 6.0, ..Camera::default() };
    let (eye, forward, right, up) = camera.rays(size.x / size.y);
    let globals = Globals {
        eye,
        forward,
        right,
        up,
        pixel_angle: camera.fov / size.y,
        exposure: 1.0,
        white_balance: Vec3::ONE,
        sun: scene.sun,
        sample_index: 0,
        interleaved: false,
        parity: 0,
    };
    let mut error = 0.0;
    let mut maximum: f32 = 0.0;
    for y in 0..32 {
        for x in 0..48 {
            let pixel = vec2(x as f32 + 0.5, y as f32 + 0.5);
            let record = visibility_pixel(globals, &scene.nodes, &scene.objects, &scene.contacts, pixel, size, 1.0);
            let material =
                material_visibility(globals, &scene.objects, scene.grid, &scene.probes, pixel, size, 1.0, record, 0);
            let actual = shade_visibility::<0>(
                globals,
                &[],
                &scene.objects,
                &scene.lights,
                &scene.light_masks,
                &[],
                scene.grid,
                &[],
                pixel,
                size,
                1.0,
                0.0,
                &atlas.image,
                atlas.info,
                record,
                0,
                material,
            );
            let expected = pixel_single(
                globals,
                &scene.nodes,
                &scene.objects,
                &scene.lights,
                &scene.light_masks,
                &scene.contacts,
                scene.grid,
                &scene.probes,
                pixel,
                size,
                1.0,
                0.0,
                &atlas.image,
                atlas.info,
                0,
            );
            let difference = (actual.truncate() - expected.truncate()).abs();
            error += difference.element_sum() / 3.0;
            maximum = maximum.max(difference.max_element());
        }
    }
    assert!(error / (48.0 * 32.0) < 0.005, "mean display error {}", error / (48.0 * 32.0));
    assert!(maximum < 0.15, "maximum display error {maximum}");
}

#[test]
fn packed_materials_preserve_hdr_energy_and_material_identity() {
    for material in
        [Material::Wood, Material::Floorboards, Material::Metal, Material::GlazedTile, Material::BathroomTile]
    {
        for scale in [0.0, 1.0e-8, 0.001, 1.0, 10000.0] {
            let tint = vec3(0.13, 0.62, 0.89);
            let light = vec3(0.23, 0.77, 1.0) * scale;
            let (decoded, illumination) = decode_material(encode_material(Hit::new(0.0, || (material, tint)), light));
            assert!(decoded.material == material);
            assert!((decoded.tint - tint).abs().max_element() < 0.002);
            assert!((illumination - light).abs().max_element() <= scale * 0.004 + 1.0e-18);
        }
    }
}
