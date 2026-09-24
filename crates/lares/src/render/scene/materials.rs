use crate::home::Material;
use isthmus::prelude::*;

fn hash(cell: Vec2) -> f32 {
    let bits = (cell.x as i32 as u32).wrapping_mul(0x9e37_79b9) ^ (cell.y as i32 as u32).wrapping_mul(0x85eb_ca6b);
    let bits = (bits ^ (bits >> 16)).wrapping_mul(0x7feb_352d);
    ((bits ^ (bits >> 15)) as f32) * 2.328_306_4e-10
}

fn noise(point: Vec2) -> f32 {
    let cell = point.floor();
    let fraction = point - cell;
    let blend = fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    let corner = |offset: Vec2| hash(cell + offset);
    corner(Vec2::ZERO).lerp(corner(Vec2::X), blend.x).lerp(corner(Vec2::Y).lerp(corner(Vec2::ONE), blend.x), blend.y)
}

fn visibility(frequency: f32, pixel: f32) -> f32 {
    (1.0 - frequency * pixel * 2.0).saturate()
}

fn coverage(distance: f32, edge: f32, pixel: f32) -> f32 {
    ((distance - edge) / pixel.max(1.0e-5)).clamp(0.0, 1.0)
}

fn wood(point: Vec2, pixel: f32) -> Vec3 {
    let width = 0.13;
    let length = 1.9;
    let row = (point.y / width).floor();
    let along = (point.x + hash(vec2(row, 3.0)) * length) / length;
    let index = vec2(row, along.floor());
    let seed = hash(index);
    let tone = vec3(0.55, 0.35, 0.2).lerp(vec3(0.71, 0.5, 0.32), seed);
    let grain = noise(vec2(point.x * 2.75, point.y * 55.0 + seed * 40.0));
    let color = tone * (0.93 + grain * 0.12 * visibility(55.0, pixel));
    let across = ((point.y / width).fract() - 0.5).abs() * width;
    let joint = (along.fract() - 0.5).abs() * length;
    let gap = 1.0 - coverage(across.min(joint), 0.004, pixel);
    color.lerp(vec3(0.38, 0.24, 0.14), gap * 0.7)
}

fn carpet(point: Vec2, pixel: f32) -> Vec3 {
    let fibre = noise(point * 180.0) * visibility(180.0, pixel);
    vec3(0.69, 0.63, 0.53) * (0.92 + fibre * 0.16)
}

fn tile(point: Vec2, pixel: f32) -> Vec3 {
    let size = 0.3;
    let cell = point / size;
    let inset = (cell.fract() - 0.5).abs();
    let to_edge = (0.5 - inset.max_element()) * size;
    let tile = vec3(0.8, 0.8, 0.77) * (0.96 + hash(cell.floor()) * 0.07);
    tile.lerp(vec3(0.52, 0.52, 0.51), 1.0 - coverage(to_edge, 0.005, pixel))
}

fn fabric(point: Vec3, pixel: f32, tint: Vec3) -> Vec3 {
    let weave = noise(point.xy() * 140.0 + point.z * 23.0) * visibility(140.0, pixel);
    tint * (0.92 + weave * 0.16)
}

pub(super) fn color(material: Material, tint: Vec3, point: Vec3, pixel: f32) -> Vec3 {
    let (frequency, variation) = match material {
        Material::Wood => return wood(point.xy(), pixel) * tint,
        Material::Carpet => return carpet(point.xy(), pixel) * tint,
        Material::Tile => return tile(point.xy(), pixel) * tint,
        Material::Fabric => return fabric(point, pixel, tint),
        Material::Paint => return tint,
        Material::Stone => (38.0, 0.16),
        Material::Ceramic => (30.0, 0.04),
        Material::Metal => (65.0, 0.08),
        Material::Foliage => (44.0, 0.3),
        Material::Soil => (70.0, 0.2),
    };
    tint * (1.0 + (noise(point.xy() * frequency + point.z * 0.4) - 0.5) * variation)
}
