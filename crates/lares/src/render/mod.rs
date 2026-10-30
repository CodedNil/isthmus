use crate::{
    home::{Kind, Object},
    presence::Ghost,
};
use acceleration::Node;
pub use acceleration::Scene;
use isthmus::{prelude::*, spirv_std::arch::kill};
#[cfg(not(target_arch = "spirv"))]
pub use scene::pixel;
pub use scene::pixel_cached;
pub use sun::{Sun, SunSettings};

pub mod acceleration;
mod ghosts;
mod motion_probe;

/// Temporary deterministic animation for diagnosing presentation jitter.
pub const MOTION_PROBE: bool = true;
mod layout;
#[cfg(not(target_arch = "spirv"))]
mod light_culling;
pub mod photometry;
mod plants;
pub(crate) mod scene;
pub mod shadows;
pub mod sun;
#[cfg(not(target_arch = "spirv"))]
pub mod temporal;

pub const WALL_HEIGHT: f32 = 2.4;
pub const WALL_THICKNESS: f32 = 0.1;
pub const EXTERIOR_WALL_THICKNESS: f32 = 0.30;
pub const FLOOR_THICKNESS: f32 = 0.12;
pub const CEILING_THICKNESS: f32 = 0.04;
pub const DOOR_HEIGHT: f32 = 2.05;
pub const WINDOW_SILL: f32 = 0.9;
pub const WINDOW_HEAD: f32 = 2.05;

#[derive(Clone, Copy, ShaderData)]
pub struct Globals {
    /// Camera position in world coordinates.
    pub eye: Vec3,
    /// Ray through the center of the screen.
    pub forward: Vec3,
    /// Horizontal ray offset at the screen edge.
    pub right: Vec3,
    /// Vertical ray offset at the screen edge.
    pub up: Vec3,
    /// Vertical camera angle per pixel, in radians.
    pub pixel_angle: f32,
    /// Photometric camera exposure, metered separately from light power.
    pub exposure: f32,
    /// Bounded photographic illuminant adaptation, applied before tone mapping.
    pub white_balance: Vec3,
    pub sun: Sun,
    pub sample_index: u32,
    pub interleaved: bool,
    pub parity: u32,
}

isthmus::program!(Globals, ());

#[allow(clippy::needless_borrow, reason = "Shader captures become owned GPU image handles")]
pub fn visibility(frame: &mut Frame<'_>, scene: &Scene) {
    assert!(scene.objects.len() < scene::VISIBILITY_OBJECTS as usize, "visibility object ID capacity exceeded");
    shader!(
        frame
            .format(isthmus::wgpu::TextureFormat::Rgba16Float)
            .blend(Blend::Raw)
            .upload({
                let contacts: &[u32] = &scene.contacts;
                let nodes: &[Node] = &scene.nodes;
                let objects: &[Object] = &scene.objects;
            })
            .primitive(|frame| {
                let half = (frame.screen_size.x * 0.5).ceil();
                let start = if frame.globals.interleaved && frame.globals.parity != 0 { half } else { 0.0 };
                let end =
                    if frame.globals.interleaved && frame.globals.parity == 0 { half } else { frame.screen_size.x };
                raster(Rect::new(vec2(start, 0.0), vec2(end, frame.screen_size.y)))
            })
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                let mut pixel = surface.pixel;
                if globals.interleaved {
                    let half = (size.x * 0.5).ceil();
                    let start = if globals.parity != 0 { half } else { 0.0 };
                    let end = if globals.parity == 0 { half } else { size.x };
                    if pixel.x < start || pixel.x >= end {
                        kill();
                    }
                    pixel.x = (pixel.x - start) * 2.0 - 0.5 + globals.parity as f32;
                }
                scene::visibility_pixel(globals, nodes, objects, contacts, pixel, size, frame.pixel_size)
            })
    );
}

