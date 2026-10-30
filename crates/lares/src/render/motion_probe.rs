//! Temporary sensor-independent motion reference, composited after scene history.
use isthmus::prelude::*;

/// Three-metre line, eight-second period: 0.75 m/s, with instantaneous reversals.
pub(super) fn position(seconds: f32) -> Vec3 {
    let phase = (seconds * 0.125).fract();
    let triangle = 1.0 - (phase * 2.0 - 1.0).abs();
    vec3(-4.5 + triangle * 3.0, 0.85, 0.35)
}

pub(super) fn composite(color: Vec3, eye: Vec3, ray: Vec3, height: f32, seconds: f32) -> Vec3 {
    let origin = eye - position(seconds);
    let along = origin.dot(ray);
    let discriminant = along * along - origin.length_squared() + 0.18 * 0.18;
    if discriminant <= 0.0 {
        return color;
    }
    let distance = -along - discriminant.sqrt();
    let scene_distance = ((height - eye.z) / ray.z).max_num(0.0);
    if distance <= 0.0 || distance > scene_distance + 0.01 {
        return color;
    }
    let normal = (origin + ray * distance).normalize();
    let light = 0.45 + 0.55 * normal.dot(vec3(-0.4, -0.3, 0.866)).max_num(0.0);
    vec3(1.0, 0.025, 0.2) * light
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_wave_has_constant_speed_and_exact_turnarounds() {
        assert_eq!(position(0.0), vec3(-4.5, 0.85, 0.35));
        assert_eq!(position(4.0), vec3(-1.5, 0.85, 0.35));
        assert_eq!(position(8.0), position(0.0));
        for step in 0..32 {
            let start = step as f32 * 0.25;
            let delta = position(start + 0.25) - position(start);
            assert!((delta.length() - 0.1875).abs() < 0.00001);
            assert_eq!(delta.y, 0.0);
            assert_eq!(delta.z, 0.0);
        }
    }

    #[test]
    fn ball_obeys_the_same_scene_depth_as_ghosts() {
        let color = Vec3::splat(0.1);
        let eye = position(0.0) + Vec3::Z * 5.0;
        assert_eq!(composite(color, eye, -Vec3::Z, 1.0, 0.0), color);
        assert_ne!(composite(color, eye, -Vec3::Z, 0.0, 0.0), color);
    }
}
