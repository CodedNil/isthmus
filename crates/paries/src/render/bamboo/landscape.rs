use super::{Frame, Program, atmosphere, daylight, hash, noise, pigment, wash, wind};
use isthmus::{
    Blend, Float as _, Quad,
    glam::{Vec2, vec2, vec3},
    shader,
    spirv_std::arch::kill,
};

pub(super) fn river(y: f32) -> Vec2 {
    let depth = ((y - 0.42) / 0.58).max(0.0);
    let entrance = ((y - 0.35) / 0.07).saturate();
    vec2(
        0.07 + (depth * 5.0).sin() * 0.11 * depth + (1.0 - entrance).powi(2) * 0.055,
        0.018 * entrance + depth * depth * 0.38,
    )
}

pub(super) fn water_mask(point: Vec2, aa: f32) -> f32 {
    shore_distance(point).smoothstep(aa, -aa) * point.y.smoothstep(0.35, 0.354)
}

fn shore_distance(point: Vec2) -> f32 {
    let bank = river(point.y);
    (point.x - bank.x).abs() - bank.y + (noise(point * 73.0) - 0.5) * 0.003 * point.y.smoothstep(0.4, 0.75)
}

fn flow_coordinates(point: Vec2, time: f32) -> Vec2 {
    let bank = river(point.y);
    vec2((point.x - bank.x) / bank.y.max(0.001), (point.y - 0.32).max(0.025).ln() * 0.4 - time * 0.026)
}

pub(super) fn reflected_point(point: Vec2, root: Vec2, time: f32) -> Vec2 {
    let flow = flow_coordinates(point, time);
    let ripple = (noise(flow * vec2(3.0, 39.0)) - 0.5) * 0.7 + (noise(flow * vec2(9.0, 113.0)) - 0.5) * 0.3;
    let spread = point.y.smoothstep(0.35, 1.0);
    vec2(point.x + ripple * 0.006 * spread, root.y * 2.0 - point.y + ripple * 0.0015 * spread)
}

