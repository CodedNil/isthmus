//! Static SDF depth with receiver-plane-corrected percentage-closer filtering.
use super::{acceleration::Node, scene};
use crate::home::{Kind, Object};
use core::f32::consts::FRAC_1_SQRT_2;
#[cfg(not(target_arch = "spirv"))]
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use isthmus::prelude::*;
#[cfg(not(target_arch = "spirv"))]
use std::io;

pub const RESOLUTION: u32 = 512;
const PADDING: u32 = 2;
const COLUMNS: u32 = 8;
const RANGE: f32 = super::photometry::LIGHT_RANGE;

#[derive(Clone, Copy, ShaderData)]
pub struct AtlasInfo {
    pub size: Vec2,
    pub resolution: u32,
    pub columns: u32,
}

impl AtlasInfo {
    pub fn new(lights: u32, resolution: u32) -> Self {
        let pitch = resolution + PADDING * 2;
        let rows = lights.max(1).div_ceil(COLUMNS);
        Self { size: vec2((COLUMNS * pitch) as f32, (rows * pitch) as f32), resolution, columns: COLUMNS }
    }

    const fn pitch(self) -> u32 {
        self.resolution + PADDING * 2
    }

    pub const fn origin(self, tile: u32) -> Vec2 {
        vec2((tile % self.columns * self.pitch()) as f32, (tile / self.columns * self.pitch()) as f32)
    }
}

fn sign(v: Vec2) -> Vec2 {
    Vec2::select(v.cmpge(Vec2::ZERO), Vec2::ONE, -Vec2::ONE)
}

/// Full-sphere octahedral parameterization; the centre faces downwards.
fn decode(uv: Vec2) -> Vec3 {
    let p = uv * 2.0 - 1.0;
    let z = 1.0 - p.abs().element_sum();
    let xy = if z < 0.0 { (Vec2::ONE - p.yx().abs()) * sign(p) } else { p };
    vec3(xy.x, xy.y, -z).normalize()
}
fn encode(direction: Vec3) -> Vec2 {
    let n = vec3(direction.x, direction.y, -direction.z) / direction.abs().element_sum();
    let xy = if n.z < 0.0 { (Vec2::ONE - n.yx().abs()) * sign(n.xy()) } else { n.xy() };
    xy * 0.5 + 0.5
}

fn pack(value: f32) -> Vec2 {
    let bits = (value * 65535.0).clamp_num(0.0, 65535.0).round();
    let high = (bits / 256.0).floor();
    vec2(high, bits - high * 256.0) / 255.0
}
fn unpack(value: Vec2) -> f32 {
    value.dot(vec2(65280.0, 255.0)) / 65535.0
}
fn direction(light: Object, uv: Vec2) -> Vec3 {
    if light.kind != Kind::Downlight {
        return decode(uv);
    }
    // Rotate and expand the octahedron's downward diamond into a full square.
    // Every texel covers the emitting hemisphere, including the tile corners.
    let q = uv * 2.0 - 1.0;
    let p = vec2(q.x - q.y, q.x + q.y) * 0.5;
    p.extend(p.abs().element_sum() - 1.0).normalize()
}
fn coordinates(light: Object, delta: Vec3) -> Vec2 {
    if light.kind != Kind::Downlight {
        return encode(delta);
    }
    let p = delta.xy() / delta.abs().element_sum().max_num(0.001);
    vec2(p.x + p.y, p.y - p.x) * 0.5 + 0.5
}

pub fn depth_pixel(nodes: &[Node], objects: &[Object], light: Object, uv: Vec2) -> Vec4 {
    let direction = direction(light, uv.clamp_num(Vec2::ZERO, Vec2::ONE));
    let distance = if light.kind == Kind::Downlight && direction.z >= 0.0 {
        RANGE
    } else {
        scene::light_depth(scene::fixtures::source(light), direction, nodes, objects, RANGE)
    };
    let d = distance / RANGE;
    pack(d).extend(0.0).extend(1.0)
}

// Compare each unfiltered depth against the receiver plane before interpolating
// visibility. Averaging depths/moments first invents light behind thin blockers.
fn gather(atlas: &Image, info: AtlasInfo, slot: u32, light: Object, delta: Vec3, normal: Vec3, uv: Vec2) -> Vec2 {
    let texel = uv * info.resolution as f32 - 0.5;
    let base = texel.floor();
    let fraction = texel - base;
    let distance = delta.length();
    let mut visibility = 0.0;
    let mut blocker = 0.0;
    let mut occlusion = 0.0;
    for tap in 0..4 {
        let step = vec2((tap & 1) as f32, (tap >> 1) as f32);
        let pixel = (base + step).clamp_num(Vec2::ZERO, Vec2::splat(info.resolution as f32 - 1.0));
        let depth = unpack(atlas.texel((info.origin(slot) + PADDING as f32 + pixel).as_ivec2()).xy()) * RANGE;
        let ray = direction(light, (pixel + 0.5) / info.resolution as f32);
        let denominator = normal.dot(ray);
        let receiver = if denominator.abs() > 0.02 { (normal.dot(delta) / denominator).max_num(0.0) } else { distance };
        let receiver = receiver.min_num(distance * 1.1).max_num(distance * 0.9);
        let w = Vec2::ONE - (step - fraction).abs();
        let weight = w.x * w.y;
        if depth + 0.0015 + distance / 65535.0 >= receiver {
            visibility += weight;
        } else {
            blocker += depth * weight;
            occlusion += weight;
        }
    }
    vec2(visibility, if occlusion > 0.0001 { blocker / occlusion } else { distance })
}

