use super::{
    Frame, Program, atmosphere, daylight, hash,
    landscape::{river, water_mask},
    noise, pigment, undergrowth, wash,
};
use isthmus::{
    Float as _, Quad,
    glam::{Vec2, vec2, vec3},
    shader,
    spirv_std::arch::kill,
};

pub(super) fn rocks(frame: &mut Frame<'_>, root: Vec2, depth: f32, seed: f32) {
    for index in 0..3 {
        shader!(
            frame
                .upload({
                    let size: Vec2 = frame.screen_size;
                    let depth: f32;
                    let seed: f32 = seed + index as f32 * 17.0;
                    let quad: Quad = Quad::new(
                        (root
                            + vec2(
                                (index as f32 - 1.0) * (0.035 + depth * 0.055),
                                -0.03 - depth * 0.035 + index as f32 * 0.009 + hash(seed + index as f32) * 0.015,
                            ))
                            * frame.screen_size.y
                            + vec2(frame.screen_size.x * 0.5, 0.0),
                        vec2(0.11 + depth * 0.2, 0.08 + depth * 0.12)
                            * (0.75 + hash(seed + index as f32) * 0.3)
                            * frame.screen_size.y,
                        Vec2::from_angle((hash(seed + index as f32 * 11.0) - 0.5) * 0.3),
                    );
                })
                .vertex(quad)
                .fragment(|_, surface| {
                    let point = surface.uv * 2.0 - 1.0;
                    let aa = 2.0 / quad.size.y;
                    let mut distance: f32 = -2.0;
                    for plane in 0..7 {
                        let angle = plane as f32 * 0.897_597_9 + hash(seed) * 0.4;
                        distance = distance
                            .max(point.dot(Vec2::from_angle(angle)) - 0.65 - hash(seed + plane as f32 * 5.7) * 0.15);
                    }
                    let texture = wash(point * 4.0 + seed);
                    distance += (noise(point * 23.0 + seed) - 0.5) * 0.022;
                    let mask = distance.smoothstep(aa, -aa);
                    if mask == 0.0 {
                        kill();
                    }
                    let facet = (point.x * 0.43 - point.y + (texture - 0.5) * 0.17).smoothstep(-0.08, -0.02);
                    let crack = (point.x * 0.74 + point.y + (noise(point * 6.0 + seed) - 0.5) * 0.12 - 0.2)
                        .abs()
                        .smoothstep(0.016 + aa, 0.003);
                    let moss = (-point.y + wash(point * 7.0 + seed) * 0.8).smoothstep(0.25, 0.75);
                    let stone = vec3(0.21, 0.31, 0.35).lerp(vec3(0.68, 0.62, 0.49), facet * 0.68 + texture * 0.2)
                        * (1.0 - crack * 0.45);
                    let color = stone.lerp(vec3(0.34, 0.51, 0.32).lerp(vec3(0.64, 0.7, 0.4), texture), moss * 0.88);
                    daylight(pigment(color, point + seed, aa * 0.1), surface.pixel / size)
                        .lerp(atmosphere(surface.pixel / size), (1.0 - depth).powi(2) * 0.35)
                        .extend(mask)
                })
        );
    }
    undergrowth::draw(frame, root + vec2(-0.025, 0.025), depth, seed + 10.0);
    undergrowth::draw(frame, root + vec2(0.045, 0.015), depth * 0.8, seed + 24.0);
}

