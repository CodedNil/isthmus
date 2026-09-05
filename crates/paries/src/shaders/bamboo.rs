use super::{Fragment, Frame, fbm, hash, noise, tapered_segment};
use core::f32::consts::{PI, TAU};
use isthmus::{
    Blend, Float as _, Quad,
    glam::{Vec2, Vec3, Vec4, vec2, vec3},
    shader,
};

pub struct Bamboo;

impl Bamboo {
    pub fn show(&mut self, frame: &mut Frame<'_>) {
        let size = frame.screen_size;
        frame.paint(
            Quad::new(size * 0.5, size, Vec2::X),
            shader!(Blend::Replace, |fragment: Fragment, size: Vec2| {
                wallpaper(fragment.pixel, size, fragment.time)
            }),
        );
    }
}

fn over(base: Vec3, color: Vec3, alpha: f32) -> Vec3 {
    base.lerp(color, alpha.saturate())
}

fn leaf(point: Vec2, center: Vec2, axis: Vec2, size: Vec2) -> f32 {
    let q = point - center;
    let x = q.dot(axis) / size.x;
    let y = q.dot(axis.perp()) / size.y;
    let taper = (1.0 - x.abs()).max(0.0);
    (y.abs() - taper * taper.sqrt()).smoothstep(0.08, -0.08) * taper.smoothstep(0.0, 0.04)
}

fn foliage(mut color: Vec3, point: Vec2, root: Vec2, direction: Vec2, scale: f32, light: f32) -> Vec3 {
    let axis = direction.normalize();
    let side = axis.perp();
    let tip = root + axis * 88.0 * scale;
    color = over(color, vec3(0.018, 0.10, 0.03), tapered_segment(point, root, tip, 2.0 * scale, 0.4 * scale).fill());
    for index in 0..3 {
        let (along, handedness, length, width) = if index == 0 {
            (0.43, -0.48, 29.0, 6.5)
        } else if index == 1 {
            (0.66, 0.46, 34.0, 7.0)
        } else {
            (0.86, 0.0, 33.0, 6.2)
        };
        let leaf_axis = (axis + side * handedness).normalize();
        let center = root.lerp(tip, along) + leaf_axis * length * scale * 0.55;
        let mask = leaf(point, center, leaf_axis, vec2(length, width) * scale);
        let vein = (point - center).dot(leaf_axis.perp()).abs().smoothstep(1.2 * scale, 0.0);
        color = over(color, vec3(0.025, 0.16, 0.045).lerp(vec3(0.22, 0.43, 0.10), light + vein * 0.12), mask);
    }
    color
}

fn culm(mut color: Vec3, point: Vec2, root: Vec2, top: Vec2, radius: f32, spacing: f32, light: f32) -> Vec3 {
    let line = top - root;
    let length = line.length();
    let axis = line / length;
    let local = point - root;
    let along = local.dot(axis);
    let progress = (along / length).clamp(0.0, 1.0);
    let width = radius.lerp(radius * 0.56, progress);
    let cell = (along / spacing).fract();
    let node = (cell.min(1.0 - cell) / 0.055).smoothstep(1.0, 0.0);
    let distance = local.dot(axis.perp()).abs() - width * (0.97 + node * 0.12);
    let mask = distance.max((-along).max(along - length)).smoothstep(1.1, -1.1);
    let round = (0.5 - local.dot(axis.perp()) / width * 0.42).saturate();
    let fibre = (local.dot(axis.perp()) / width * 4.0 + along * 0.004).sin() * 0.04;
    let stem = vec3(0.035, 0.16, 0.025).lerp(vec3(0.48, 0.56, 0.10), (round + fibre + light).saturate());
    color = over(color, stem, mask);
    over(color, vec3(0.02, 0.08, 0.015), node * mask * 0.58)
}

fn bamboo_layer(mut color: Vec3, point: Vec2, size: Vec2, time: f32, depth: f32) -> Vec3 {
    for index in 0..3 {
        let seed = index as f32 + depth * 13.7;
        let x = size.x * (0.12 + 0.38 * index as f32) + (hash(seed) - 0.5) * size.x * 0.12;
        let sway = (time * (0.10 + depth * 0.04) + seed).sin() * (12.0 + depth * 8.0);
        let root = vec2(x, size.y + 30.0);
        let top = vec2(x + sway, -80.0);
        let radius = (12.0 + hash(seed + 2.0) * 10.0) * (0.65 + depth * 0.55);
        let light = hash(seed + 9.0) * 0.22;
        color = culm(color, point, root, top, radius, 72.0 + seed.fract() * 24.0, light);
        let axis = (top - root).normalize();
        for branch in 0..4 {
            let phase = branch as f32 * 1.7 + seed;
            let origin = root.lerp(top, 0.24 + branch as f32 * 0.18);
            let direction = (axis + axis.perp() * phase.sin() * 0.72).normalize();
            color = foliage(color, point, origin, direction, 0.72 + depth * 0.3, light);
        }
    }
    color
}

fn wallpaper(point: Vec2, size: Vec2, time: f32) -> Vec4 {
    let uv = point / size.max(Vec2::ONE);
    let mist = fbm(uv * vec2(2.2, 1.4) + vec2(time * 0.008, 4.0));
    let glow = (1.0 - uv.distance(vec2(0.72, 0.22))).saturate();
    let mut color = vec3(0.006, 0.018, 0.012).lerp(vec3(0.035, 0.095, 0.055), (1.0 - uv.y) * 0.55 + mist * 0.18)
        + vec3(0.16, 0.19, 0.09) * glow.powf(3.0) * 0.35;
    color = bamboo_layer(color, point, size, time, 0.25);
    color = bamboo_layer(color, point + vec2(noise(uv * 3.0 + time * 0.02), 0.0) * 4.0, size, time, 0.85);
    let grain = hash((point + time * 0.2).floor().dot(vec2(127.1, 311.7))) - 0.5;
    let vignette = (uv - 0.5).length_squared().smoothstep(0.16, 0.62);
    let breeze = (uv.x * TAU + time * 0.08).sin() * (uv.y * PI).sin() * 0.008;
    (color + grain * 0.012 + breeze).lerp(Vec3::ZERO, vignette * 0.42).extend(1.0)
}