pub fn visible(atlas: &Image, info: AtlasInfo, slot: u32, light: Object, point: Vec3, normal: Vec3) -> f32 {
    let delta = point - scene::fixtures::source(light);
    if light.kind == Kind::Downlight && delta.z >= 0.0 {
        return 0.0;
    }
    let uv = coordinates(light, delta);
    let first = gather(atlas, info, slot, light, delta, normal, uv);
    let distance = delta.length().max_num(0.001);
    let radius = scene::fixtures::radius(light);
    let scale = if light.kind == Kind::Downlight { FRAC_1_SQRT_2 } else { 0.5 };
    // Search on both sides of the silhouette. A lit centre must also receive
    // the penumbra; filtering only shadowed centres clips half of every edge.
    let search = (radius * info.resolution as f32 * scale / distance).clamp_num(1.0, 8.0);
    let mut blocker = first.y * (1.0 - first.x);
    let mut occlusion = 1.0 - first.x;
    for tap in 0..4 {
        let offset = vec2((tap & 1) as f32 * 2.0 - 1.0, (tap >> 1) as f32 * 2.0 - 1.0)
            * (search * 0.707 / info.resolution as f32);
        let result = gather(atlas, info, slot, light, delta, normal, uv + offset);
        blocker += result.y * (1.0 - result.x);
        occlusion += 1.0 - result.x;
    }
    if occlusion < 0.0001 || occlusion > 4.9999 {
        return first.x;
    }
    let depth = blocker / occlusion;
    let spread = (radius * (distance - depth).max_num(0.0) / depth.max_num(0.01) * info.resolution as f32 * scale
        / distance)
        .min_num(8.0);
    if spread <= 0.5 {
        return first.x;
    }
    // A filled disc, evaluated in every frame, rather than four displaced
    // silhouettes or a different emitter sample for each accumulation frame.
    let mut sum = 0.0;
    for tap in 0..16 {
        let r = ((tap as f32 + 0.5) / 16.0).sqrt();
        let angle = tap as f32 * 2.399_963_1;
        let offset = vec2(angle.cos(), angle.sin()) * (r * spread / info.resolution as f32);
        sum += gather(atlas, info, slot, light, delta, normal, uv + offset).x;
    }
    first.x + (sum / 16.0 - first.x) * ((spread - 0.5) * 2.0).min_num(1.0)
}

#[cfg(not(target_arch = "spirv"))]
pub struct ShadowAtlas {
    pub info: AtlasInfo,
    pub image: Image,
}

#[cfg(not(target_arch = "spirv"))]
impl ShadowAtlas {
    pub fn encode(&self, scene: &super::Scene) -> Result<Vec<u8>, io::Error> {
        use std::io::Write;
        let mut output = GzEncoder::new(Vec::new(), Compression::best());
        output.write_all(b"LARESV03")?;
        output.write_all(&scene.signature().to_le_bytes())?;
        output.write_all(&self.info.resolution.to_le_bytes())?;
        let size = self.image.size();
        let mut pixels = self.image.pixels().expect("CPU visibility pixels").to_vec();
        // Lossless row prediction, as in PNG's Sub filter. Packed depth moments
        // are smooth spatially but their raw low bytes compress poorly.
        for row in pixels.chunks_exact_mut(size[0] as usize * 4) {
            for index in (4..row.len()).rev() {
                row[index] = row[index].wrapping_sub(row[index - 4]);
            }
        }
        output.write_all(&pixels)?;
        output.finish()
    }

