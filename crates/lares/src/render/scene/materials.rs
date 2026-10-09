//! Procedural metallic/roughness materials, evaluated in the surface tangent frame.
use crate::home::Material;
use core::f32::consts::TAU;
use isthmus::prelude::*;

#[derive(Clone, Copy)]
pub(super) struct Surface {
    pub albedo: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    pub sheen: f32,
    pub coat: f32,
    pub gradient: Vec3,
    pub occlusion: f32,
}
fn hash(cell: Vec2) -> f32 {
    let bits = (cell.x as i32 as u32).wrapping_mul(0x9e37_79b9) ^ (cell.y as i32 as u32).wrapping_mul(0x85eb_ca6b);
    let bits = (bits ^ (bits >> 16)).wrapping_mul(0x7feb_352d);
    ((bits ^ (bits >> 15)) as f32) * 2.328_306_4e-10
}
/// Value and analytic derivatives share the same four lattice lookups.
#[inline(never)]
fn noise_gradient(p: Vec2) -> Vec3 {
    let cell = p.floor();
    let fraction = p - cell;
    let blend = fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    let du = 30.0 * fraction * fraction * (fraction - 1.0) * (fraction - 1.0);
    let lower_left = hash(cell);
    let lower_right = hash(cell + Vec2::X);
    let upper_left = hash(cell + Vec2::Y);
    let upper_right = hash(cell + Vec2::ONE);
    vec3(
        lower_left.lerp(lower_right, blend.x).lerp(upper_left.lerp(upper_right, blend.x), blend.y),
        (lower_right - lower_left).lerp(upper_right - upper_left, blend.y) * du.x,
        (upper_left.lerp(upper_right, blend.x) - lower_left.lerp(lower_right, blend.x)) * du.y,
    )
}
#[inline(never)]
pub(super) fn noise(p: Vec2) -> f32 {
    noise_gradient(p).x
}
/// Filtered value and tangent gradient; one evaluation supplies color and bump.
fn detail(uv: Vec2, scale: Vec2, pixel: f32) -> Vec3 {
    let weight = (1.0 - scale.max_element() * pixel * 2.0).max_num(0.0).min_num(1.0);
    if weight == 0.0 {
        return Vec3::ZERO;
    }
    let p = uv * scale;
    let a = noise_gradient(p);
    let b = noise_gradient(vec2(p.x * 0.8 - p.y * 0.6, p.x * 0.6 + p.y * 0.8) + 17.3);
    let derivative = (a.yz() * 0.55 + vec2(b.y * 0.8 + b.z * 0.6, -b.y * 0.6 + b.z * 0.8) * 0.45) * scale;
    vec3(a.x * 0.55 + b.x * 0.45 - 0.5, derivative.x, derivative.y) * weight
}
fn wave(phase: f32, frequency: f32, pixel: f32) -> f32 {
    phase.sin() * (1.0 - frequency * pixel * 2.0).max_num(0.0).min_num(1.0)
}
fn edge(distance: f32, width: f32, pixel: f32) -> f32 {
    1.0 - ((distance - width) / pixel.max_num(0.0001) + 0.5).max_num(0.0).min_num(1.0)
}
fn linear(color: Vec3) -> Vec3 {
    // sRGB albedo inputs; all illumination is evaluated in linear light.
    color.clamp(Vec3::ZERO, Vec3::ONE).powf(2.2)
}
#[inline(never)]
fn sample_plane(kind: Material, tint: Vec3, uv: Vec2, pixel: f32) -> Surface {
    let mut surface = Surface {
        albedo: tint,
        roughness: 0.65,
        metallic: 0.0,
        sheen: 0.0,
        coat: 0.0,
        gradient: Vec3::ZERO,
        occlusion: 1.0,
    };
    match kind {
        Material::Wood | Material::Floorboards | Material::Maple | Material::MapleHorizontal => {
            let maple = matches!(kind, Material::Maple | Material::MapleHorizontal);
            let uv = if kind == Material::MapleHorizontal { uv.yx() } else { uv };
            let boards = kind == Material::Floorboards;
            let row = (uv.y / 0.19).floor();
            let offset = hash(vec2(row, 7.0)) * 1.65;
            let board = vec2(((uv.x + offset) / 1.65).floor(), row);
            let seed = if boards { hash(board) } else { 0.45 };
            let warp = detail(uv, vec2(1.8, 6.0), pixel) * if maple { 0.22 } else { 1.0 };
            let grain_scale = if boards { vec2(1.6, 85.0) } else { vec2(70.0, 2.0) };
            let grain_uv = uv * grain_scale;
            let grain = detail(uv + Vec2::splat(warp.x * 3.0) / grain_scale, grain_scale, pixel);
            let pores = detail(uv, vec2(180.0, 32.0), pixel);
            let growth = detail(uv, if boards { vec2(0.8, 18.0) } else { vec2(16.0, 1.2) }, pixel);
            let rings = wave(grain_uv.x * 0.9 + grain_uv.y * 0.55 + warp.x * 7.0, 35.0, pixel);
            let tone = if boards {
                vec3(0.36, 0.26, 0.19).lerp(vec3(0.49, 0.36, 0.25), seed * 0.65 + 0.15)
            } else if maple {
                vec3(0.84, 0.78, 0.67)
            } else {
                vec3(0.62, 0.46, 0.30)
            };
            let knot = if boards && seed > 0.82 {
                let center = vec2((board.x + 0.25 + hash(board + Vec2::X) * 0.5) * 1.65 - offset, (row + 0.5) * 0.19);
                let radial = ((uv - center) * vec2(3.0, 12.0)).length();
                (1.0 - radial).max_num(0.0).min_num(1.0) * wave(radial * 45.0 + warp.x * 2.0, 25.0, pixel)
            } else {
                0.0
            };
            let seams = if boards {
                let f = uv.y / 0.19;
                let d = (f - f.floor()).min_num(1.0 - (f - f.floor())) * 0.19;
                edge(d, 0.0006, pixel) * 0.08
            } else {
                0.0
            };
            surface.albedo = tint
                * tone
                * (1.0 + grain.x * 0.11 + growth.x * 0.10 + pores.x * 0.04 + rings * 0.025 + knot * 0.04 - seams);
            surface.roughness = (if maple { 0.48 } else { 0.38 }) + grain.x * 0.09 + pores.x * 0.05;
            surface.coat = if boards { 0.22 } else { 0.12 };
            let grain_gradient = grain.yz() + warp.yz() * (3.0 * (grain.y / grain_scale.x + grain.z / grain_scale.y));
            surface.gradient = (growth.yz() * 0.0004 + grain_gradient * 0.0006 + pores.yz() * 0.00015).extend(0.0);
            if maple {
                surface.gradient *= 0.25;
            }
        }
        Material::Carpet | Material::Shag | Material::Fabric | Material::Knit => {
            let shag = kind == Material::Shag;
            let carpet = kind == Material::Carpet || shag;
            let mottling = detail(uv, Vec2::splat(7.0), pixel);
            let pile = if shag {
                let a = vec2(uv.x * 0.8 - uv.y * 0.6, uv.x * 0.6 + uv.y * 0.8);
                let b = vec2(uv.x * 0.38 + uv.y * 0.925, -uv.x * 0.925 + uv.y * 0.38);
                {
                    let first = detail(a, Vec2::splat(42.0), pixel);
                    let second = detail(b, Vec2::splat(60.0), pixel);
                    vec3(first.x, first.y * 0.8 + first.z * 0.6, -first.y * 0.6 + first.z * 0.8) * 0.6
                        + vec3(second.x, second.y * 0.38 - second.z * 0.925, second.y * 0.925 + second.z * 0.38) * 0.4
                }
            } else {
                detail(uv, Vec2::splat(65.0), pixel)
            };
            let fibre = detail(uv, Vec2::splat(230.0), pixel);
            let weave = wave(uv.x * TAU * 750.0, 750.0, pixel) * wave(uv.y * TAU * 680.0, 680.0, pixel);
            let knit = if kind == Material::Knit {
                let loops = vec2(uv.x * 35.0, uv.y * 45.0);
                wave(loops.x * TAU, 35.0, pixel) * wave(loops.y * TAU + loops.x.sin() * 0.8, 45.0, pixel)
            } else {
                0.0
            };
            let base = if carpet { vec3(0.94, 0.91, 0.84) } else { Vec3::ONE };
            surface.albedo = tint
                * base
                * (1.0
                    + mottling.x * 0.04
                    + pile.x * if shag { 0.07 } else { 0.012 }
                    + fibre.x * 0.025
                    + weave * 0.02
                    + knit * 0.055);
            surface.roughness = if carpet { 0.96 } else { 0.84 };
            surface.sheen = if shag { 0.65 } else { 0.3 };
            surface.occlusion = if shag { 0.92 + pile.x * 0.12 } else { 1.0 };
            let mut gradient = pile.yz() * if shag { 0.0008 } else { 0.0001 } + fibre.yz() * 0.000_035;
            let phase = uv * vec2(750.0, 680.0) * TAU;
            let weight = (1.0 - 750.0 * pixel * 2.0).max_num(0.0).min_num(1.0)
                * (1.0 - 680.0 * pixel * 2.0).max_num(0.0).min_num(1.0);
            gradient += vec2(phase.x.cos() * phase.y.sin() * 750.0, phase.x.sin() * phase.y.cos() * 680.0)
                * (TAU * 0.000_025 * weight);
            if kind == Material::Knit {
                let loops = uv * vec2(35.0, 45.0);
                let phase_x = loops.x * TAU;
                let phase_y = loops.y * TAU + loops.x.sin() * 0.8;
                let weight = (1.0 - 35.0 * pixel * 2.0).max_num(0.0).min_num(1.0)
                    * (1.0 - 45.0 * pixel * 2.0).max_num(0.0).min_num(1.0);
                gradient += vec2(
                    35.0 * (TAU * phase_x.cos() * phase_y.sin() + phase_x.sin() * phase_y.cos() * loops.x.cos() * 0.8),
                    45.0 * TAU * phase_x.sin() * phase_y.cos(),
                ) * (0.00075 * weight);
            }
            surface.gradient = gradient.extend(0.0);
        }
        Material::Stone | Material::Tile => {
            let broad = detail(uv, Vec2::splat(4.0), pixel);
            let mineral = detail(uv + Vec2::splat(broad.x * 2.0 / 44.0), Vec2::splat(44.0), pixel);
            let quartz = detail(uv, Vec2::splat(165.0), pixel);
            let grout = if kind == Material::Tile {
                let c = uv / 0.4;
                let f = c - c.floor();
                edge(f.min_num(Vec2::ONE - f).min_element() * 0.4, 0.003, pixel)
            } else {
                0.0
            };
            let base = if kind == Material::Tile { vec3(0.20, 0.22, 0.23) } else { Vec3::ONE };
            surface.albedo = tint * base * (1.0 + broad.x * 0.06 + mineral.x * 0.10 + quartz.x * 0.13);
            surface.albedo = surface.albedo.lerp(vec3(0.30, 0.30, 0.29), grout * 0.5);
            surface.roughness = 0.36 + mineral.x * 0.07 + grout * 0.4;
            surface.coat = 0.06 * (1.0 - grout);
            let mineral_gradient = mineral.yz() + broad.yz() * ((mineral.y + mineral.z) * 2.0 / 44.0);
            surface.gradient = (mineral_gradient * 0.00025).extend(0.0);
        }
        Material::Emissive => {}
        Material::Ceramic => {
            surface.roughness = 0.2;
            surface.coat = 0.45;
        }
        Material::Metal | Material::Mirror => {
            let brushed = detail(uv, vec2(220.0, 4.0), pixel);
            surface.metallic = 0.95;
            surface.roughness = 0.3 + brushed.x * 0.14;
            surface.albedo = tint * (1.0 + brushed.x * 0.04);
            surface.gradient = (brushed.yz() * 0.00015).extend(0.0);
        }
        Material::Paint => {
            surface.roughness = 0.86;
            surface.gradient = (detail(uv, Vec2::splat(120.0), pixel).yz() * 0.00015).extend(0.0);
        }
        Material::Foliage => {
            surface.roughness = 0.48;
            surface.sheen = 0.15;
        }
        Material::Soil => {
            surface.roughness = 1.0;
            surface.albedo = tint * (1.0 + detail(uv, Vec2::splat(65.0), pixel).x * 0.2);
        }
    }

    surface.albedo = linear(surface.albedo);
    surface.roughness = surface.roughness.max_num(0.12).min_num(1.0);
    surface
}
/// Blend projections on bevels instead of abruptly switching axes. Each plane
/// also filters by its projected pixel footprint, including grazing side faces.
#[inline(never)]
pub(super) fn sample(kind: Material, tint: Vec3, p: Vec3, n: Vec3, pixel: f32) -> Surface {
    // Uniform materials do not need three copies of the same projection.
    if kind == Material::Foliage || kind == Material::Ceramic || (kind == Material::Metal && pixel >= 1.0 / 440.0) {
        return sample_plane(kind, tint, p.xy(), pixel);
    }
    let weights = n.abs().powf(4.0);
    let weights = weights / weights.element_sum().max_num(0.0001);
    let mut result = Surface {
        albedo: Vec3::ZERO,
        roughness: 0.0,
        metallic: 0.0,
        sheen: 0.0,
        coat: 0.0,
        gradient: Vec3::ZERO,
        occlusion: 0.0,
    };
    for axis in 0..3 {
        let (uv, weight, facing) = match axis {
            0 => (p.yz(), weights.x, n.x),
            1 => (p.xz(), weights.y, n.y),
            _ => (p.xy(), weights.z, n.z),
        };
        if weight < 0.001 {
            continue;
        }
        let surface = sample_plane(kind, tint, uv, pixel / facing.abs().max_num(0.12));
        result.albedo += surface.albedo * weight;
        result.roughness += surface.roughness * weight;
        result.metallic += surface.metallic * weight;
        result.sheen += surface.sheen * weight;
        result.coat += surface.coat * weight;
        let g = surface.gradient.xy();
        result.gradient += if axis == 0 {
            vec3(0.0, g.x, g.y)
        } else if axis == 1 {
            vec3(g.x, 0.0, g.y)
        } else {
            vec3(g.x, g.y, 0.0)
        } * weight;
        result.occlusion += surface.occlusion * weight;
    }
    result
}
#[inline(never)]
pub(super) fn bump(n: Vec3, gradient: Vec3) -> Vec3 {
    let tangent = gradient - n * gradient.dot(n);
    (n - tangent / (tangent.length() / 0.25).max_num(1.0)).normalize()
}
