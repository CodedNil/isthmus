use super::{Frame, Program, daylight, hash, noise, pigment, segment, wash, wind};
use isthmus::{prelude::*, spirv_std::arch::kill};

pub(super) fn draw(frame: &mut Frame<'_>, root: Vec2, depth: f32, seed: f32) {
    shader!(
        frame
            .upload({
                let size: Vec2 = frame.screen_size;
                let root: Vec2;
                let depth: f32;
                let seed: f32;
                let kind: u32 = (hash(seed + 23.0) * 4.0) as u32;
                let reach: f32 = (0.012 + depth * 0.13) * (0.65 + hash(seed + 31.0) * 0.6);
            })
            .vertex(
                Quad::new(
                    (root - vec2(0.0, reach * 0.4)) * size.y + vec2(size.x * 0.5, 0.0),
                    vec2(2.4, 1.6) * reach * size.y,
                    Vec2::X,
                )
                .expanded(2.0)
            )
            .fragment(|frame, surface| {
                let point = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                let local = (point - root) / reach;
                let aa = 1.0 / (size.y * reach);
                let gust = wind(root, frame.time);
                let mut color = vec3(0.0, 0.0, 0.0);
                let mut alpha = 0.0;
                if hash(seed + 19.0) > 0.68 {
                    let stone = (local - vec2((hash(seed + 27.0) - 0.5) * 0.65, -0.02)) / vec2(0.36, 0.24);
                    let chips = (noise(stone * 9.0 + seed) - 0.5) * 0.06;
                    let edge = (stone.x.abs() - 0.85)
                        .max(stone.y - 0.45)
                        .max(-stone.y - 0.85)
                        .max(stone.x.abs() * 0.8 - stone.y * 0.6 - 0.87)
                        .max(stone.x * 0.9 + stone.y * 0.4 - 0.83);
                    alpha = (edge + chips).smoothstep(aa * 4.0, -aa * 4.0);
                    let facet = (stone.x * 0.35 - stone.y + chips * 3.0).smoothstep(0.05, 0.14);
                    let moss = (-stone.y + wash(stone * 3.0 + seed) * 0.7).smoothstep(0.45, 0.9);
                    color = vec3(0.19, 0.3, 0.28)
                        .lerp(vec3(0.61, 0.59, 0.4), facet * 0.75)
                        .lerp(vec3(0.35, 0.48, 0.27), moss)
                        * alpha;
                }
                for leaf in 0..if kind == 1 { 15 } else { 9 } {
                    let random = hash(seed + leaf as f32 * 8.71);
                    let axis = Vec2::from_angle(-1.57 + (random - 0.5) * 2.8);
                    let offset = local - vec2((hash(seed + leaf as f32 + 41.0) - 0.5) * 0.28, 0.035);
                    let mut q = vec2(offset.dot(axis), offset.dot(axis.perp()));
                    let length = (0.3 + hash(seed + leaf as f32 * 4.13) * 0.7) * if kind >= 2 { 0.65 } else { 1.0 };
                    let t = q.x / length;
                    q.y -= length * t * t * ((random - 0.5) * 0.7 + gust * 0.1);
                    let width = if kind == 0 { 0.007 } else { 0.014 + random * 0.02 };
                    let mut distance = (q.y.abs() - width * (4.0 * t * (1.0 - t)).max(0.0)).max(-q.x).max(q.x - length);
                    if kind == 0 {
                        let pair = ((q.x - q.y.abs() * 0.55) / length * 8.0 - 0.35).round().clamp(0.0, 7.0);
                        let span = length * 0.25 * (1.0 - (pair / 8.0).powi(2));
                        let rib = q.x - (pair + 0.35) / 8.0 * length - q.y.abs() * 0.55;
                        let serration = (q.y.abs() / length * 110.0).sin() * length * 0.005;
                        let wing =
                            rib.abs() - (length * 0.024 + serration) * (1.0 - q.y.abs() / span.max(0.001)).saturate();
                        distance =
                            distance.min(wing.max(q.y.abs() - span).max((0.1 - t) * length).max((t - 0.94) * length));
                    } else if kind >= 2 {
                        let blade = (q - vec2(length * 0.61, 0.0)) / vec2(length * 0.39, length * 0.29);
                        let lobes = if kind == 3 { 1.0 + (blade.y.atan2(blade.x) * 5.0).cos() * 0.16 } else { 1.0 };
                        let edge = (blade.length() - lobes) * length * 0.29;
                        let notch = length * 0.16 - (q - vec2(length * 0.19, 0.0)).length();
                        distance = distance.min(edge.max(notch));
                    }
                    let mask = distance.smoothstep(aa, -aa);
                    let vein = q.y.abs().smoothstep(width * 0.25 + aa, 0.0);
                    let light = random * 0.55 + t.saturate() * 0.25 + vein * 0.12;
                    let blade = vec3(0.045, 0.23, 0.2)
                        .lerp(vec3(0.52, 0.63, 0.29), light)
                        .lerp(vec3(0.67, 0.44, 0.3), if kind == 0 && random > 0.73 { 0.6 } else { 0.0 });
                    color = color * (1.0 - mask) + blade * mask;
                    alpha += mask * (1.0 - alpha);
                }
                if kind >= 2 {
                    for flower in 0..5 {
                        let random = hash(seed + flower as f32 * 9.7);
                        let center =
                            vec2((hash(seed + flower as f32 * 3.1) - 0.5) * 1.05 + gust * 0.045, -0.32 - random * 0.4);
                        let stalk = (segment(local, vec2(center.x * 0.5, 0.015), center) - 0.007).smoothstep(aa, -aa);
                        color = color * (1.0 - stalk) + vec3(0.2, 0.37, 0.22) * stalk;
                        alpha += stalk * (1.0 - alpha);
                        let radius = 0.072 + random * 0.035;
                        let face = (local - center) / radius;
                        for petal in 0..5 {
                            let angle = 1.57 + petal as f32 * 1.256_637 + (random - 0.5) * 0.4;
                            let axis = Vec2::from_angle(angle);
                            let q = vec2(face.dot(axis), face.dot(axis.perp()));
                            let length = if kind == 2 && petal == 0 { 0.59 } else { 0.48 };
                            let ellipse = (q - vec2(0.43, 0.0)) / vec2(length, 0.34);
                            let mask = (ellipse.length() - 1.0).smoothstep(aa / (radius * 0.34), -aa / (radius * 0.34));
                            let shade = (q.y + q.x * 0.3).smoothstep(-0.24, 0.4);
                            let bloom = if kind == 2 {
                                vec3(0.44, 0.36, 0.68).lerp(vec3(0.79, 0.73, 0.91), shade)
                            } else {
                                vec3(0.7, 0.79, 0.76).lerp(vec3(0.98, 0.96, 0.83), shade)
                            };
                            color = color * (1.0 - mask) + bloom * mask;
                            alpha += mask * (1.0 - alpha);
                        }
                        let heart = face.length().smoothstep(0.26 + aa / radius, 0.12);
                        let throat = if kind == 2 { vec3(0.94, 0.91, 0.69) } else { vec3(0.85, 0.68, 0.29) };
                        color = color * (1.0 - heart) + throat * heart;
                        alpha += heart * (1.0 - alpha);
                    }
                }
                if alpha == 0.0 {
                    kill();
                }
                daylight(pigment(color / alpha, point + seed, 1.0 / size.y), surface.pixel / size)
                    .lerp(vec3(0.51, 0.69, 0.61), (1.0 - depth).powi(2) * 0.35)
                    .extend(alpha)
            })
    );
}