    pub fn decode(scene: &super::Scene, bytes: &[u8]) -> Result<Self, &'static str> {
        use std::io::Read;
        let mut decoder = GzDecoder::new(bytes);
        let mut header = [0; 20];
        decoder.read_exact(&mut header).map_err(|_error| "invalid visibility asset")?;
        if &header[..8] != b"LARESV03" || header[8..16] != scene.signature().to_le_bytes() {
            return Err("visibility asset is stale; rebuild with just lares");
        }
        let resolution = u32::from_le_bytes(header[16..20].try_into().map_err(|_error| "invalid resolution")?);
        if resolution != RESOLUTION {
            return Err("visibility resolution differs; rebuild with just lares");
        }
        let info = AtlasInfo::new(scene.lights.len() as u32, resolution);
        let size = info.size.to_array().map(|value| value as u32);
        let mut pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        decoder.read_exact(&mut pixels).map_err(|_error| "truncated visibility asset")?;
        let mut tail = [0];
        if decoder.read(&mut tail).map_err(|_error| "corrupt visibility asset")? != 0 {
            return Err("oversized visibility asset");
        }
        for row in pixels.chunks_exact_mut(size[0] as usize * 4) {
            for index in 4..row.len() {
                row[index] = row[index].wrapping_add(row[index - 4]);
            }
        }
        Ok(Self { info, image: Image::raw_rgba8(size, pixels) })
    }

    /// CPU equivalent of the GPU bake, used for reproducible image comparisons.
    pub fn bake_cpu(scene: &super::Scene, resolution: u32) -> Self {
        use std::thread;
        let info = AtlasInfo::new(scene.lights.len() as u32, resolution);
        let [width, height] = info.size.to_array().map(|v| v as usize);
        let mut pixels = vec![255u8; width * height * 4];
        let rows = height.div_ceil(thread::available_parallelism().map_or(1, usize::from).min(8));
        thread::scope(|scope| {
            for (chunk, bytes) in pixels.chunks_mut(rows * width * 4).enumerate() {
                scope.spawn(move || {
                    for (index, rgba) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let x = index % width;
                        let y = chunk * rows + index / width;
                        let pitch = info.pitch() as usize;
                        let tile = y / pitch * info.columns as usize + x / pitch;
                        if tile >= scene.lights.len() {
                            continue;
                        }
                        let uv =
                            (vec2((x % pitch) as f32, (y % pitch) as f32) + 0.5 - PADDING as f32) / resolution as f32;
                        let color =
                            depth_pixel(&scene.nodes, &scene.objects, scene.objects[scene.lights[tile] as usize], uv);
                        *rgba = color.to_array().map(|v| (v * 255.0).round() as u8);
                    }
                });
            }
        });
        let image = Image::raw_rgba8([width as u32, height as u32], pixels);
        Self { info, image }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::Material::Paint;
    #[test]
    fn octahedral_mapping_roundtrips_both_hemispheres() {
        for y in 0..31 {
            for x in 0..31 {
                let uv = vec2((x as f32 + 0.5) / 31.0, (y as f32 + 0.5) / 31.0);
                let direction = decode(uv);
                assert!((decode(encode(direction)) - direction).length() < 0.00001);
            }
        }
    }

    #[test]
    fn packed_depth_roundtrips_without_filtering() {
        for value in [0.0, 0.1, 0.25, 0.5, 0.75, 1.0] {
            assert!((unpack(pack(value)) - value).abs() < 1.0 / 65535.0);
        }
    }
    #[test]
    fn depth_discontinuities_filter_visibility_without_inventing_transmitted_light() {
        let light = Object::new(Kind::Downlight, Vec3::ZERO, vec3(0.11, 0.11, 0.032), Paint);
        let info = AtlasInfo::new(1, 16);
        let [width, height] = info.size.to_array().map(|v| v as usize);
        let mut pixels = vec![255; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let depth = if x < PADDING as usize + 8 { 1.0 } else { 5.0 };
                let packed = pack(depth / RANGE);
                let index = (y * width + x) * 4;
                pixels[index] = (packed.x * 255.0).round() as u8;
                pixels[index + 1] = (packed.y * 255.0).round() as u8;
            }
        }
        let atlas = Image::raw_rgba8([width as u32, height as u32], pixels);
        for x in [0.46, 0.48, 0.5, 0.52, 0.54] {
            let uv = vec2(x, 0.5);
            let delta = direction(light, uv) * 3.0;
            let actual = gather(&atlas, info, 0, light, delta, -delta.normalize(), uv).x;
            let expected = (x * 16.0 - 7.5).clamp(0.0, 1.0);
            assert!(
                (actual - expected).abs() < 0.00001,
                "visibility must be the fraction of unblocked taps: {actual} vs {expected}"
            );
        }
    }
    #[test]
    fn downward_projection_roundtrips_and_allocates_no_texels_to_the_upper_hemisphere() {
        let light = Object::new(Kind::Downlight, Vec3::ZERO, Vec3::ONE, Paint);
        for y in 0..31 {
            for x in 0..31 {
                let uv = vec2((x as f32 + 0.5) / 31.0, (y as f32 + 0.5) / 31.0);
                let ray = direction(light, uv);
                assert!(ray.z <= 0.0);
                if ray.z < 0.0 {
                    assert!((coordinates(light, ray) - uv).length() < 0.00001);
                }
            }
        }
    }
}