#[allow(clippy::needless_borrow, reason = "Shader captures become owned GPU image handles")]
pub fn materials(frame: &mut Frame<'_>, scene: &Scene, visibility: &Image) {
    shader!(
        frame
            .format(isthmus::wgpu::TextureFormat::Rgba16Float)
            .blend(Blend::Raw)
            .upload({
                let objects: &[Object] = &scene.objects;
                let grid: scene::irradiance::Grid = scene.grid;
                let probes: &[scene::irradiance::Probe] = &scene.probes;
                let visibility: &Image;
                let mirror: u32 = scene.objects.iter().position(|item| item.kind == Kind::Mirror).unwrap_or(0) as u32;
            })
            .primitive(|frame| {
                let half = (frame.screen_size.x * 0.5).ceil();
                let start = if frame.globals.interleaved && frame.globals.parity != 0 { half } else { 0.0 };
                let end =
                    if frame.globals.interleaved && frame.globals.parity == 0 { half } else { frame.screen_size.x };
                raster(Rect::new(vec2(start, 0.0), vec2(end, frame.screen_size.y)))
            })
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                let mut pixel = surface.pixel;
                if globals.interleaved {
                    let half = (size.x * 0.5).ceil();
                    let start = if globals.parity != 0 { half } else { 0.0 };
                    let end = if globals.parity == 0 { half } else { size.x };
                    if pixel.x < start || pixel.x >= end {
                        kill();
                    }
                    pixel.x = (pixel.x - start) * 2.0 - 0.5 + globals.parity as f32;
                }
                scene::material_visibility(
                    globals,
                    objects,
                    grid,
                    probes,
                    pixel,
                    size,
                    frame.pixel_size,
                    visibility.texel(surface.pixel.as_ivec2()),
                    mirror,
                )
            })
    );
}

