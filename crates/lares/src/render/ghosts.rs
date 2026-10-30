//! World-space translucent SDF estimates, depth-tested against the scene height.
use crate::presence::Ghost;
use isthmus::prelude::*;

fn distance(p: Vec3) -> f32 {
    let head = (p - Vec3::Z * 1.24).length() - 0.24;
    let radius = 0.32 - (p.z - 0.18).clamp_num(0.0, 0.95) * 0.105;
    let body = ((p.xy().length() - radius) / 1.02).max_num(0.16 - p.z).max_num(p.z - 1.24);
    let ring = vec2(p.xy().length() - 0.39, p.z - 0.035).length() - 0.012;
    head.min_num(body).min_num(ring)
}

fn normal(point: Vec3) -> Vec3 {
    let corner_a = vec3(1.0, -1.0, -1.0);
    let corner_b = vec3(-1.0, -1.0, 1.0);
    let corner_c = vec3(-1.0, 1.0, -1.0);
    let corner_d = Vec3::ONE;
    (corner_a * distance(point + corner_a * 0.002)
        + corner_b * distance(point + corner_b * 0.002)
        + corner_c * distance(point + corner_c * 0.002)
        + corner_d * distance(point + corner_d * 0.002))
    .normalize()
}

/// Only rays through a small bounding sphere march the ghost; no house rays.
pub(super) fn composite(
    mut color: Vec3,
    ghosts: &[Ghost],
    eye: Vec3,
    ray: Vec3,
    height: f32,
    pixel_angle: f32,
) -> Vec3 {
    let limit = ((height - eye.z) / ray.z).max_num(0.0);
    let mut index = 0;
    while index < ghosts.len() {
        let ghost = ghosts[index];
        index += 1;
        let origin = eye - ghost.position;
        let center = origin - Vec3::Z * 0.75;
        let b = center.dot(ray);
        let discriminant = b * b - center.length_squared() + 0.9 * 0.9;
        if discriminant <= 0.0 {
            continue;
        }
        let root = discriminant.sqrt();
        let mut t = (-b - root).max_num(0.0);
        let end = (-b + root).min_num(limit + 0.01);
        let mut step = 0;
        while t < end && step < 64 {
            let p = origin + ray * t;
            let d = distance(p).abs();
            let epsilon = (t * pixel_angle * 0.35).max_num(0.002);
            if d <= epsilon {
                let rim = (1.0 - normal(p).dot(-ray).abs()).clamp_num(0.0, 1.0);
                let ring = if p.z < 0.07 { 0.32 } else { 0.0 };
                let alpha = ((0.025 + rim * rim * 0.70).max_num(ring) * ghost.opacity).min_num(0.8);
                color = color.lerp(vec3(0.64, 0.85, 0.9), alpha);
                break;
            }
            t += d.max_num(epsilon * 0.5);
            step += 1;
        }
    }
    color
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solid_surfaces_occlude_ghosts_and_empty_input_preserves_the_scene() {
        let color = vec3(0.1, 0.2, 0.3);
        let ghost = Ghost { position: Vec3::ZERO, opacity: 1.0 };
        let eye = vec3(0.18, 0.0, 5.0);
        assert_eq!(composite(color, &[ghost], eye, -Vec3::Z, 2.0, 0.001), color);
        assert!(composite(color, &[ghost], eye, -Vec3::Z, 0.0, 0.001).distance(color) > 0.005);
        assert_eq!(composite(color, &[], eye, -Vec3::Z, 0.0, 0.001), color);
    }
}
