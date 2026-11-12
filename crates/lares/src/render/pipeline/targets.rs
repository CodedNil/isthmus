//! The offscreen images one view renders into, and the recording that fills them.
use crate::{
    app::App,
    render::{
        Frame, Globals, Renderer,
        light::shadows::ShadowAtlas,
        pipeline::{passes, resolve::resolve},
    },
};
use core::array::from_fn;
use isthmus::{Image, RenderError, SurfaceHandle, glam::Vec2, wgpu::TextureFormat};

/// Views above this many pixels interleave their samples.
const INTERLEAVE_PIXELS: f32 = 2_500_000.0;

/// Four antialiasing samples and the buffers they are shaded from.
pub struct Targets {
    samples: [SurfaceHandle; 4],
    images: [Image; 4],
    visibility: (SurfaceHandle, Image),
    materials: (SurfaceHandle, Image),
    /// Primary visibility borrows the material image before materials are written.
    geometry: SurfaceHandle,
    size: [u32; 2],
}

impl Targets {
    pub fn new(gpu: &mut Renderer, size: [u32; 2]) -> Self {
        let pairs: [(SurfaceHandle, Image); 4] = from_fn(|_| gpu.add_image(size));
        let visibility = gpu.add_image_format(size, TextureFormat::Rgba16Float);
        let materials = gpu.add_image_format(size, TextureFormat::Rgba16Float);
        let geometry = gpu.alias_image(materials.0).expect("offscreen material target");
        Self {
            samples: pairs.each_ref().map(|pair| pair.0),
            images: pairs.map(|pair| pair.1),
            visibility,
            materials,
            geometry,
            size,
        }
    }

    /// Whether a view of this size interleaves its samples.
    pub fn interleaved(size: [u32; 2]) -> bool {
        size[0] as f32 * size[1] as f32 > INTERLEAVE_PIXELS
    }

    /// Releases the images and recreates them at a new size.
    pub fn resized(self, gpu: &mut Renderer, size: [u32; 2]) -> Self {
        for sample in self.samples {
            gpu.remove_surface(sample);
        }
        gpu.remove_surface(self.visibility.0);
        gpu.remove_surface(self.geometry);
        gpu.remove_surface(self.materials.0);
        Self::new(gpu, size)
    }

    /// Records one frame: the sample for `phase` when `redraw` is set, then the resolve into `output`.
    pub fn record(
        &self,
        gpu: &mut Renderer,
        app: &App,
        shadows: &ShadowAtlas,
        output: (SurfaceHandle, Vec2),
        phase: u32,
        mask: u32,
        redraw: bool,
    ) -> Result<(), RenderError> {
        let interleaved = Self::interleaved(self.size);
        let size = Vec2::from_array(self.size.map(|value| value as f32));
        gpu.render(|render| {
            if redraw {
                let sample = if interleaved { phase / 2 } else { phase };
                let globals = Globals { sample_index: sample, interleaved, parity: phase & 1, ..app.globals(size) };
                let scene = &app.scene;
                render.surface(self.geometry, size, globals, |mut frame: Frame<'_>| {
                    passes::visibility_geometry(&mut frame, scene);
                });
                render.surface(self.visibility.0, size, globals, |mut frame: Frame<'_>| {
                    passes::visibility(&mut frame, scene, &self.materials.1);
                });
                render.surface(self.materials.0, size, globals, |mut frame: Frame<'_>| {
                    passes::materials(&mut frame, scene, &self.visibility.1);
                });
                render.surface(self.samples[sample as usize], size, globals, |mut frame: Frame<'_>| {
                    passes::shade(&mut frame, scene, shadows, &self.visibility.1, &self.materials.1);
                });
            }
            render.surface(output.0, output.1, app.globals(output.1), |mut frame: Frame<'_>| {
                resolve(&mut frame, &self.images, mask, interleaved, &app.presence.ghosts);
            });
        })
    }
}