#[allow(clippy::needless_borrow, reason = "Shader captures become owned GPU image handles")]
pub fn draw(
    frame: &mut Frame<'_>,
    scene: &Scene,
    shadows: &shadows::ShadowAtlas,
    visibility: &Image,
    materials: &Image,
) {
    shader!(
        frame
            .blend(Blend::Raw)
            .upload({
                let visibility: &Image;
                let materials: &Image;
                let mirror: u32 = scene.objects.iter().position(|item| item.kind == Kind::Mirror).unwrap_or(0) as u32;
                let objects: &[Object] = &scene.objects;
                // Deferred lighting never queries scene geometry or probes.
                // Typed empty captures avoid re-encoding those large arrays.
                let nodes: &[Node] = &[];
                let contacts: &[u32] = &[];
                let probes: &[scene::irradiance::Probe] = &[];
                let lights: &[u32] = &scene.lights;
                let light_masks: &[u32] = &scene.light_masks;
                let grid: scene::irradiance::Grid = scene.grid;
                let atlas: &Image = &shadows.image;
                let info: shadows::AtlasInfo = shadows.info;
            })
            .primitive(|frame| {
                let half = (frame.screen_size.x * 0.5).ceil();
                let start = if frame.globals.interleaved && frame.globals.parity != 0 { half } else { 0.0 };
                let end =
                    if frame.globals.interleaved && frame.globals.parity == 0 { half } else { frame.screen_size.x };
                raster(Rect::new(vec2(start, 0.0), vec2(end, frame.screen_size.y)))
            })
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                let mut pixel = surface.pixel;
                if globals.interleaved {
                    let half = (size.x * 0.5).ceil();
                    let start = if globals.parity != 0 { half } else { 0.0 };
                    let end = if globals.parity == 0 { half } else { size.x };
                    if pixel.x < start || pixel.x >= end {
                        kill();
                    }
                    pixel.x = (pixel.x - start) * 2.0 - 0.5 + globals.parity as f32;
                }
                scene::shade_visibility::<1>(
                    globals,
                    nodes,
                    objects,
                    lights,
                    light_masks,
                    contacts,
                    grid,
                    probes,
                    pixel,
                    size,
                    frame.pixel_size,
                    frame.time,
                    &atlas,
                    info,
                    visibility.texel(surface.pixel.as_ivec2()),
                    mirror,
                    materials.texel(surface.pixel.as_ivec2()),
                )
            })
    );
    shader!(
        frame
            .blend(Blend::Raw)
            .upload({
                let visibility: &Image;
                let materials: &Image;
                let mirror: u32 = scene.objects.iter().position(|item| item.kind == Kind::Mirror).unwrap_or(0) as u32;
                let objects: &[Object] = &scene.objects;
                // Deferred lighting never queries scene geometry or probes.
                // Typed empty captures avoid re-encoding those large arrays.
                let nodes: &[Node] = &[];
                let contacts: &[u32] = &[];
                let probes: &[scene::irradiance::Probe] = &[];
                let lights: &[u32] = &scene.lights;
                let light_masks: &[u32] = &scene.light_masks;
                let grid: scene::irradiance::Grid = scene.grid;
                let atlas: &Image = &shadows.image;
                let info: shadows::AtlasInfo = shadows.info;
            })
            .primitive(|frame| {
                let half = (frame.screen_size.x * 0.5).ceil();
                let start = if frame.globals.interleaved && frame.globals.parity != 0 { half } else { 0.0 };
                let end =
                    if frame.globals.interleaved && frame.globals.parity == 0 { half } else { frame.screen_size.x };
                raster(Rect::new(vec2(start, 0.0), vec2(end, frame.screen_size.y)))
            })
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                let mut pixel = surface.pixel;
                if globals.interleaved {
                    let half = (size.x * 0.5).ceil();
                    let start = if globals.parity != 0 { half } else { 0.0 };
                    let end = if globals.parity == 0 { half } else { size.x };
                    if pixel.x < start || pixel.x >= end {
                        kill();
                    }
                    pixel.x = (pixel.x - start) * 2.0 - 0.5 + globals.parity as f32;
                }
                scene::shade_visibility::<2>(
                    globals,
                    nodes,
                    objects,
                    lights,
                    light_masks,
                    contacts,
                    grid,
                    probes,
                    pixel,
                    size,
                    frame.pixel_size,
                    frame.time,
                    &atlas,
                    info,
                    visibility.texel(surface.pixel.as_ivec2()),
                    mirror,
                    materials.texel(surface.pixel.as_ivec2()),
                )
            })
    );
    shader!(
        frame
            .blend(Blend::Raw)
            .upload({
                let visibility: &Image;
                let materials: &Image;
                let mirror: u32 = scene.objects.iter().position(|item| item.kind == Kind::Mirror).unwrap_or(0) as u32;
                let objects: &[Object] = &scene.objects;
                // Deferred lighting never queries scene geometry or probes.
                // Typed empty captures avoid re-encoding those large arrays.
                let nodes: &[Node] = &[];
                let contacts: &[u32] = &[];
                let probes: &[scene::irradiance::Probe] = &[];
                let lights: &[u32] = &scene.lights;
                let light_masks: &[u32] = &scene.light_masks;
                let grid: scene::irradiance::Grid = scene.grid;
                let atlas: &Image = &shadows.image;
                let info: shadows::AtlasInfo = shadows.info;
            })
            .primitive(|frame| {
                let half = (frame.screen_size.x * 0.5).ceil();
                let start = if frame.globals.interleaved && frame.globals.parity != 0 { half } else { 0.0 };
                let end =
                    if frame.globals.interleaved && frame.globals.parity == 0 { half } else { frame.screen_size.x };
                raster(Rect::new(vec2(start, 0.0), vec2(end, frame.screen_size.y)))
            })
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                let mut pixel = surface.pixel;
                if globals.interleaved {
                    let half = (size.x * 0.5).ceil();
                    let start = if globals.parity != 0 { half } else { 0.0 };
                    let end = if globals.parity == 0 { half } else { size.x };
                    if pixel.x < start || pixel.x >= end {
                        kill();
                    }
                    pixel.x = (pixel.x - start) * 2.0 - 0.5 + globals.parity as f32;
                }
                scene::shade_visibility::<3>(
                    globals,
                    nodes,
                    objects,
                    lights,
                    light_masks,
                    contacts,
                    grid,
                    probes,
                    pixel,
                    size,
                    frame.pixel_size,
                    frame.time,
                    &atlas,
                    info,
                    visibility.texel(surface.pixel.as_ivec2()),
                    mirror,
                    materials.texel(surface.pixel.as_ivec2()),
                )
            })
    );
    shader!(
        frame
            .blend(Blend::Raw)
            .upload({
                let visibility: &Image;
                let materials: &Image;
                let mirror: u32 = scene.objects.iter().position(|item| item.kind == Kind::Mirror).unwrap_or(0) as u32;
                let objects: &[Object] = &scene.objects;
                // Deferred lighting never queries scene geometry or probes.
                // Typed empty captures avoid re-encoding those large arrays.
                let nodes: &[Node] = &[];
                let contacts: &[u32] = &[];
                let probes: &[scene::irradiance::Probe] = &[];
                let lights: &[u32] = &scene.lights;
                let light_masks: &[u32] = &scene.light_masks;
                let grid: scene::irradiance::Grid = scene.grid;
                let atlas: &Image = &shadows.image;
                let info: shadows::AtlasInfo = shadows.info;
            })
            .primitive(|frame| {
                let half = (frame.screen_size.x * 0.5).ceil();
                let start = if frame.globals.interleaved && frame.globals.parity != 0 { half } else { 0.0 };
                let end =
                    if frame.globals.interleaved && frame.globals.parity == 0 { half } else { frame.screen_size.x };
                raster(Rect::new(vec2(start, 0.0), vec2(end, frame.screen_size.y)))
            })
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                let mut pixel = surface.pixel;
                if globals.interleaved {
                    let half = (size.x * 0.5).ceil();
                    let start = if globals.parity != 0 { half } else { 0.0 };
                    let end = if globals.parity == 0 { half } else { size.x };
                    if pixel.x < start || pixel.x >= end {
                        kill();
                    }
                    pixel.x = (pixel.x - start) * 2.0 - 0.5 + globals.parity as f32;
                }
                scene::shade_visibility::<4>(
                    globals,
                    nodes,
                    objects,
                    lights,
                    light_masks,
                    contacts,
                    grid,
                    probes,
                    pixel,
                    size,
                    frame.pixel_size,
                    frame.time,
                    &atlas,
                    info,
                    visibility.texel(surface.pixel.as_ivec2()),
                    mirror,
                    materials.texel(surface.pixel.as_ivec2()),
                )
            })
    );
}

