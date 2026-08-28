use super::{Fragment, fbm, hash, noise, tapered_segment};
use core::f32::consts::PI;
#[cfg(target_arch = "spirv")]
use isthmus::Float as _;
use isthmus::{
    Quad,
    glam::{FloatExt, Vec2, Vec3, Vec4, vec2, vec3},
};

#[cfg(not(target_arch = "spirv"))]
use isthmus::Frame;

pub struct Bamboo;

#[isthmus::paint]
impl Bamboo {
    #[cfg(not(target_arch = "spirv"))]
    pub fn show(&mut self, frame: &mut Frame<'_>) {
        let size = frame.screen_size;
        frame.set_globals(super::Globals::default());
        frame.paint(
            Quad::new(size * 0.5, size, Vec2::X),
            |fragment: Fragment, size: Vec2| wallpaper(fragment.pixel, size, fragment.time),
        );
    }
}

fn over(base: Vec3, paint: Vec3, alpha: f32) -> Vec3 {
    base.lerp(paint, alpha.saturate())
}

fn leaf_mask(point: Vec2, center: Vec2, axis: Vec2, length: f32, width: f32) -> f32 {
    let q = point - center;
    let x = q.dot(axis) / length;
    let y = q.dot(axis.perp()) / width;
    let taper = (1.0 - x.abs()).max(0.0);
    let active = taper.smoothstep(0.0, 0.035);
    (y.abs() - taper * taper.sqrt()).smoothstep(0.08, -0.08) * active
}

fn culm(
    point: Vec2,
    root: Vec2,
    top: Vec2,
    root_radius: f32,
    spacing: f32,
    dark: Vec3,
    light: Vec3,
) -> (f32, f32, Vec3) {
    let line = top - root;
    let length = line.length();
    let direction = line / length;
    let across = direction.perp();
    let local = point - root;
    let along = local.dot(direction);
    let progress = (along / length).clamp(0.0, 1.0);
    let radius = root_radius.lerp(root_radius * 0.56, progress);
    let cell = (along / spacing).fract().abs();
    let node = if cell > 0.5 { cell - 1.0 } else { cell };
    let ridge = (1.0 - (node - 0.025).abs() / 0.052).saturate();
    let ridge = ridge * ridge * (3.0 - ridge * 2.0);
    let shoulder = (1.0 - (node + 0.018).abs() / 0.090).saturate();
    let shoulder = shoulder * shoulder * (3.0 - shoulder * 2.0);
    let internode = (cell * PI).sin() * 0.022;
    let shaped_radius = radius * (0.965 + internode + ridge * 0.12 + shoulder * 0.055);
    let cross = local.dot(across);
    let side = cross.abs() - shaped_radius;
    let ends = (-along).max(along - length);
    let mask = side.max(ends).smoothstep(1.2, -1.2);
    let seam = (1.0 - (node - 0.025).abs() / 0.006).saturate() * mask;
    let fibre = ((cross / radius * 4.2 + along * 0.004).sin() * 0.5 + 0.5).saturate();
    let shade = (0.50 - cross / radius * 0.40 + fibre * 0.09).saturate();
    let yellow = fibre * fibre * 0.12 + ridge * 0.08;
    let stem = dark.lerp(light, shade).lerp(vec3(0.72, 0.59, 0.13), yellow.saturate());
    (mask, seam, stem)
}

fn paint_culm(
    color: Vec3,
    point: Vec2,
    root: Vec2,
    top: Vec2,
    radius: f32,
    spacing: f32,
    dark: Vec3,
    light: Vec3,
) -> Vec3 {
    let (mask, seam, stem) = culm(point, root, top, radius, spacing, dark, light);
    let color = over(color, stem, mask);
    over(color, dark * 0.52, seam * 0.68)
}