pub(super) fn draw(frame: &mut Frame<'_>) {
    shader!(
        frame
            .blend(Blend::Replace)
            .upload({
                let size: Vec2 = frame.screen_size;
            })
            .vertex(Quad::new(size * 0.5, size, Vec2::X))
            .fragment(|frame, surface| {
                let point = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                let uv = surface.pixel / size;
                let glow = (-((point - vec2(0.08, 0.3)) * vec2(1.5, 0.8)).length_squared() * 3.0).exp();
                let clouds = wash(point * 3.0 + vec2(frame.time * 0.003, 0.0));
                let mut color = vec3(0.58, 0.71, 0.83)
                    .lerp(vec3(0.97, 0.82, 0.71), uv.x.smoothstep(0.15, 0.9))
                    .lerp(vec3(0.97, 0.95, 0.78), glow * 0.65);
                color += (clouds - 0.5) * vec3(0.1, 0.08, 0.03);
                for ridge in 0..2 {
                    let layer = ridge as f32;
                    let slope = uv.x - 0.72 + layer * 0.085;
                    let summit = 0.025
                        + layer * 0.1
                        + slope.abs() * (1.1 - layer * 0.18)
                        + slope * 0.22
                        + (noise(vec2(uv.x * 39.0, layer * 7.0)) - 0.5) * 0.018
                        + noise(vec2(uv.x * 12.0, layer * 3.0)) * 0.03;
                    let face = wash(vec2(uv.x * 11.0 + uv.y * 3.0, uv.y * 17.0) + layer * 19.0);
                    let folds = noise(vec2(uv.x * 32.0 + uv.y * 11.0, uv.y * 4.0));
                    let mountain = vec3(0.42, 0.48, 0.65)
                        .lerp(vec3(0.68, 0.67, 0.76), face * 0.7 + folds * 0.2)
                        .lerp(vec3(0.38, 0.59, 0.59), layer * 0.5)
                        .lerp(atmosphere(uv), uv.y.smoothstep(0.2, 0.43) * 0.55);
                    color = color.lerp(mountain, (uv.y - summit).smoothstep(-0.0015, 0.002));
                }
                for layer in 0..4 {
                    let depth = layer as f32;
                    let crown = wash(vec2(point.x * (5.0 + depth * 2.0), depth * 13.0));
                    let clearing = (point.x - 0.11).abs().smoothstep(0.03, 0.7);
                    let edge = 0.36 - clearing * (0.07 + depth * 0.055) - crown * 0.065;
                    let broken = (wash(point * (22.0 + depth * 11.0)) - 0.5) * 0.035;
                    let silhouette = (point.y - edge + broken).smoothstep(-0.003, 0.005);
                    let foliage = daylight(vec3(0.25, 0.46, 0.39).lerp(vec3(0.61, 0.7, 0.39), crown), uv);
                    color = color.lerp(foliage, silhouette * (0.14 + depth * 0.04));
                }
                let shore = shore_distance(point);
                let horizon = 0.35 - (point.x - 0.125).abs() * 0.045 + (noise(vec2(point.x * 18.0, 7.0)) - 0.5) * 0.012;
                let ground = (point.y - horizon).smoothstep(-0.001, 0.002);
                if ground > 0.0 || point.y >= 0.35 {
                    let perspective = (point.y - 0.29).max(0.06);
                    let terrain = vec2(point.x / perspective, 0.24 / perspective);
                    let footprint = (26.0 / perspective + 6.3 / perspective.powi(2)) / size.y;
                    let brush = wash(terrain * vec2(4.0, 7.0));
                    let moss = wash(terrain * 2.0 + 11.0).smoothstep(0.36, 0.68);
                    let granules = noise(terrain * 83.0);
                    let visible_detail = (1.0 - footprint * 3.0).saturate();
                    let mut earth = vec3(0.14, 0.24, 0.22)
                        .lerp(vec3(0.52, 0.43, 0.32), brush)
                        .lerp(vec3(0.31, 0.47, 0.28).lerp(vec3(0.57, 0.65, 0.38), granules), moss * 0.7);
                    earth *= 0.94 + (granules - 0.35) * 0.27 * visible_detail;
                    for layer in 0..2 {
                        let layer = layer as f32;
                        let litter = terrain * vec2(17.0, 24.0) * (1.0 + layer * 0.41) + layer * vec2(13.7, 8.3);
                        let cell = litter.floor();
                        let random = noise(cell + 91.0);
                        let axis = Vec2::from_angle(noise(cell + 17.0) * 6.283_185_5);
                        let offset = litter - cell - 0.5 - vec2(random - 0.5, noise(cell + 33.0) - 0.5) * 0.22;
                        let leaf = vec2(offset.dot(axis), offset.dot(axis.perp()));
                        let length = 0.16 + random * 0.17;
                        let curve = leaf.y - leaf.x * leaf.x * (random - 0.5);
                        let distance = (curve.abs() - (1.0 - (leaf.x / length).powi(2)).max(0.0) * 0.055)
                            .max(leaf.x.abs() - length);
                        let aa = footprint * (1.0 + layer * 0.41);
                        let mask = distance.smoothstep(aa, -aa) * random.smoothstep(0.17, 0.4) * visible_detail;
                        let vein = curve.abs().smoothstep(0.008 + aa, 0.0);
                        let dry_leaf = vec3(0.3, 0.31, 0.22).lerp(vec3(0.75, 0.63, 0.4), random)
                            * (0.86 + vein * 0.14 + curve.smoothstep(-0.03, 0.03) * 0.15);
                        earth = earth.lerp(dry_leaf, mask * 0.9);
                    }
                    let gravel = terrain * 29.0;
                    let cell = gravel.floor();
                    let random = noise(cell + 71.0);
                    let chip = gravel - cell - 0.5 - vec2(random - 0.5, noise(cell + 21.0) - 0.5) * 0.3;
                    let edge = (chip.x.abs() + chip.y.abs() * 0.7 - 0.2).max(chip.y.abs() - 0.12 - random * 0.06);
                    let stone = edge.smoothstep(footprint, -footprint) * random.smoothstep(0.55, 0.8) * visible_detail;
                    earth = earth
                        .lerp(vec3(0.32, 0.4, 0.4).lerp(vec3(0.7, 0.65, 0.5), (-chip.y).smoothstep(-0.04, 0.1)), stone);
                    let dapple = wash(terrain * 3.0 + vec2(frame.time * 0.014, frame.time * -0.008));
                    earth = earth.lerp(vec3(0.72, 0.73, 0.42), dapple.smoothstep(0.48, 0.76) * 0.19);
                    earth = daylight(earth, uv).lerp(vec3(0.53, 0.67, 0.59), (-perspective * 6.0).exp() * 0.42);
                    color = color.lerp(earth, ground * shore.smoothstep(-0.001, 0.001));
                    let channel = flow_coordinates(point, 0.0).x.abs().saturate();
                    let bed = wash(terrain * 7.0);
                    let sand = vec3(0.33, 0.43, 0.36).lerp(vec3(0.7, 0.65, 0.45), bed);
                    let water = vec3(0.13, 0.39, 0.4)
                        .lerp(vec3(0.47, 0.69, 0.61), glow * 0.5)
                        .lerp(sand, channel.powi(3) * 0.62);
                    color = color.lerp(water, water_mask(point, 1.0 / size.y));
                    let wet_edge = shore.abs().smoothstep(0.011, 0.001) * ground;
                    color = color.lerp(vec3(0.12, 0.25, 0.22), wet_edge * 0.45);
                }
                color = pigment(color, point, 1.0 / size.y);
                color.extend(1.0)
            })
    );
}

