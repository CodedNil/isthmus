use super::*;

#[test]
fn accumulation_survives_running_clock_and_resets_only_for_discontinuities() {
    let noon = 1_782_043_200.0;
    for interleaved in [false, true] {
        let room = home::Room::new(Vec2::ZERO, Vec2::splat(3.0), Material::Paint).downlights("test", &[(0.0, 0.0)]);
        let mut scene = Scene::new_at(&[room], noon, SunSettings::default());
        let mut samples = Accumulation::new(interleaved);
        let revision = scene.lighting_revision;
        let mut previous = 1;
        for frame in 1..=600 {
            scene.update_time(noon + f64::from(frame) * 0.2);
            let (_, mask) = samples.advance(scene.lighting_revision != revision);
            let valid = mask.count_ones();
            assert!(valid >= previous, "ordinary time must not discard valid samples");
            previous = valid;
            if frame >= 8 {
                assert!(samples.converged());
            }
        }
        let mut revision = scene.lighting_revision;
        for brightness in [0.0, 0.5, 1.0] {
            assert!(scene.set_light_state("test", brightness));
            assert_ne!(scene.lighting_revision, revision);
            let (_, mask) = samples.advance(true);
            assert_eq!(mask.count_ones(), 1);
            for _ in 0..8 {
                samples.advance(false);
            }
            assert!(samples.converged());
            revision = scene.lighting_revision;
            scene.set_light_state("test", brightness);
            assert_eq!(revision, scene.lighting_revision, "idempotent controls must not invalidate history");
        }
        scene.update_time(noon + 86_400.0);
        assert!(scene.lighting_revision != revision, "large clock jumps must invalidate old lighting");
    }
}

#[test]
fn emitter_distribution_is_finite_symmetric_and_controllable_across_angles() {
    let fixture = Object {
        emission: 1.0,
        ..Object::new(Kind::Downlight, vec3(0.0, 0.0, 2.0), vec3(0.11, 0.11, 0.032), Material::Paint)
    };
    let source = fixture.pos - Vec3::Z * 0.025;
    for distance in [0.2, 1.0, 2.0, 3.9] {
        let mut previous = f32::INFINITY;
        for degree in (0..90).step_by(5) {
            let angle = (degree as f32).to_radians();
            let delta = vec3(angle.sin(), 0.0, -angle.cos()) * distance;
            let (_, radiance, _) = fixtures::incident(fixture, source + delta, Vec2::ZERO);
            let (_, mirrored, _) = fixtures::incident(fixture, source - delta * vec3(1.0, 1.0, -1.0), Vec2::ZERO);
            assert!(radiance.is_finite() && radiance.min_element() > 0.0);
            assert!(radiance.x <= previous + 0.0001);
            assert!((radiance - mirrored).length() < 0.0001);
            previous = radiance.x;
            let (_, off, _) = fixtures::incident(Object { emission: 0.0, ..fixture }, source + delta, Vec2::ZERO);
            assert_eq!(off, Vec3::ZERO);
        }
    }
}

#[test]
fn emitters_conserve_rated_lumens_and_follow_inverse_square_law() {
    use crate::render::photometry::{GU10_LUMENS, UNITS};
    use core::f32::consts::{PI, TAU};
    let y = vec3(0.2126, 0.7152, 0.0722);
    for (kind, flux) in [(Kind::Downlight, GU10_LUMENS), (Kind::Lamp, 470.0)] {
        let fixture = Object { emission: 1.0, ..Object::new(kind, Vec3::ZERO, Vec3::ONE, Material::Paint) };
        let source = Vec3::Z * if kind == Kind::Downlight { -0.025 } else { 0.005 };
        let mut integrated = 0.0;
        let samples = 4096;
        for sample in 0..samples {
            let cos = 1.0 - 2.0 * (sample as f32 + 0.5) / samples as f32;
            let direction = vec3((1.0 - cos * cos).sqrt(), 0.0, cos);
            integrated += fixtures::incident(fixture, source - direction, Vec2::ZERO).1.dot(y) * UNITS * 4.0 * PI
                / samples as f32;
        }
        assert!((integrated / flux - 1.0).abs() < 0.001, "rated flux lost in angular distribution");
        let reference = fixtures::incident(fixture, source - Vec3::Z, Vec2::ZERO).1;
        for distance in [0.5, 2.0, 4.0, 6.0] {
            let incident = fixtures::incident(fixture, source - Vec3::Z * distance, Vec2::ZERO).1;
            assert!((incident * distance * distance - reference).abs().max_element() < 1e-5);
        }
        if kind == Kind::Downlight {
            let angle = TAU / 18.0; // 20°, half of the nominal FWHM beam.
            let incident = fixtures::incident(fixture, source - vec3(angle.sin(), 0.0, angle.cos()), Vec2::ZERO).1;
            assert!((incident.x / reference.x - 0.5).abs() < 0.001);
        }
    }
}

#[test]
fn camera_meter_tracks_source_energy_without_being_driven_by_empty_background() {
    let room = home::Room::new(Vec2::ZERO, Vec2::splat(3.0), Material::Paint)
        .downlights("test", &[(0.0, 0.0)])
        .window(vec2(0.0, -1.5), 0.0);
    let mut scene = Scene::new_at(&[room], 1_782_043_200.0, SunSettings::default());
    let original = scene.probes.clone();
    let sun = scene.sun;
    let eye = vec3(0.0, 0.0, 1.5);
    let forward = Vec3::Y;
    let right = Vec3::X * 0.6;
    let up = Vec3::Z * 0.4;
    let mut previous = f32::INFINITY;
    for factor in [0.25, 1.0, 4.0, 16.0] {
        scene.sun.radiance = sun.radiance * factor;
        scene.sun.sky = sun.sky * factor;
        for (probe, base) in scene.probes.iter_mut().zip(&original) {
            probe.light.constant = base.light.constant * factor;
            probe.light.x = base.light.x * factor;
            probe.light.y = base.light.y * factor;
            probe.light.z = base.light.z * factor;
        }
        let (exposure, white_balance) = scene.camera_settings(eye, forward, right, up);
        assert!(white_balance.is_finite());
        assert!(white_balance.cmpge(Vec3::splat(0.65)).all() && white_balance.cmple(Vec3::splat(2.2)).all());
        assert!(exposure.is_finite() && exposure > 0.0 && exposure <= previous);
        previous = exposure;
    }
    let outside = scene.camera_exposure(vec3(20.0, 20.0, 4.0), Vec3::Z, right, up);
    assert!(outside.is_finite() && outside > 0.0);
}