/// Resolve independent full-resolution SDF samples in linear light. Camera
/// changes invalidate every older sample, so there is no temporal ghosting.
#[allow(clippy::needless_borrow, reason = "Shader captures become owned GPU image handles")]
pub fn resolve(frame: &mut Frame<'_>, images: &[Image; 4], mask: u32, interleaved: bool, people: &[Ghost]) {
    shader!(
        frame
            .blend(Blend::Replace)
            .upload({
                let first: &Image = &images[0];
                let second: &Image = &images[1];
                let third: &Image = &images[2];
                let fourth: &Image = &images[3];
                let mask: u32;
                let sample_size: Vec2 = Vec2::from_array(images[0].size().map(|value| value as f32));
                let interleaved: bool;
                let people: &[Ghost];
            })
            .primitive(|frame| Rect::new(Vec2::ZERO, frame.screen_size))
            .fragment(|frame, surface| {
                let size = sample_size;
                let p = surface.pixel * size / frame.screen_size;
                let color = resolved_sample(&first, p, size, mask, 0, interleaved)
                    + resolved_sample(&second, p, size, mask, 1, interleaved)
                    + resolved_sample(&third, p, size, mask, 2, interleaved)
                    + resolved_sample(&fourth, p, size, mask, 3, interleaved);
                let ndc = surface.pixel / frame.screen_size * 2.0 - 1.0;
                let globals = frame.globals;
                let ray = (globals.forward + globals.right * ndc.x - globals.up * ndc.y).normalize();
                let height = resolved_height(&first, p, size, mask, interleaved);
                let color = ghosts::composite(
                    color.truncate() / color.w.max_num(1.0),
                    people,
                    globals.eye,
                    ray,
                    height,
                    globals.pixel_angle,
                );
                let color = if MOTION_PROBE {
                    motion_probe::composite(color, globals.eye, ray, height, frame.time)
                } else {
                    color
                };
                color.powf(1.0 / 2.2).extend(1.0)
            })
    );
}

