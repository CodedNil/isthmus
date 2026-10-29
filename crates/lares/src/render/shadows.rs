//! SDF-traced variance shadows, independent of camera, dimming and light colour.
use super::{acceleration::Node, scene};
use crate::home::{Kind, Object};
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

fn emitter(light: Object) -> Vec3 {
    let down = light.kind == Kind::Downlight;
    light.pos + Vec3::Z * if down { -0.025 } else { 0.005 }
}

fn pack(value: f32) -> Vec2 {
    let bits = (value * 65535.0).clamp_num(0.0, 65535.0).round();
    let high = (bits / 256.0).floor();
    vec2(high, bits - high * 256.0) / 255.0
}
fn unpack(value: Vec2) -> f32 {
    value.dot(vec2(65280.0, 255.0)) / 65535.0
}
fn moments(value: Vec4) -> Vec2 {
    vec2(unpack(value.xy()), unpack(value.zw()))
}
fn packed(value: Vec2) -> Vec4 {
    let a = pack(value.x);
    let b = pack(value.y);
    vec4(a.x, a.y, b.x, b.y)
}

pub fn depth_pixel(nodes: &[Node], objects: &[Object], light: Object, uv: Vec2) -> Vec4 {
    let direction = decode(uv.clamp_num(Vec2::ZERO, Vec2::ONE));
    let distance = if light.kind == Kind::Downlight && direction.z >= 0.0 {
        RANGE
    } else {
        scene::light_depth(emitter(light), direction, nodes, objects, RANGE)
    };
    let d = distance / RANGE;
    packed(vec2(d, d * d))
}

pub fn visible(
    atlas: &Image,
    info: AtlasInfo,
    slot: u32,
    _sample: u32,
    light: Object,
    point: Vec3,
    normal: Vec3,
) -> f32 {
    let source = emitter(light);
    let delta = point - source;
    let uv = encode(delta);
    let value = moments(
        atlas.sample_raw((info.origin(slot) + Vec2::splat(PADDING as f32) + uv * info.resolution as f32) / info.size),
    );
    // A filtered texel covers a larger depth interval on grazing receivers.
    // Account for that footprint rather than letting a surface shadow itself
    // at rounded rims and wall junctions. Cap the offset to retain contacts.
    let distance = delta.length();
    let cosine = normal.dot(-delta / distance.max_num(0.001)).abs().max_num(0.15);
    let slope = (1.0 - cosine * cosine).max_num(0.0).sqrt() / cosine;
    let footprint = distance * 4.0 / info.resolution as f32;
    let bias = (0.003 + footprint * slope * 0.5).min_num(0.025);
    let receiver = (distance - bias) / RANGE;
    if receiver <= value.x {
        return 1.0;
    }
    let variance = (value.y - value.x * value.x).max_num(0.000_000_2);
    let difference = receiver - value.x;
    let probability = variance / (variance + difference * difference);
    // Suppress the low-probability tail that otherwise leaks behind blockers.
    ((probability - 0.22) / 0.78).clamp_num(0.0, 1.0)
}

pub fn filter_pixel(source: &Image, info: AtlasInfo, origin: Vec2, pixel: Vec2, axis: Vec2) -> Vec4 {
    let mut sum = Vec2::ZERO;
    for tap in 0..5 {
        let weight = if tap == 0 || tap == 4 {
            1.0
        } else if tap == 1 || tap == 3 {
            4.0
        } else {
            6.0
        };
        let p = (pixel + axis * (tap as f32 - 2.0)).clamp_num(origin + 0.5, origin + info.pitch() as f32 - 0.5);
        sum += moments(source.sample_raw(p / info.size)) * (weight / 16.0);
    }
    packed(sum)
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
        output.write_all(b"LARESV02")?;
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
        if &header[..8] != b"LARESV02" || header[8..16] != scene.signature().to_le_bytes() {
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
        let mut image = Image::raw_rgba8([width as u32, height as u32], pixels);
        for axis in [Vec2::X, Vec2::Y] {
            let mut filtered = vec![255u8; width * height * 4];
            let source = &image;
            thread::scope(|scope| {
                for (chunk, bytes) in filtered.chunks_mut(rows * width * 4).enumerate() {
                    scope.spawn(move || {
                        for (index, rgba) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                            let x = index % width;
                            let y = chunk * rows + index / width;
                            let pitch = info.pitch() as usize;
                            let tile = y / pitch * info.columns as usize + x / pitch;
                            if tile >= scene.lights.len() {
                                continue;
                            }
                            let value = filter_pixel(
                                source,
                                info,
                                info.origin(tile as u32),
                                vec2(x as f32 + 0.5, y as f32 + 0.5),
                                axis,
                            );
                            *rgba = value.to_array().map(|v| (v * 255.0).round() as u8);
                        }
                    });
                }
            });
            image = Image::raw_rgba8([width as u32, height as u32], filtered);
        }
        Self { info, image }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn depth_moments_filter_without_alpha_coupling() {
        for value in [0.0, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let encoded = packed(vec2(value, value * value));
            let bytes = encoded.to_array().map(|v| (v * 255.0).round() as u8).to_vec();
            let image = Image::raw_rgba8([1, 1], bytes);
            let decoded = moments(image.sample_raw(Vec2::splat(0.5)));
            assert!((decoded.x - value).abs() < 1.0 / 65535.0);
            assert!((decoded.y - value * value).abs() < 1.0 / 65535.0);
        }
    }
}