fn foliage(color: Vec3, point: Vec2, root: Vec2, direction: Vec2, scale: f32, alpha: f32) -> Vec3 {
    let axis = direction.normalize();
    let side = axis.perp();
    let tip = root + axis * 88.0 * scale;
    let twig = tapered_segment(point, root, tip, 2.2 * scale, 0.5 * scale).fill();
    let color = over(color, vec3(0.018, 0.11, 0.035), twig * alpha);
    let left = (axis - side * 0.46).normalize();
    let right = (axis + side * 0.46).normalize();
    let first = leaf_mask(
        point,
        root.lerp(tip, 0.47) + left * 18.0 * scale,
        left,
        29.0 * scale,
        6.5 * scale,
    );
    let second = leaf_mask(
        point,
        root.lerp(tip, 0.68) + right * 21.0 * scale,
        right,
        34.0 * scale,
        7.0 * scale,
    );
    let third = leaf_mask(
        point,
        root.lerp(tip, 0.86) + axis * 19.0 * scale,
        axis,
        33.0 * scale,
        6.8 * scale,
    );
    let leaves = first.max(second).max(third) * alpha;
    let glint = (0.54 - (point - root).dot(side) / (130.0 * scale)).saturate();
    over(
        color,
        vec3(0.025, 0.17, 0.045).lerp(vec3(0.43, 0.67, 0.14), glint),
        leaves,
    )
}

fn root_tuft(color: Vec3, point: Vec2, root: Vec2, scale: f32, seed: f32) -> Vec3 {
    let q = point - root;
    let radius = 18.0 * scale;
    let spacing = 4.6 * scale;
    let cell = ((q.x + radius) / spacing).floor();
    let blade_seed = hash(cell * 5.17 + seed * 13.0);
    let blade_root = root + vec2(-radius + (cell + 0.5) * spacing, 0.8 * scale);
    let height = (10.0 + blade_seed * 15.0) * scale;
    let tip = blade_root + vec2((blade_seed - 0.5) * 11.0 * scale, -height);
    let blade = tapered_segment(point, blade_root, tip, 1.25 * scale, 0.18 * scale).fill();
    let active = q.x.abs().smoothstep(radius + spacing, radius);
    over(
        color,
        vec3(0.055, 0.15, 0.050).lerp(vec3(0.28, 0.40, 0.085), blade_seed),
        blade * active,
    )
}

fn terrain_profile(x: f32, drift: f32, character: f32) -> f32 {
    let p = x + drift;
    (p * (3.2 + character * 0.35)).sin() * 0.052
        + (p * (7.7 + character) + 1.3).sin() * 0.021
        + (p * (15.1 - character * 0.6) + 2.5).sin() * 0.009
}

fn terrain_height(x: f32, base: f32, drift: f32, character: f32) -> f32 {
    base + terrain_profile(x, drift, character)
}

#[allow(clippy::too_many_arguments)]
fn paint_terrain(
    color: Vec3,
    uv: Vec2,
    height: f32,
    base: Vec3,
    accent: Vec3,
    wash: f32,
    brush: f32,
    edge: f32,
) -> Vec3 {
    let broken_edge = height + (brush - 0.5) * edge * 5.0;
    let mask = (uv.y - broken_edge).smoothstep(-edge, edge);
    let depth = ((uv.y - height) * 2.4).saturate();
    let pigment = (wash * 0.55 + brush * 0.34).saturate();
    let mut paint = base.lerp(accent, pigment * (1.0 - depth * 0.42));
    let dry = brush.smoothstep(0.56, 0.78);
    let pooling = (1.0 - wash.smoothstep(0.26, 0.58)) * (0.16 + depth * 0.10);
    paint = paint.lerp(base * 0.66, pooling);
    paint = paint.lerp(accent * 0.94, dry * 0.38);
    let granulation = (wash - brush).abs();
    paint += accent * (granulation * 0.11 - 0.025);
    let rim = (1.0 - (uv.y - broken_edge).abs() / 0.014).saturate();
    paint += accent * rim * (0.10 + brush * 0.13);
    over(color, paint, mask)
}

#[allow(clippy::too_many_arguments)]
fn forest_layer(
    color: Vec3,
    point: Vec2,
    size: Vec2,
    offset: f32,
    period: f32,
    radius: f32,
    lean: f32,
    ground_base: f32,
    ground_drift: f32,
    ground_character: f32,
    ground_depth: f32,
    height_scale: f32,
    dark: Vec3,
    light: Vec3,
) -> Vec3 {
    let scale = (size.y / 1080.0).max(0.55);
    let cell = ((point.x + offset) / period).floor();
    let seed = hash(cell * 7.13 + offset * 0.019);
    let depth_seed = hash(cell * 3.97 + offset * 0.031 + 5.4);
    let root_x = (cell + 0.5) * period - offset;
    let ridge = terrain_height(root_x / size.x, ground_base, ground_drift, ground_character);
    let rolling_depth =
        depth_seed * 0.82 + ((root_x / size.x) * 8.0 + depth_seed * 4.0 + ground_character).sin() * 0.08 + 0.10;
    let ground = ridge + ground_depth * rolling_depth.saturate();
    let root = vec2(root_x, ground * size.y);
    let perspective = 0.68 + rolling_depth.saturate() * 0.48;
    let height = size.y * height_scale * (0.48 + seed * 0.72) * perspective;
    let top = root + vec2(lean * (0.55 + seed * 0.45), -height);
    let color = paint_culm(
        color,
        point,
        root,
        top,
        radius * (0.72 + seed * 0.38) * perspective,
        (92.0 + seed * 45.0) * scale,
        dark,
        light,
    );
    let direction = (top - root).normalize();
    let branch_sign = if seed < 0.5 { -1.0 } else { 1.0 };
    foliage(
        color,
        point,
        top - direction * 13.0 * scale,
        vec2(branch_sign * 0.90, -0.42),
        scale * (0.22 + radius / (45.0 * scale)) * perspective,
        1.0,
    )
}

