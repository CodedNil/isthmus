//! The deferred passes of one sample. An interleaved sample covers every other pixel column, packed into half its target.
use crate::{
    home::Object,
    irradiance::{Grid, Probe},
    render::{
        Frame, Program,
        geometry::acceleration::{Node, Scene},
        light::shadows,
    },
    scene,
};
use isthmus::{prelude::*, spirv_std::arch::kill};
use isthmus_2d::{Rect, raster};

/// The target columns this sample covers: all of them, or the half its parity owns when interleaved.
fn columns(frame: ShaderFrame<Program>) -> Vec2 {
    let globals = frame.globals;
    let half = (frame.screen_size.x * 0.5).ceil();
    let start = if globals.interleaved && globals.parity != 0 { half } else { 0.0 };
    let end = if globals.interleaved && globals.parity == 0 { half } else { frame.screen_size.x };
    vec2(start, end)
}

fn covered(frame: ShaderFrame<Program>) -> Rect {
    let columns = columns(frame);
    Rect::new(vec2(columns.x, 0.0), vec2(columns.y, frame.screen_size.y))
}

/// The full-resolution pixel a covered target pixel stands for.
fn native_pixel(frame: ShaderFrame<Program>, pixel: Vec2) -> Vec2 {
    let globals = frame.globals;
    if !globals.interleaved {
        return pixel;
    }
    let [start, end] = columns(frame).to_array();
    if pixel.x < start || pixel.x >= end {
        kill();
    }
    vec2((pixel.x - start) * 2.0 - 0.5 + globals.parity as f32, pixel.y)
}

pub fn visibility_geometry(frame: &mut Frame<'_>, scene: &Scene) {
    assert!(scene.objects.len() < scene::VISIBILITY_OBJECTS as usize, "visibility object ID capacity exceeded");
    let nodes: &[Node] = &scene.nodes;
    let objects: &[Object] = &scene.objects;
    shader!(
        "visibility geometry",
        frame.format(TextureFormat::Rgba16Float).blend(Blend::Raw),
        |frame, nodes: &[Node], objects: &[Object]| raster(covered(frame)),
        |frame, surface| scene::visibility_geometry(
            frame.globals,
            nodes,
            objects,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
        ),
    );
}

pub fn visibility(frame: &mut Frame<'_>, scene: &Scene, geometry: &Image) {
    assert!(scene.objects.len() < scene::VISIBILITY_OBJECTS as usize, "visibility object ID capacity exceeded");
    let mirror = scene.mirror;
    let contacts: &[u32] = &scene.contacts;
    let nodes: &[Node] = &scene.nodes;
    let objects: &[Object] = &scene.objects;
    shader!(
        "visibility",
        frame.format(TextureFormat::Rgba16Float).blend(Blend::Raw),
        |frame, geometry: &Image, mirror: u32, contacts: &[u32], nodes: &[Node], objects: &[Object]| raster(covered(
            frame
        )),
        |frame, surface| scene::finish_visibility(
            frame.globals,
            nodes,
            objects,
            contacts,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
            geometry.texel(surface.pixel.as_ivec2()),
            mirror,
        ),
    );
}

pub fn materials(frame: &mut Frame<'_>, scene: &Scene, visibility: &Image) {
    let mirror = scene.mirror;
    let objects: &[Object] = &scene.objects;
    let grid: Grid = scene.grid;
    let probes: &[Probe] = &scene.probes;
    shader!(
        "materials",
        frame.format(TextureFormat::Rgba16Float).blend(Blend::Raw),
        |frame, objects: &[Object], grid: Grid, probes: &[Probe], visibility: &Image, mirror: u32| raster(covered(
            frame
        )),
        |frame, surface| scene::material_visibility(
            frame.globals,
            objects,
            grid,
            probes,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
            visibility.texel(surface.pixel.as_ivec2()),
            mirror,
        ),
    );
}