pub(super) fn deadwood(frame: &mut Frame<'_>, root: Vec2, depth: f32, seed: f32) {
    shader!(
        frame
            .upload({
                let size: Vec2 = frame.screen_size;
                let seed: f32;
                let depth: f32;
                let quad: Quad = Quad::new(
                    (root - vec2(0.0, 0.017)) * frame.screen_size.y + vec2(frame.screen_size.x * 0.5, 0.0),
                    vec2(0.2 + depth * 0.32, 0.06 + depth * 0.06) * frame.screen_size.y,
                    Vec2::from_angle(-0.18 + hash(seed) * 0.35),
                );
            })
            .vertex(quad)
            .fragment(|_, surface| {
                let mut point = surface.uv * 2.0 - 1.0;
                point.y += (point.x * 3.0).sin() * 0.09;
                let aa = 2.0 / quad.size.y;
                let grain = wash(point * vec2(2.0, 15.0) + seed);
                let edge = (point.y.abs() - 0.49 + grain * 0.08)
                    .max(point.x.abs() - 0.89 + (noise(vec2(point.y * 23.0, seed)) - 0.5) * 0.12);
                let mut alpha = edge.smoothstep(aa, -aa);
                let moss = (-point.y + wash(point * 4.0 + seed) * 0.5).smoothstep(0.2, 0.65);
                let wood =
                    vec3(0.22, 0.27, 0.23).lerp(vec3(0.62, 0.49, 0.34), grain).lerp(vec3(0.39, 0.53, 0.33), moss * 0.8);
                let mut color = wood * alpha;
                for fungus in 0..4 {
                    let random = hash(seed + fungus as f32 * 7.0);
                    let offset = point - vec2(-0.6 + fungus as f32 * 0.31 + random * 0.09, -0.45 + random * 0.04);
                    let cap = ((offset / vec2(0.06 + random * 0.05, 0.15 + random * 0.08)).length() - 1.0)
                        .max(offset.y * 5.0 - 0.1);
                    let mask = cap.smoothstep(aa * 5.0, -aa * 5.0);
                    let tint = vec3(0.7, 0.54, 0.42).lerp(vec3(0.92, 0.82, 0.65), (-offset.y).smoothstep(0.0, 0.18));
                    color = color * (1.0 - mask) + tint * mask;
                    alpha += mask * (1.0 - alpha);
                }
                if alpha == 0.0 {
                    kill();
                }
                daylight(pigment(color / alpha, point + seed, aa * 0.1), surface.pixel / size)
                    .lerp(atmosphere(surface.pixel / size), (1.0 - depth).powi(2) * 0.35)
                    .extend(alpha)
            })
    );
    undergrowth::draw(frame, root + vec2(0.06, 0.01), depth, seed + 41.0);
}

pub(super) fn fish(frame: &mut Frame<'_>) {
    for index in 0..7 {
        let seed = index as f32 * 9.31 + 13.0;
        let phase = frame.time * (0.2 + hash(seed) * 0.13) + seed + (frame.time * 0.31 + seed).sin() * 0.24;
        let path = |phase: f32| {
            let y = 0.71 + (index % 3) as f32 * 0.065 + phase.sin() * (0.055 + hash(seed + 3.0) * 0.025);
            let bank = river(y);
            let lane = (hash(seed + 1.0) - 0.5) * 0.8 + phase.cos() * 0.24 + (phase * 1.7).sin() * 0.07;
            vec2(bank.x + bank.y * lane, y)
        };
        let center = path(phase);
        let velocity = path(phase + 0.02) - center;
        shader!(
            frame
                .upload({
                    let size: Vec2 = frame.screen_size;
                    let seed: f32;
                    let beat: f32 = frame.time * (6.0 + hash(seed) * 2.0) + (phase * 2.0).sin() * 0.8 + seed;
                    let quad: Quad = Quad::oriented(
                        center * frame.screen_size.y + vec2(frame.screen_size.x * 0.5, 0.0),
                        vec2(0.055 + hash(seed + 2.0) * 0.018, 0.027) * frame.screen_size.y,
                        velocity,
                    );
                })
                .vertex(quad)
                .fragment(|_, surface| {
                    let mut point = surface.uv * 2.0 - 1.0;
                    let aa = 2.0 / quad.size.y;
                    point.y -= (beat + point.x * 4.0).sin() * 0.075 * (1.0 - point.x).powi(2);
                    let body = (point.y.abs() - 0.31 * (1.0 - ((point.x - 0.12) / 0.7).powi(2)).max(0.0))
                        .max((point.x - 0.12).abs() - 0.7);
                    let tail = (point.y.abs() - (-point.x - 0.48) * 0.8).max(point.x + 0.48).max(-0.92 - point.x);
                    let fins = (point.y.abs() - 0.54 + (point.x - 0.24).abs() * 2.1).max((point.x - 0.13).abs() - 0.22);
                    let mask = body.min(tail).smoothstep(aa, -aa);
                    let fin_mask = fins.smoothstep(aa, -aa) * 0.48;
                    let alpha = mask + fin_mask * (1.0 - mask);
                    let scales = noise(point * vec2(29.0, 17.0) + seed);
                    let belly = point.y.smoothstep(-0.12, 0.22);
                    let sheen = (point.y + 0.065).abs().smoothstep(0.09 + aa, 0.02);
                    let eye =
                        ((point - vec2(0.6, -0.12)) / vec2(0.035, 0.055)).length().smoothstep(1.0 + aa * 8.0, 0.5);
                    let color = vec3(0.075, 0.21, 0.23).lerp(vec3(0.7, 0.73, 0.6), belly * 0.72 + sheen * 0.23)
                        * (0.91 + scales * 0.16)
                        * (1.0 - eye * 0.8);
                    let pixel = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                    color.extend(alpha * water_mask(pixel, 1.0 / size.y) * 0.86)
                })
        );
    }
}