#[allow(clippy::too_many_arguments)]
fn grass_layer(
    color: Vec3,
    point: Vec2,
    size: Vec2,
    offset: f32,
    period: f32,
    ground_base: f32,
    ground_drift: f32,
    ground_character: f32,
    ground_depth: f32,
    grass_scale: f32,
) -> Vec3 {
    let cell = ((point.x + offset) / period).floor();
    let seed = hash(cell * 4.73 + offset * 0.027);
    let depth_seed = hash(cell * 8.19 + offset * 0.011 + 3.2);
    let root_x = (cell + 0.5) * period - offset;
    let ridge = terrain_height(root_x / size.x, ground_base, ground_drift, ground_character);
    let depth = (0.08 + depth_seed * 0.84 + ((root_x / size.x) * 9.0 + depth_seed * 3.7).sin() * 0.07).saturate();
    let root = vec2(root_x, (ridge + ground_depth * depth) * size.y);
    root_tuft(color, point, root, grass_scale * (0.72 + depth * 0.48), seed)
}

fn foreground_culm(
    color: Vec3,
    point: Vec2,
    size: Vec2,
    time: f32,
    x: f32,
    lean: f32,
    radius: f32,
    height: f32,
    seed: f32,
) -> Vec3 {
    let scale = (size.y / 1080.0).max(0.55);
    let ridge = terrain_height(x, 0.79, -time * 0.0022 + 3.4, 1.30);
    let ground = ridge + 0.15 * (0.18 + seed * 0.72 + (x * 11.0 + seed * 3.0).sin() * 0.07);
    let root = vec2(x * size.x, ground * size.y);
    let breeze = (time * 0.20 + seed * 2.0).sin() * 14.0 * scale;
    let top = root + vec2(lean * scale + breeze, -height * size.y);
    let dark = vec3(0.018, 0.095, 0.045);
    let color = paint_culm(
        color,
        point,
        root,
        top,
        radius * scale,
        (132.0 + seed * 28.0) * scale,
        dark,
        vec3(0.39, 0.64, 0.17),
    );
    let branch_sign = if seed < 0.5 { -1.0 } else { 1.0 };
    foliage(
        color,
        point,
        root.lerp(top, 0.62 + seed * 0.14),
        vec2(branch_sign * 0.92, -0.39),
        scale * 1.05,
        1.0,
    )
}

fn soft_disc(point: Vec2, center: Vec2, radius: f32) -> f32 {
    (1.0 - (point - center).length() / radius).saturate()
}

fn soft_beam(slope: f32, center: f32, width: f32) -> f32 {
    let beam = (1.0 - (slope - center).abs() / width).saturate();
    beam * beam * (3.0 - beam * 2.0)
}