pub(super) fn water(frame: &mut Frame<'_>) {
    shader!(
        frame
            .upload({
                let size: Vec2 = frame.screen_size;
            })
            .vertex(Quad::from_min_max(vec2(0.0, size.y * 0.35), size))
            .fragment(|frame, surface| {
                let point = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                let mask = water_mask(point, 1.0 / size.y);
                if mask == 0.0 {
                    kill();
                }
                let flow = flow_coordinates(point, frame.time);
                let broad = noise(flow * vec2(2.3, 17.0));
                let warp = vec2(noise(flow * vec2(5.0, 23.0)), noise(flow * vec2(3.0, 31.0))) - 0.5;
                let ripples = flow + warp * vec2(0.12, 0.008);
                let strokes = noise(ripples * vec2(5.0, 61.0));
                let broken = strokes.smoothstep(0.55, 0.7) * broad.smoothstep(0.38, 0.65);
                let opening = (1.0 - (flow.x + broad * 0.35 - 0.12).abs()).saturate();
                let detail = point.y.smoothstep(0.39, 0.64);
                let lace = (noise(ripples * vec2(18.0, 53.0)) - 0.52).abs().smoothstep(0.07, 0.018);
                let shallows = flow.x.abs().smoothstep(0.4, 0.95);
                let caustics = lace * shallows * detail * 0.18;
                let glints =
                    noise(ripples * vec2(13.0, 173.0)).smoothstep(0.68, 0.85) * broad.smoothstep(0.47, 0.66) * detail;
                let color = vec3(0.29, 0.57, 0.54)
                    .lerp(vec3(0.83, 0.86, 0.69), opening * 0.48 + broken * 0.35)
                    .lerp(vec3(0.99, 0.96, 0.79), glints * 0.65 + caustics);
                daylight(pigment(color, point, 1.0 / size.y), surface.pixel / size)
                    .extend(mask * (0.16 + broken * 0.48 + glints * 0.32 + caustics).saturate())
            })
    );
}

pub(super) fn haze(frame: &mut Frame<'_>, layer: f32) {
    shader!(
        frame
            .upload({
                let size: Vec2 = frame.screen_size;
                let layer: f32;
            })
            .vertex(Quad::new(size * 0.5, size, Vec2::X))
            .fragment(|frame, surface| {
                let point = (surface.pixel - vec2(size.x * 0.5, 0.0)) / size.y;
                let fog = noise(point * vec2(3.0, 8.0) + vec2(frame.time * 0.018 + layer * 13.0, layer * 3.0));
                let height = (-(point.y - 0.52 - layer * 0.12).powi(2) * 35.0).exp();
                let offset = point - vec2(0.08, 0.22);
                let shaft = wash(vec2((point.x + point.y * 0.24) * 3.0, frame.time * 0.012 + layer * 4.0));
                let light = shaft.smoothstep(0.35, 0.75) * (-offset.length_squared() * 2.0).exp();
                let cell = (point * 65.0 + vec2(wind(point, frame.time) * 0.3, frame.time * 0.1)).floor();
                let speck = ((point * 65.0 + vec2(wind(point, frame.time) * 0.3, frame.time * 0.1)).fract()
                    - vec2(hash(cell.x), hash(cell.y)))
                .length()
                .smoothstep(0.09, 0.015)
                    * hash(cell.dot(vec2(17.0, 43.0))).smoothstep(0.985, 1.0);
                atmosphere(surface.pixel / size)
                    .lerp(vec3(1.0, 0.86, 0.48), light)
                    .extend((fog * height * 0.075 + light * 0.04 + speck * light * 0.45) * (1.1 - layer * 0.45))
            })
    );
}