fn resolved_height(image: &Image, pixel: Vec2, size: Vec2, mask: u32, interleaved: bool) -> f32 {
    let mut position = pixel;
    if interleaved {
        let mut parity = pixel.x.floor() as u32 & 1;
        if mask & (1 << parity) == 0 {
            parity ^= 1;
        }
        let half = (size.x * 0.5).ceil();
        let start = if parity != 0 { half } else { 0.0 };
        let end = if parity != 0 { size.x } else { half };
        position.x = ((pixel.x + 0.5 - parity as f32) * 0.5 + start).clamp_num(start + 0.5, end - 0.5);
    }
    image.sample_raw(position / size).w * 3.0
}

fn resolved_sample(image: &Image, pixel: Vec2, size: Vec2, mask: u32, sample: u32, interleaved: bool) -> Vec4 {
    let mut position = pixel;
    if interleaved {
        let mut parity = pixel.x.floor() as u32 & 1;
        if mask & (1 << (sample * 2 + parity)) == 0 {
            parity ^= 1;
        }
        if mask & (1 << (sample * 2 + parity)) == 0 {
            return Vec4::ZERO;
        }
        let half = (size.x * 0.5).ceil();
        let start = if parity != 0 { half } else { 0.0 };
        let end = if parity != 0 { size.x } else { half };
        position.x = ((pixel.x + 0.5 - parity as f32) * 0.5 + start).clamp(start + 0.5, end - 0.5);
    } else if mask & (1 << sample) == 0 {
        return Vec4::ZERO;
    }
    let mut color = image.sample_raw(position / size).truncate().powf(2.2);
    if !interleaved && mask.count_ones() < 4 {
        // Soften high-contrast single-sample edges during movement/restarts.
        // This disappears completely at four samples; native settled detail
        // and procedural material frequencies remain unchanged.
        let a = image.sample_raw((position + vec2(0.5, 0.0)) / size).truncate().powf(2.2);
        let b = image.sample_raw((position - vec2(0.5, 0.0)) / size).truncate().powf(2.2);
        let c = image.sample_raw((position + vec2(0.0, 0.5)) / size).truncate().powf(2.2);
        let d = image.sample_raw((position - vec2(0.0, 0.5)) / size).truncate().powf(2.2);
        let high = a.max(b).max(c).max(d).max_element();
        let low = a.min(b).min(c).min(d).min_element();
        if high - low > (high + 0.01) * 0.15 {
            color = color.lerp((a + b + c + d) * 0.25, (4 - mask.count_ones()) as f32 / 6.0);
        }
    }
    color.extend(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaved_resolve_restores_every_native_pixel_for_even_and_odd_widths() {
        for width in 1u32..=33 {
            let half = width.div_ceil(2);
            let mut bytes = vec![0u8; width as usize * 4];
            for x in 0..width {
                let slot = x / 2 + if x & 1 != 0 { half } else { 0 };
                bytes[slot as usize * 4..slot as usize * 4 + 4].copy_from_slice(&[(x * 7) as u8, 64, 128, 255]);
            }
            let image = Image::rgba8([width, 1], bytes);
            for x in 0..width {
                let value = resolved_sample(&image, vec2(x as f32 + 0.5, 0.5), vec2(width as f32, 1.0), 3, 0, true);
                let expected = vec3((x * 7) as f32, 64.0, 128.0) / 255.0;
                assert!((value.truncate().powf(1.0 / 2.2) - expected).length() < 0.00001);
                assert_eq!(value.w, 1.0);
            }
            assert_eq!(resolved_sample(&image, vec2(0.5, 0.5), vec2(width as f32, 1.0), 0, 0, true), Vec4::ZERO);
        }
    }
}