fn wallpaper(point: Vec2, size: Vec2, time: f32) -> Vec4 {
    let uv = point / size;
    let scale = (size.y / 1080.0).max(0.55);
    let wash = fbm(point * 0.0022 + vec2(7.0 + time * 0.010, 13.0));
    let brush = noise(point * 0.0075 + vec2(21.0, 4.0));
    let sun_position = vec2(0.48, 0.24);
    let sun = soft_disc(uv, sun_position, 0.43);
    let mut color = vec3(0.30, 0.40, 0.48).lerp(vec3(0.81, 0.76, 0.58), (1.0 - uv.y) * 0.72);
    color = color.lerp(vec3(1.0, 0.87, 0.57), sun * sun * 0.80);
    color += (wash - 0.5) * vec3(0.065, 0.055, 0.075);
    let from_sun = uv - sun_position;
    let slope = from_sun.x / (from_sun.y.abs() + 0.075);
    let drift = (time * 0.045).sin() * 0.035;
    let beams = soft_beam(slope, -0.68 + drift, 0.13) * 0.42
        + soft_beam(slope, -0.39 - drift * 0.35, 0.20) * 0.70
        + soft_beam(slope, -0.08 + drift * 0.22, 0.105)
        + soft_beam(slope, 0.23 - drift * 0.42, 0.16) * 0.78
        + soft_beam(slope, 0.55 + drift * 0.28, 0.23) * 0.48;
    let ray_fade = from_sun.y.smoothstep(-0.03, 0.13) * (1.0 - from_sun.length() * 0.74).saturate();
    let ray_pigment = 0.64 + wash * 0.24 + brush * 0.12;
    color = over(color, vec3(1.0, 0.79, 0.39), beams * ray_fade * ray_pigment * 0.30);
    color += vec3(1.0, 0.69, 0.31) * sun * ray_fade * 0.055;
    color = over(color, vec3(1.0, 0.89, 0.62), soft_disc(uv, sun_position, 0.095) * 0.27);

    let edge = 1.6 / size.y;
    let far_drift = time * 0.0012;
    let far_height = terrain_height(uv.x, 0.52, far_drift, 0.30);
    color = paint_terrain(
        color,
        uv,
        far_height,
        vec3(0.18, 0.30, 0.31),
        vec3(0.39, 0.49, 0.36),
        wash,
        brush,
        edge,
    );
    color = forest_layer(
        color,
        point,
        size,
        19.0 * scale,
        size.x * 0.043,
        5.0 * scale,
        35.0 * scale,
        0.52,
        far_drift,
        0.30,
        0.12,
        0.34,
        vec3(0.08, 0.18, 0.17),
        vec3(0.29, 0.45, 0.24),
    );
    color = grass_layer(
        color,
        point,
        size,
        41.0 * scale,
        size.x * 0.032,
        0.52,
        far_drift,
        0.30,
        0.12,
        scale * 0.62,
    );

    let middle_drift = -time * 0.0017 + 1.8;
    let middle_height = terrain_height(uv.x, 0.66, middle_drift, 0.82);
    color = paint_terrain(
        color,
        uv,
        middle_height,
        vec3(0.085, 0.22, 0.19),
        vec3(0.25, 0.39, 0.24),
        wash,
        1.0 - brush,
        edge,
    );
    color = forest_layer(
        color,
        point,
        size,
        77.0 * scale,
        size.x * 0.060,
        8.0 * scale,
        -50.0 * scale,
        0.66,
        middle_drift,
        0.82,
        0.16,
        0.45,
        vec3(0.035, 0.13, 0.085),
        vec3(0.32, 0.53, 0.16),
    );
    color = grass_layer(
        color,
        point,
        size,
        96.0 * scale,
        size.x * 0.040,
        0.66,
        middle_drift,
        0.82,
        0.16,
        scale * 0.82,
    );

    let near_drift = -time * 0.0022 + 3.4;
    let near_height = terrain_height(uv.x, 0.79, near_drift, 1.30);
    color = paint_terrain(
        color,
        uv,
        near_height,
        vec3(0.025, 0.12, 0.09),
        vec3(0.16, 0.30, 0.15),
        wash,
        brush,
        edge,
    );
    color = forest_layer(
        color,
        point,
        size,
        143.0 * scale,
        size.x * 0.080,
        12.0 * scale,
        65.0 * scale,
        0.79,
        near_drift,
        1.30,
        0.17,
        0.56,
        vec3(0.018, 0.095, 0.050),
        vec3(0.37, 0.60, 0.17),
    );

    color = foreground_culm(color, point, size, time, 0.075, 42.0, 25.0, 0.91, 0.14);
    color = foreground_culm(color, point, size, time, 0.30, -38.0, 21.0, 0.82, 0.73);
    color = foreground_culm(color, point, size, time, 0.53, 28.0, 27.0, 0.94, 0.32);
    color = foreground_culm(color, point, size, time, 0.76, -31.0, 23.0, 0.86, 0.87);
    color = foreground_culm(color, point, size, time, 0.96, 35.0, 26.0, 0.92, 0.46);
    color = grass_layer(
        color,
        point,
        size,
        157.0 * scale,
        size.x * 0.045,
        0.79,
        near_drift,
        1.30,
        0.17,
        scale * 1.06,
    );
    color.extend(1.0)
}