pub(super) fn falling_leaves(frame: &mut Frame<'_>) {
    for index in 0..18 {
        let seed = index as f32 * 13.7;
        let age = (frame.time * (0.012 + hash(seed) * 0.008) + hash(seed + 1.0)).fract();
        let scale = frame.screen_size.y;
        shader!(
            frame
                .upload({
                    let alpha: f32 = age.smoothstep(0.0, 0.12) * age.smoothstep(1.0, 0.8) * 0.75;
                    let tint: f32 = hash(seed + 3.0);
                    let quad: Quad = Quad::oriented(
                        vec2(
                            frame.screen_size.x * hash(seed + 2.0) + (age * 9.0 + seed).sin() * scale * 0.035,
                            age * scale,
                        ),
                        vec2(0.008 + hash(seed + 4.0) * 0.008, 0.004 * (0.3 + (frame.time * 1.7 + seed).sin().abs()))
                            * scale,
                        Vec2::from_angle(frame.time * 0.3 + seed + (frame.time * 0.7 + seed).sin() * 0.6),
                    );
                })
                .vertex(quad)
                .fragment(|_, surface| {
                    let point = surface.uv * 2.0 - 1.0;
                    let mask = (point.y.abs() - (1.0 - point.x * point.x).max(0.0)).smoothstep(0.2, -0.2);
                    vec3(0.23, 0.3, 0.075).lerp(vec3(0.65, 0.49, 0.14), tint).extend(mask * alpha)
                })
        );
    }
}

pub(super) fn butterflies(frame: &mut Frame<'_>) {
    for index in 0..3 {
        let seed = index as f32 * 17.3 + 9.0;
        let phase = frame.time * (0.2 + hash(seed) * 0.12) + seed;
        shader!(
            frame
                .upload({
                    let seed: f32;
                    let quad: Quad = Quad::new(
                        vec2(
                            frame.screen_size.x * (0.26 + index as f32 * 0.21)
                                + phase.sin() * frame.screen_size.y * 0.07,
                            frame.screen_size.y * (0.57 + (phase * 1.7).sin() * 0.05 + index as f32 * 0.055),
                        ),
                        vec2(0.022, 0.018) * frame.screen_size.y,
                        Vec2::from_angle((phase * 1.3).sin() * 0.5),
                    );
                })
                .vertex(quad)
                .fragment(|frame, surface| {
                    let point = surface.uv * 2.0 - 1.0;
                    let spread = 0.15 + (frame.time * (7.0 + hash(seed) * 2.0) + seed).sin().abs() * 0.85;
                    let wing = vec2(point.x.abs() / spread, point.y);
                    let upper = ((wing - vec2(0.42, -0.26)) / vec2(0.5, 0.55)).length() - 1.0;
                    let lower = ((wing - vec2(0.32, 0.32)) / vec2(0.36, 0.4)).length() - 1.0;
                    let edge = upper.min(lower);
                    let wings = edge.smoothstep(0.13, -0.13);
                    let body = (point.x.abs() - 0.055).max(point.y.abs() - 0.5).smoothstep(0.08, -0.04);
                    let color = vec3(0.48, 0.32, 0.17)
                        .lerp(vec3(0.95, 0.78, 0.43), edge.smoothstep(0.0, -0.5))
                        .lerp(vec3(0.12, 0.24, 0.18), body);
                    color.extend(wings.max(body) * 0.9)
                })
        );
    }
}