/// Lights every visible surface, with one pipeline per material family to limit register use.
pub fn shade(
    frame: &mut Frame<'_>,
    scene: &Scene,
    shadows: &shadows::ShadowAtlas,
    visibility: &Image,
    materials: &Image,
) {
    let mirror = scene.mirror;
    let objects: &[Object] = &scene.objects;
    // Cached shading reads no geometry or probes, so these are empty captures.
    let nodes: &[Node] = &[];
    let contacts: &[u32] = &[];
    let probes: &[Probe] = &[];
    let lights: &[u32] = &scene.lights;
    let light_masks: &[u32] = &scene.light_masks;
    let grid: Grid = scene.grid;
    let atlas: &Image = &shadows.image;
    let info: shadows::AtlasInfo = shadows.info;
    shader!(
        "shade 1 sample",
        frame.blend(Blend::Raw),
        |frame,
         visibility: &Image,
         materials: &Image,
         mirror: u32,
         objects: &[Object],
         nodes: &[Node],
         contacts: &[u32],
         probes: &[Probe],
         lights: &[u32],
         light_masks: &[u32],
         grid: Grid,
         atlas: &Image,
         info: shadows::AtlasInfo| raster(covered(frame)),
        |frame, surface| scene::shade_visibility::<1>(
            frame.globals,
            nodes,
            objects,
            lights,
            light_masks,
            contacts,
            grid,
            probes,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
            frame.time,
            atlas,
            info,
            visibility.texel(surface.pixel.as_ivec2()),
            mirror,
            materials.texel(surface.pixel.as_ivec2()),
        ),
    );
    shader!(
        "shade 2 samples",
        frame.blend(Blend::Raw),
        |frame,
         visibility: &Image,
         materials: &Image,
         mirror: u32,
         objects: &[Object],
         nodes: &[Node],
         contacts: &[u32],
         probes: &[Probe],
         lights: &[u32],
         light_masks: &[u32],
         grid: Grid,
         atlas: &Image,
         info: shadows::AtlasInfo| raster(covered(frame)),
        |frame, surface| scene::shade_visibility::<2>(
            frame.globals,
            nodes,
            objects,
            lights,
            light_masks,
            contacts,
            grid,
            probes,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
            frame.time,
            atlas,
            info,
            visibility.texel(surface.pixel.as_ivec2()),
            mirror,
            materials.texel(surface.pixel.as_ivec2()),
        ),
    );
    shader!(
        "shade 3 samples",
        frame.blend(Blend::Raw),
        |frame,
         visibility: &Image,
         materials: &Image,
         mirror: u32,
         objects: &[Object],
         nodes: &[Node],
         contacts: &[u32],
         probes: &[Probe],
         lights: &[u32],
         light_masks: &[u32],
         grid: Grid,
         atlas: &Image,
         info: shadows::AtlasInfo| raster(covered(frame)),
        |frame, surface| scene::shade_visibility::<3>(
            frame.globals,
            nodes,
            objects,
            lights,
            light_masks,
            contacts,
            grid,
            probes,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
            frame.time,
            atlas,
            info,
            visibility.texel(surface.pixel.as_ivec2()),
            mirror,
            materials.texel(surface.pixel.as_ivec2()),
        ),
    );
    shader!(
        "shade 4 samples",
        frame.blend(Blend::Raw),
        |frame,
         visibility: &Image,
         materials: &Image,
         mirror: u32,
         objects: &[Object],
         nodes: &[Node],
         contacts: &[u32],
         probes: &[Probe],
         lights: &[u32],
         light_masks: &[u32],
         grid: Grid,
         atlas: &Image,
         info: shadows::AtlasInfo| raster(covered(frame)),
        |frame, surface| scene::shade_visibility::<4>(
            frame.globals,
            nodes,
            objects,
            lights,
            light_masks,
            contacts,
            grid,
            probes,
            native_pixel(frame, surface.pixel),
            frame.screen_size,
            frame.pixel_size(),
            frame.time,
            atlas,
            info,
            visibility.texel(surface.pixel.as_ivec2()),
            mirror,
            materials.texel(surface.pixel.as_ivec2()),
        ),
    );
}
