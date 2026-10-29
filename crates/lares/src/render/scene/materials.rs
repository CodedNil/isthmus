//! Procedural metallic/roughness materials, evaluated in the surface tangent frame.
use crate::home::Material;
use core::f32::consts::TAU;
use isthmus::prelude::*;

#[derive(Clone, Copy)]
pub(super) struct Surface {
    pub albedo: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    /// Effective coverage of the dielectric specular finish.
    pub specular: f32,
    pub anisotropy: f32,
    pub tangent: Vec3,
    pub sheen: f32,
    pub coat: f32,
    pub coat_roughness: f32,
    pub gradient: Vec3,
    pub occlusion: f32,
    pub fiber_depth: f32,
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            albedo: Vec3::ZERO,
            roughness: 0.0,
            metallic: 0.0,
            specular: 1.0,
            anisotropy: 0.0,
            tangent: Vec3::ZERO,
            sheen: 0.0,
            coat: 0.0,
            coat_roughness: 0.0,
            gradient: Vec3::ZERO,
            occlusion: 0.0,
            fiber_depth: 0.0,
        }
    }
}
pub(super) fn hash(cell: Vec2) -> f32 {
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
    floor_detail(uv, scale, Vec2::splat(pixel))
}
fn floor_detail(uv: Vec2, scale: Vec2, footprint: Vec2) -> Vec3 {
    let weight = (1.0 - (scale * footprint).max_element() * 2.0).max_num(0.0).min_num(1.0);
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
    let weight = (1.0 - frequency * pixel * 2.0).max_num(0.0).min_num(1.0);
    if weight > 0.0 { phase.sin() * weight } else { 0.0 }
}
fn edge(distance: f32, width: f32, pixel: f32) -> f32 {
    1.0 - ((distance - width) / pixel.max_num(0.0001) + 0.5).max_num(0.0).min_num(1.0)
}
fn linear(color: Vec3) -> Vec3 {
    // sRGB albedo inputs; all illumination is evaluated in linear light.
    color.clamp_num(Vec3::ZERO, Vec3::ONE).powf(2.2)
}
/// Jittered mineral cells retain sharp crystal boundaries at close range.
fn mineral_grains(uv: Vec2, pixel: f32) -> Vec3 {
    let scale = 165.0;
    let point = uv * scale;
    let cell = point.floor();
    let mut nearest = 100.0;
    let mut crystal = Vec3::ZERO;
    for neighbour in 0..9 {
        let index = cell + vec2((neighbour % 3) as f32 - 1.0, (neighbour / 3) as f32 - 1.0);
        let seed = hash(index);
        let variation = hash(index + vec2(67.0, 23.0));
        let delta = point - index - vec2(0.1 + seed * 0.8, 0.1 + variation * 0.8);
        let distance = delta.length_squared();
        if distance < nearest {
            nearest = distance;
            let tone = if variation > 0.88 {
                0.16 + seed * 0.10
            } else if variation > 0.55 {
                0.015 + seed * 0.035
            } else {
                seed * 0.006
            };
            crystal = vec3(tone, (seed - 0.5) * 0.025, (variation - 0.5) * 0.025);
        }
    }
    let filter = 1.0 / (1.0 + (pixel * scale * 1.5).powi(4));
    vec3(0.044, 0.0, 0.0).lerp(crystal, filter)
}
/// Board-local oak growth, pores, finish wear and a shallow machined bevel.
/// Height derivatives perturb the true SDF normal only after the ray hits.
fn floorboards(tint: Vec3, uv: Vec2, footprint: Vec2) -> Surface {
    let pixel = footprint.max_element();
    let width = 0.185;
    let length = 1.45;
    let row = (uv.y / width).floor();
    let offset = hash(vec2(row, 7.0)) * length;
    let column = ((uv.x + offset) / length).floor();
    let board = vec2(column, row);
    let seed = hash(board);
    let local = vec2(uv.x + offset - column * length, uv.y - row * width);
    let drift = floor_detail(local + Vec2::splat(seed * 19.0), vec2(1.1, 7.0), footprint);
    // Off-centre growth cylinders make arching/cathedral grain rather than
    // equally spaced printed stripes. Each plank cuts a different part.
    let radial_point = local - vec2(length * (0.25 + seed * 0.6), -0.08 - seed * 0.20);
    let radial = (radial_point * vec2(0.09, 1.0)).length().max_num(0.001);
    let frequency = 30.0 + seed * 13.0;
    let ring_phase = radial * TAU * frequency + drift.x * 3.0;
    let phase = ring_phase + (ring_phase * 0.37 + seed * TAU).sin() * 0.65;
    let growth = wave(phase, frequency * 1.25, footprint.y);
    let fibres = floor_detail(local + vec2(seed * 31.0, drift.x * 0.012), vec2(2.0, 78.0), footprint);
    let pores = floor_detail(local + seed * 13.0, vec2(9.0, 240.0), footprint);
    let broad = floor_detail(local + seed * 37.0, vec2(2.5, 13.0), footprint);
    let mut knot = 0.0;
    let mut knot_rings = 0.0;
    for index in 0..2 {
        let identity = board + vec2(index as f32 * 83.0, 29.0);
        let chance = hash(identity);
        if chance < if index == 0 { 0.54 } else { 0.90 } {
            continue;
        }
        let center = vec2(0.12 + hash(identity + 3.0) * 1.20, width * (0.15 + hash(identity + 9.0) * 0.70));
        let delta = local - center;
        let turn = (hash(identity + 15.0) - 0.5) * 0.42;
        let (sin, cos) = turn.sin_cos();
        let axes = vec2(10.0 + hash(identity + 21.0) * 14.0, 38.0 + hash(identity + 25.0) * 45.0);
        let q = vec2(delta.x * cos - delta.y * sin, delta.x * sin + delta.y * cos) * axes;
        let radius = q.length();
        if radius < 2.0 {
            let angle = q.y.atan2(q.x);
            let irregular = detail(local + chance * 19.0, vec2(57.0, 93.0), pixel);
            let warped = radius
                + (angle * 3.0 + chance * TAU).sin() * 0.07
                + (angle * 5.0 + radius * 2.0).sin() * 0.035
                + irregular.x * 0.18;
            let envelope = (1.0 - radius * 0.5).max_num(0.0).powi(2);
            let core = edge(warped, 0.18 + hash(identity + 31.0) * 0.19, pixel * axes.max_element());
            let phase = warped * (24.0 + chance * 24.0) + angle.sin() * 1.4 + irregular.x * 3.0;
            let rings = wave(phase, axes.max_element() * 7.0, pixel);
            let fissure =
                edge((q.y + (q.x * 3.0).sin() * 0.08).abs(), 0.035, pixel * axes.y) * (1.0 - radius).max_num(0.0);
            knot += core * 0.65 + fissure * 0.25;
            knot_rings += rings * envelope;
        }
    }
    let distance = local.min(vec2(length, width) - local);
    let seam = edge(distance.x, 0.00055, footprint.x).max_num(edge(distance.y, 0.00055, footprint.y));
    let bevel_width = 0.0022;
    // Convolve a shallow bevel with the pixel footprint instead of dropping
    // it once the pixel grows beyond the bevel. Edge highlights stay local.
    let filtered_width = Vec2::splat(bevel_width) + footprint * 0.5;
    let bevel = (Vec2::ONE - distance / filtered_width).max(Vec2::ZERO) * bevel_width / filtered_width
        * (Vec2::ONE - footprint / 0.05).clamp_num(Vec2::ZERO, Vec2::ONE);
    let bevel_sign = Vec2::select(local.cmpgt(vec2(length, width) * 0.5), -Vec2::ONE, Vec2::ONE);
    let relief = floor_detail(local + seed * 53.0, vec2(6.0, 32.0), footprint);
    let finish = 0.5 + fibres.x * 0.5 + broad.x * 0.35;
    let tone = vec3(0.36, 0.29, 0.225).lerp(vec3(0.53, 0.435, 0.34), seed * 0.70 + 0.10);
    let albedo = tint
        * tone
        * (1.0 + broad.x * 0.22 + fibres.x * 0.23 + growth * 0.035 + pores.x * 0.08 - knot * 0.48 + knot_rings * 0.13
            - seam * 0.30);
    let radial_gradient = radial_point * vec2(0.0081, 1.0) / radial;
    let phase_gradient = (radial_gradient * (TAU * frequency) + drift.yz() * 3.0)
        * (1.0 + (ring_phase * 0.37 + seed * TAU).cos() * 0.2405);
    let ring_gradient = phase_gradient * phase.cos() * (1.0 - frequency * 1.25 * footprint.y * 2.0).clamp_num(0.0, 1.0);
    let gradient = fibres.yz() * 0.00095
        + pores.yz() * 0.00024
        + ring_gradient * 0.00065
        + broad.yz() * 0.0015
        + relief.yz() * 0.0018
        + bevel_sign * bevel * 0.50;
    // Unresolved pores become microsurface roughness rather than disappearing
    // and leaving an unnaturally perfect mirror at distant viewing angles.
    Surface {
        albedo,
        roughness: (0.59 + (seed - 0.5) * 0.10 + broad.x * 0.10 + fibres.x * 0.14 + pores.x * 0.08 + seam * 0.10
            - bevel.max_element() * 0.18)
            .clamp_num(0.37, 0.80),
        specular: 0.32 + bevel.max_element() * 0.38,
        coat: 0.012 + finish * 0.008,
        anisotropy: 0.16,
        tangent: Vec3::X,
        coat_roughness: 0.60 + broad.x * 0.10 + pores.x * 0.08,
        gradient: gradient.extend(0.0),
        occlusion: 1.0 - seam * 0.12,
        ..Surface::default()
    }
}
#[inline(never)]
fn sample_plane(kind: Material, tint: Vec3, uv: Vec2, pixel: f32) -> Surface {
    let mut surface =
        Surface { albedo: tint, roughness: 0.65, coat_roughness: 0.35, occlusion: 1.0, ..Surface::default() };
    match kind {
        Material::Floorboards => {
            surface = floorboards(tint, uv, Vec2::splat(pixel));
        }
        Material::Wood | Material::Maple | Material::MapleHorizontal => {
            let maple = matches!(kind, Material::Maple | Material::MapleHorizontal);
            let uv = if kind == Material::MapleHorizontal { uv.yx() } else { uv };
            let warp = detail(uv, vec2(1.8, 6.0), pixel);
            let scale = vec2(65.0, 2.0);
            let grain = detail(uv + Vec2::splat(warp.x * 2.0) / scale, scale, pixel);
            let pores = detail(uv, vec2(210.0, 9.0), pixel);
            let growth = detail(uv, vec2(13.0, 1.2), pixel);
            let tone = if maple { vec3(0.84, 0.79, 0.69) } else { vec3(0.82, 0.75, 0.64) };
            surface.albedo =
                tint * tone * (1.0 + grain.x * if maple { 0.06 } else { 0.20 } + growth.x * 0.09 + pores.x * 0.06);
            surface.roughness = (if maple { 0.42 } else { 0.48 }) + grain.x * 0.10 + growth.x * 0.06;
            surface.coat = if maple { 0.15 } else { 0.10 };
            surface.coat_roughness = if maple { 0.36 } else { 0.43 };
            let gradient = grain.yz() + warp.yz() * ((grain.y / scale.x + grain.z / scale.y) * 2.0);
            surface.gradient = ((gradient * 0.0006 + growth.yz() * 0.0004 + pores.yz() * 0.00012)
                * if maple { 0.25 } else { 1.0 })
            .extend(0.0);
        }
        Material::Shag => return shag(tint, uv.extend(0.0), Vec3::Z, Vec3::Z, pixel),
        Material::Carpet | Material::Fabric | Material::Satin | Material::Knit => {
            let carpet = kind == Material::Carpet;
            let mottling = detail(uv, Vec2::splat(7.0), pixel);
            let pile = detail(uv, Vec2::splat(65.0), pixel);
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
                * (1.0 + mottling.x * 0.04 + pile.x * 0.035 + fibre.x * 0.025 + weave * 0.02 + knit * 0.055);
            surface.roughness = (if carpet {
                0.94
            } else if kind == Material::Satin {
                0.40
            } else {
                0.84
            }) + pile.x * 0.06
                + mottling.x * 0.035;
            surface.sheen = if carpet { 0.35 } else { 0.5 };
            surface.anisotropy = if kind == Material::Satin { 0.45 } else { 0.0 };
            let mut gradient = pile.yz() * 0.0001 + fibre.yz() * 0.000_035;
            let phase = uv * vec2(750.0, 680.0) * TAU;
            let weight = (1.0 - 750.0 * pixel * 2.0).max_num(0.0).min_num(1.0)
                * (1.0 - 680.0 * pixel * 2.0).max_num(0.0).min_num(1.0);
            if weight > 0.0 {
                gradient += vec2(phase.x.cos() * phase.y.sin() * 750.0, phase.x.sin() * phase.y.cos() * 680.0)
                    * (TAU * 0.000_025 * weight);
            }
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
        Material::Stone => {
            // Interlocking feldspar/quartz grains, dark mica inclusions and
            // a finer polished crystal population; never a blurred grey wash.
            let facets = mineral_grains(uv, pixel);
            let cloud = detail(uv, Vec2::splat(3.5), pixel);
            let aggregate = detail(uv + cloud.x * 0.018, vec2(27.0, 39.0), pixel);
            let grains = detail(uv + cloud.x * 0.012, vec2(100.0, 135.0), pixel);
            let crystals = detail(uv + vec2(17.3, 9.1), vec2(410.0, 360.0), pixel);
            let mica = detail(uv + 31.7, Vec2::splat(680.0), pixel);
            let quartz = ((aggregate.x * 0.80 + grains.x + crystals.x * 0.45 - 0.01) * 4.0).clamp_num(0.0, 1.0);
            let dark = ((-grains.x - 0.08) * 4.0).clamp_num(0.0, 1.0);
            surface.albedo = tint * (0.82 + cloud.x * 0.25 + aggregate.x * 0.65 - dark * 0.35)
                + vec3(0.21, 0.205, 0.19) * quartz
                + Vec3::splat(mica.x.max_num(0.0) * 0.10 + facets.x);
            surface.roughness =
                (0.18 + aggregate.x * 0.15 + grains.x * 0.10 - quartz * 0.04 + dark * 0.05).clamp_num(0.10, 0.30);
            surface.coat = 0.48;
            surface.coat_roughness = 0.12 + aggregate.x * 0.08;
            surface.gradient =
                (grains.yz() * 0.000_035 + crystals.yz() * 0.000_015 + mica.yz() * 0.000_004 + facets.yz()).extend(0.0);
        }
        Material::Tile => {
            let mineral = detail(uv, Vec2::splat(44.0), pixel);
            let c = uv / 0.4;
            let f = c - c.floor();
            let grout = edge(f.min_num(Vec2::ONE - f).min_element() * 0.4, 0.003, pixel);
            surface.albedo =
                (tint * vec3(0.20, 0.22, 0.23) * (1.0 + mineral.x * 0.10)).lerp(vec3(0.30, 0.30, 0.29), grout * 0.5);
            surface.roughness = 0.42 + mineral.x * 0.14 + grout * 0.4;
            surface.coat = 0.06 * (1.0 - grout);
            surface.gradient = (mineral.yz() * 0.00032).extend(0.0);
        }
        Material::Emissive => {}
        Material::GlazedTile | Material::BathroomTile => {
            let uneven = detail(uv, vec2(13.0, 17.0), pixel);
            let ripple = detail(uv + uneven.x * 0.004, vec2(41.0, 53.0), pixel);
            let glaze = detail(uv, Vec2::splat(135.0), pixel);
            surface.albedo = tint * (1.0 + uneven.x * 0.035 + ripple.x * 0.018);
            surface.roughness = (0.17 + uneven.x * 0.08 + ripple.x * 0.05).clamp_num(0.10, 0.27);
            surface.coat = 0.68;
            surface.coat_roughness = 0.11 + glaze.x * 0.05;
            surface.gradient = (uneven.yz() * 0.0011 + ripple.yz() * 0.00035 + glaze.yz() * 0.00004).extend(0.0);
            if kind == Material::BathroomTile {
                let pitch = vec2(0.25, 0.33);
                let phase = uv / pitch - (uv / pitch).floor();
                let distance = phase.min_num(Vec2::ONE - phase) * pitch;
                let grout = edge(distance.min_element(), 0.0014, pixel);
                surface.albedo = surface.albedo.lerp(vec3(0.64, 0.64, 0.62), grout);
                surface.roughness = surface.roughness * (1.0 - grout) + 0.85 * grout;
                surface.coat *= 1.0 - grout;
                let bevel = (Vec2::ONE - distance / (0.004 + pixel)).max_num(Vec2::ZERO);
                let sign = Vec2::select(phase.cmplt(Vec2::splat(0.5)), Vec2::ONE, -Vec2::ONE);
                surface.gradient += (bevel * sign * 0.13 * (1.0 - grout)).extend(0.0);
            }
        }
        Material::Ceramic => {
            let glaze = detail(uv, vec2(38.0, 38.0), pixel);
            surface.roughness = 0.20 + glaze.x * 0.06;
            surface.coat = 0.30;
            surface.coat_roughness = 0.19;
            surface.gradient = (glaze.yz() * 0.00004).extend(0.0);
        }
        Material::Metal | Material::Mirror => {
            let brushed = detail(uv, vec2(220.0, 4.0), pixel);
            surface.metallic = 0.95;
            surface.anisotropy = if kind == Material::Mirror { 0.0 } else { 0.48 };
            surface.tangent = Vec3::Y;
            surface.roughness = 0.3 + brushed.x * 0.14;
            surface.albedo = tint * (1.0 + brushed.x * 0.04);
            surface.gradient = (brushed.yz() * 0.00015).extend(0.0);
        }
        Material::Paint => {
            let plaster = detail(uv, Vec2::splat(9.0), pixel);
            surface.roughness = 0.80 + plaster.x * 0.07;
            surface.albedo = tint * (1.0 + plaster.x * 0.012);
            surface.gradient = (detail(uv, Vec2::splat(120.0), pixel).yz() * 0.00015).extend(0.0);
        }
        Material::Foliage | Material::VariegatedFoliage | Material::MarbledFoliage => {
            let cuticle = detail(uv, vec2(70.0, 140.0), pixel);
            surface.roughness = 0.43 + cuticle.x * 0.12;
            surface.gradient = (cuticle.yz() * 0.00006).extend(0.0);
            surface.sheen = 0.15;
        }
        Material::Soil => {
            surface.roughness = 1.0;
            surface.gradient = (detail(uv, Vec2::splat(90.0), pixel).yz() * 0.0004).extend(0.0);
            surface.albedo = tint * (1.0 + detail(uv, Vec2::splat(65.0), pixel).x * 0.2);
        }
    }

    surface.albedo = linear(surface.albedo);
    surface.roughness = surface.roughness.max_num(0.12).min_num(1.0);
    surface
}
/// Dense cut pile: resolvable yarn ridges shade individually; smaller fibres
/// fade into a rough, occluded microsurface instead of a painted colour noise.
fn shag(tint: Vec3, point: Vec3, normal: Vec3, view: Vec3, cone: f32) -> Surface {
    let pixel = cone / normal.dot(view).abs().max_num(0.30);
    let uv = point.xy();
    let nap = detail(uv, Vec2::splat(3.0), pixel);
    let flow = vec2(uv.x * 0.8 - uv.y * 0.6, uv.x * 0.6 + uv.y * 0.8);
    let spacing = vec2(0.014, 0.018);
    let cell = (flow / spacing).floor();
    let mut ridge = Vec3::ZERO;
    // Overlapping yarn bundles, jittered in two dimensions across the whole
    // rug. Each has its own lean; roots do not form a regular fringe/grid.
    for neighbour in 0..9 {
        let index = cell + vec2((neighbour % 3) as f32 - 1.0, (neighbour / 3) as f32 - 1.0);
        let seed = hash(index);
        let variation = hash(index + vec2(37.0, 81.0));
        let center = (index + vec2(0.2 + seed * 0.6, 0.2 + variation * 0.6)) * spacing;
        let lean = (variation - 0.5) * 1.2;
        let direction = vec2(1.0, lean).normalize();
        let delta = flow - center;
        let local = vec2(delta.dot(direction), delta.dot(vec2(-direction.y, direction.x)));
        let inverse = vec2(1.0 / (0.0048 + seed * 0.0021), 1.0 / (0.010 + variation * 0.005));
        let radial = local * inverse;
        let profile = (-radial.length_squared() * 1.6).exp();
        if profile > ridge.x {
            let slope = local * inverse * inverse * (-3.2 * profile * 0.0010);
            let gradient = direction * slope.x + vec2(-direction.y, direction.x) * slope.y;
            ridge = vec3(profile, gradient.x, gradient.y);
        }
    }
    let filtered = 1.0 / (1.0 + (pixel / 0.007).powi(4));
    let gradient = ridge.yz() * filtered;
    let gradient = vec2(gradient.x * 0.8 + gradient.y * 0.6, -gradient.x * 0.6 + gradient.y * 0.8);
    let texture_depth = 0.55 + (0.40 - ridge.x * 0.8) * filtered;
    let pile_depth = ((0.040 - point.z) / 0.035).clamp_num(0.05, 0.95);
    let valley = pile_depth * 0.7 + texture_depth * 0.3;
    Surface {
        albedo: linear(tint * vec3(0.93, 0.92, 0.88) * (1.0 + nap.x * 0.065)),
        roughness: 0.94,
        sheen: 0.85,
        coat_roughness: 1.0,
        gradient: (gradient * 0.5 + nap.yz() * 0.003).extend(0.0),
        tangent: normal,
        fiber_depth: valley,
        occlusion: 1.0 - valley * 0.12,
        ..Surface::default()
    }
}
/// Blend projections on bevels instead of abruptly switching axes. Each plane
/// also filters by its projected pixel footprint, including grazing side faces.
#[inline(never)]
pub(super) fn sample(kind: Material, tint: Vec3, p: Vec3, n: Vec3, view: Vec3, cone: f32) -> Surface {
    if kind == Material::Shag {
        return shag(tint, p, n, view, cone);
    }
    let incidence = n.dot(view).abs().max_num(0.10);
    if kind == Material::Floorboards && n.z.abs() > 0.999 {
        // The long axis of the grazing ray footprint follows the viewing
        // direction. Do not blur cross-grain with that much larger footprint.
        let tangent_view = view - n * n.dot(view);
        let footprint = (Vec2::ONE + tangent_view.xy() * tangent_view.xy() / (incidence * incidence)).sqrt() * cone;
        let mut surface = floorboards(tint, p.xy(), footprint);
        surface.albedo = linear(surface.albedo);
        return surface;
    }
    let pixel = cone / incidence;
    // Uniform materials do not need three copies of the same projection.
    if matches!(kind, Material::Foliage | Material::VariegatedFoliage | Material::MarbledFoliage)
        || kind == Material::Ceramic
        || (kind == Material::Metal && pixel >= 1.0 / 440.0)
    {
        return sample_plane(kind, tint, p.xy(), pixel);
    }
    let square = n * n;
    let weights = square * square;
    let weights = weights / weights.element_sum().max_num(0.0001);
    let mut result = Surface { specular: 0.0, ..Surface::default() };
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
        result.specular += surface.specular * weight;
        result.anisotropy += surface.anisotropy * weight;
        result.sheen += surface.sheen * weight;
        result.coat += surface.coat * weight;
        result.coat_roughness += surface.coat_roughness * weight;
        // Plane gradients are tangent to their projection, so z is always zero.
        let g = surface.gradient;
        let projected_tangent = match axis {
            0 => surface.tangent.zxy(),
            1 => surface.tangent.xzy(),
            _ => surface.tangent,
        } * weight;
        let projected_gradient = match axis {
            0 => g.zxy(),
            1 => g.xzy(),
            _ => g,
        } * weight;
        result.tangent += projected_tangent;
        result.gradient += projected_gradient;
        result.occlusion += surface.occlusion * weight;
    }
    result
}
#[inline(never)]
pub(super) fn bump(n: Vec3, gradient: Vec3, limit: f32) -> Vec3 {
    let tangent = gradient - n * gradient.dot(n);
    (n - tangent / (tangent.length() / limit).max_num(1.0)).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materials_remain_finite_across_projection_and_filter_scales() {
        for material in [
            Material::Floorboards,
            Material::Maple,
            Material::MapleHorizontal,
            Material::Fabric,
            Material::Satin,
            Material::Knit,
            Material::Shag,
            Material::Carpet,
            Material::Stone,
            Material::Tile,
            Material::Ceramic,
            Material::GlazedTile,
            Material::BathroomTile,
            Material::Metal,
            Material::Paint,
            Material::Foliage,
            Material::VariegatedFoliage,
            Material::MarbledFoliage,
            Material::Soil,
            Material::Emissive,
            Material::Mirror,
            Material::Wood,
        ] {
            for normal in [Vec3::X, -Vec3::Y, Vec3::Z, vec3(1.0, -1.0, 1.0).normalize()] {
                for pixel in [0.0001, 0.005, 0.1] {
                    for point in [Vec3::ZERO, vec3(-1.65, -0.19, 0.4), vec3(12.5, 0.001, -2.0)] {
                        let surface = sample(material, Vec3::ONE, point, normal, normal, pixel);
                        assert!(surface.albedo.is_finite() && surface.gradient.is_finite());
                        assert!((0.12..=1.0).contains(&surface.roughness));
                        assert!((0.0..=1.0).contains(&surface.specular));
                        assert!(surface.albedo.cmpge(Vec3::ZERO).all() && surface.albedo.cmple(Vec3::ONE).all());
                    }
                }
            }
        }
        let distant = sample(Material::Floorboards, Vec3::ONE, vec3(-1.65, -0.19, 0.0), Vec3::Z, Vec3::Z, 0.1);
        assert_eq!(distant.gradient, Vec3::ZERO, "subpixel grain and bevels must filter away");
    }
    #[test]
    fn grazing_floor_filter_preserves_resolvable_cross_grain_without_distant_shimmer() {
        let along = vec3(0.995, 0.0, 0.1).normalize();
        let across = vec3(0.0, 0.995, 0.1).normalize();
        let mut along_variation = 0.0;
        let mut across_variation = 0.0;
        for index in 0..128 {
            let point = vec3(index as f32 * 0.073, index as f32 * 0.017, 0.0);
            let a = sample(Material::Floorboards, Vec3::ONE, point, Vec3::Z, along, 0.003);
            let b = sample(Material::Floorboards, Vec3::ONE, point, Vec3::Z, across, 0.003);
            along_variation += a.gradient.y.abs();
            across_variation += b.gradient.y.abs();
            let distant = sample(Material::Floorboards, Vec3::ONE, point, Vec3::Z, along, 0.2);
            assert_eq!(distant.gradient, Vec3::ZERO);
        }
        assert!(
            along_variation > across_variation * 1.5,
            "grazing blur must follow the view, not erase transverse grain"
        );
    }
}
