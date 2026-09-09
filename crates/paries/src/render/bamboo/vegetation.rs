use super::{
    Frame, Placement, Program, daylight, hash,
    landscape::{reflected_point, water_mask},
    noise, pigment, undergrowth, wash, wind,
};
use isthmus::{prelude::*, spirv_std::arch::kill};

pub(super) fn draw(frame: &mut Frame<'_>, plant: Placement, reflected: bool) {
    let size = frame.screen_size;
    let root = plant.position(size);
    if reflected && root.y >= 1.0 {
        return;
    }
    let young = hash(plant.seed + 9.0) < 0.12;
    let grove = noise(vec2(plant.bank_position * 2.3 + 19.0, plant.depth * 2.8)).smoothstep(0.28, 0.7);
    let height = (0.32 + plant.depth * 1.25) * (0.65 + hash(plant.seed + 1.0) * 0.8) * if young { 0.58 } else { 1.0 };
    let top = root + vec2((hash(plant.seed + 4.0) - 0.5) * height * 0.3, -height);
    let width = (0.003 + plant.depth * 0.024)
        * (0.75 + grove * 0.85)
        * (0.8 + hash(plant.seed + 2.0) * 0.4)
        * if young { 0.48 } else { 1.0 };
    let variation = hash(plant.seed + 5.0);
    let gust = wind(root, frame.time) * 0.65 + (frame.time * (0.36 + variation * 0.37) + plant.seed).sin() * 0.35;
    let curvature = height * ((variation - 0.5) * 0.05 + gust * (0.009 + variation * 0.012));
    let spacing = (0.035 + plant.depth * 0.16) * (0.8 + hash(plant.seed + 3.0) * 0.4);
    let node_offset = hash(plant.seed);
    shader!(
        frame
            .upload({
                let size: Vec2;
                let root: Vec2;
                let top: Vec2;
                let width: f32;
                let height: f32;
                let curvature: f32;
                let spacing: f32;
                let node_offset: f32;
                let seed: f32 = plant.seed;
                let depth: f32 = plant.depth;
                let reflected: bool;
            })
            .primitive({
                let start = if reflected { root } else { top };
                let end = if reflected { vec2(top.x, root.y * 2.0 - top.y) } else { root };
                Rect::new(
                    start.min(end) * size.y + vec2(size.x * 0.5, 0.0),
                    start.max(end) * size.y + vec2(size.x * 0.5, 0.0),
                )
                .expanded((width * 1.5 + curvature.abs() * 1.44 + 0.006) * size.y)
            })
            .fragment(|frame, surface| {
                let original = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                let reflection_mask = if reflected { water_mask(original, 1.0 / size.y) } else { 1.0 };
                if reflection_mask == 0.0 {
                    kill();
                }
                let mut point = original;
                if reflected {
                    point = reflected_point(point, root, frame.time);
                }
                point.x -= curvature * ((root.y - point.y) / height).clamp(0.0, 1.2).powi(2);
                let along = root.y - point.y;
                let t = (along / height).saturate();
                let across = point.x - root.x.lerp(top.x, t);
                let radius = width * (1.0 - t * 0.35 - t.powi(4) * 0.15);
                if (across.abs() - radius * 1.1).max(-along).max(along - height) > 1.0 / size.y {
                    kill();
                }
                let normal = (across / radius).clamp(-1.0, 1.0);
                let node =
                    ((along / spacing + node_offset).fract() - 0.5) * spacing + radius * 0.12 * (1.0 - normal * normal);
                let aa = 1.0 / size.y;
                let collar = node.abs().smoothstep(radius * 0.17 + aa, radius * 0.04);
                let texture = wash(vec2(across * 135.0, along * 13.0) + seed);
                let distance = (across.abs() - radius * (0.94 + collar * 0.12) + (texture - 0.5) * radius * 0.035)
                    .max(-along)
                    .max(along - height);
                let mask = distance.smoothstep(aa, -aa);
                let light = (normal * if root.x < 0.08 { 0.5 } else { -0.5 }) + 0.5;
                let panel = light.smoothstep(0.16 + texture * 0.2, 0.75 + texture * 0.1);
                let warm = hash(seed + 7.0);
                let shade = vec3(0.035, 0.22, 0.22).lerp(vec3(0.25, 0.34, 0.15), warm * 0.6);
                let lit = vec3(0.36, 0.64, 0.43).lerp(vec3(0.71, 0.76, 0.39), warm * (0.35 + depth * 0.65));
                let mut stem = shade.lerp(lit, panel) * (0.84 + texture * 0.29);
                let bloom = wash(vec2(across * 72.0, along * 36.0) + seed + 17.0).smoothstep(0.53, 0.8);
                stem = stem.lerp(vec3(0.64, 0.75, 0.59), bloom * 0.28);
                let fibres = noise(vec2(across * 1800.0, along * 9.0) + seed).smoothstep(0.65, 0.86);
                stem *= 1.0 - fibres * 0.13 * (radius * size.y / 8.0).saturate();
                let ink = (distance / radius).smoothstep(-0.16, 0.0);
                stem *= 1.0 - ink * 0.22;
                stem *= 1.0 - (node - radius * 0.19).abs().smoothstep(radius * 0.13 + aa, 0.0) * 0.32;
                stem = stem.lerp(vec3(0.79, 0.82, 0.58), collar * (0.18 + panel * 0.4) * (0.65 + texture * 0.35));
                let sheath = (along / width).smoothstep(4.8, 0.0) * (normal + texture * 0.5).smoothstep(0.15, 0.75);
                stem = stem.lerp(vec3(0.46, 0.45, 0.25), sheath * 0.7);
                let uv = vec2(original.x * size.y / size.x + 0.5, original.y);
                daylight(pigment(stem, vec2(along, across) + seed, aa), uv)
                    .lerp(vec3(0.51, 0.69, 0.61), (1.0 - depth).powi(2) * 0.35)
                    .extend(mask * reflection_mask * if reflected { 0.2 } else { 1.0 })
            })
    );
    let branches = if young { 3 } else { 4 + (hash(plant.seed + 11.0) * 3.0) as u32 };
    for branch in 0..branches + 2 {
        let seed = plant.seed + branch as f32 * 13.7;
        let crown = branch >= branches;
        let position =
            if crown { 0.985 } else { 0.22 + (branch as f32 + hash(seed + 8.0) * 0.65) / branches as f32 * 0.68 };
        let node = ((position * height / spacing + node_offset - 0.5).round() + 0.5 - node_offset) * spacing;
        let origin = root.lerp(top, if crown { position } else { (node / height).clamp(0.12, 0.92) });
        let reach = height * (0.12 + hash(seed + 12.0) * 0.1) * (1.15 - position * 0.3);
        let direction = Vec2::from_angle(if crown {
            -1.57 + (if branch == branches { -0.65 } else { 0.65 })
        } else if hash(seed) > 0.5 {
            0.1 - hash(seed + 3.0) * 1.3
        } else {
            -1.95 - hash(seed + 3.0) * 1.3
        });
        shader!(
            frame
                .upload({
                    let size: Vec2;
                    let origin: Vec2;
                    let direction: Vec2;
                    let root: Vec2;
                    let seed: f32;
                    let curvature: f32;
                    let reach: f32;
                    let height: f32;
                    let depth: f32 = plant.depth;
                    let reflected: bool;
                })
                .primitive({
                    let center = origin + (direction * 0.78 + direction.perp() * 0.35) * reach;
                    Quad::new(
                        (if reflected { vec2(center.x, root.y * 2.0 - center.y) } else { center }) * size.y
                            + vec2(size.x * 0.5, 0.0),
                        (vec2(2.0, 2.4) * reach + curvature.abs() * 2.88 + 0.012) * size.y,
                        if reflected { vec2(direction.x, -direction.y) } else { direction },
                    )
                })
                .fragment(|frame, surface| {
                    let original = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                    let reflection_mask = if reflected { water_mask(original, 1.0 / size.y) } else { 1.0 };
                    if reflection_mask == 0.0 {
                        kill();
                    }
                    let mut point = original;
                    if reflected {
                        point = reflected_point(point, root, frame.time);
                    }
                    point.x -= curvature * ((root.y - point.y) / height).clamp(0.0, 1.2).powi(2);
                    let delta = point - origin;
                    let mut local = vec2(delta.dot(direction), delta.dot(direction.perp()));
                    let droop = (0.08 + hash(seed + 2.0) * 0.2) + wind(origin, frame.time) * 0.045;
                    local.y -= reach * droop * (local.x / reach).powi(2);
                    let aa = 1.0 / size.y;
                    let stem = (local.y.abs() - reach * 0.008).max(-local.x).max(local.x - reach).smoothstep(aa, -aa);
                    let mut alpha = stem;
                    let mut color = vec3(0.025, 0.09, 0.04) * stem;
                    for leaf in 0..7 {
                        let random = hash(seed + leaf as f32 * 7.3);
                        let angle = (if leaf % 2 == 0 { 1.0 } else { -1.0 }) * (0.28 + random * 0.85)
                            + wind(origin, frame.time) * 0.07
                            + (frame.time * (0.45 + random * 0.9) + random * 9.0).sin() * 0.035;
                        let axis = Vec2::from_angle(angle);
                        let offset = local - vec2(reach * (0.1 + leaf as f32 * 0.125 + random * 0.045), 0.0);
                        let q = vec2(offset.dot(axis), offset.dot(axis.perp()));
                        let length = reach * (0.25 + random * 0.4);
                        let t = q.x / length;
                        let curve = q.y - length * 0.17 * t * t * (random - 0.3);
                        let distance = (curve.abs()
                            - length * (0.045 + random * 0.045) * (4.0 * t * (1.0 - t)).max(0.0))
                        .max(-q.x)
                        .max(q.x - length);
                        let mask = distance.smoothstep(aa, -aa);
                        let vein = curve.abs().smoothstep(aa + length * 0.006, 0.0);
                        let light = random * 0.6 + vein * 0.12 + (curve / length).smoothstep(-0.02, 0.01) * 0.16;
                        let dew = (q - vec2(length * 0.76, length * 0.017))
                            .length()
                            .smoothstep(length * 0.023 + aa, length * 0.009)
                            * random.smoothstep(0.75, 0.95);
                        let blade = vec3(0.055, 0.24, 0.16)
                            .lerp(vec3(0.63, 0.73, 0.33), light)
                            .lerp(vec3(0.94, 0.96, 0.71), dew * 0.8);
                        color = color * (1.0 - mask) + blade * mask;
                        alpha += mask * (1.0 - alpha);
                    }
                    if alpha == 0.0 {
                        kill();
                    }
                    let straight = pigment(color / alpha, local + seed, aa);
                    let uv = vec2(original.x * size.y / size.x + 0.5, original.y);
                    daylight(straight, uv)
                        .lerp(vec3(0.51, 0.69, 0.61), (1.0 - depth).powi(2) * 0.35)
                        .extend(alpha * reflection_mask * if reflected { 0.28 } else { 1.0 })
                })
        );
    }
    if !reflected {
        undergrowth::draw(frame, root, plant.depth, plant.seed);
    }
}
