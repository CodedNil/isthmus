use super::{
    gpu::{Gpu, Paint},
    renderer::RenderError,
};

pub struct SurfaceTarget {
    pub(super) target: Target,
    pub(super) paints: Vec<Paint>,
    pub(super) frame: u32,
}

pub(super) enum Target {
    Window { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration, needs_reconfigure: bool },
    Image(crate::Image),
}

impl SurfaceTarget {
    pub(super) fn from_raw(
        device: &wgpu::Device,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        surface.configure(device, &config);
        Self { target: Target::Window { surface, config, needs_reconfigure: false }, paints: Vec::new(), frame: 0 }
    }

    pub(super) const fn from_image(image: crate::Image) -> Self {
        Self { target: Target::Image(image), paints: Vec::new(), frame: 0 }
    }

    pub(super) const fn size(&self) -> [u32; 2] {
        match &self.target {
            Target::Window { config, .. } => [config.width, config.height],
            Target::Image(image) => image.size(),
        }
    }

    pub(super) fn acquire(
        &mut self,
        gpu: &Gpu,
    ) -> Result<Option<(wgpu::TextureView, Option<wgpu::SurfaceTexture>)>, RenderError> {
        let (surface, config, needs_reconfigure) = match &mut self.target {
            Target::Window { surface, config, needs_reconfigure } => (surface, config, needs_reconfigure),
            Target::Image(image) => {
                return Ok(Some((gpu.images.view(image).ok_or(RenderError::MissingImage)?.clone(), None)));
            }
        };
        // Outdated swapchains get one immediate reconfiguration attempt.
        for _ in 0..2 {
            if *needs_reconfigure {
                surface.configure(&gpu.device, config);
                *needs_reconfigure = false;
            }
            let output = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(output) => output,
                wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                    // Reconfiguration must wait until this texture is released.
                    *needs_reconfigure = true;
                    output
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    *needs_reconfigure = true;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(None);
                }
                wgpu::CurrentSurfaceTexture::Lost => return Err(RenderError::SurfaceLost),
                wgpu::CurrentSurfaceTexture::Validation => return Err(RenderError::Validation),
            };
            return Ok(Some((output.texture.create_view(&wgpu::TextureViewDescriptor::default()), Some(output))));
        }
        Ok(None)
    }

    pub(crate) fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        let Target::Window { surface, config, needs_reconfigure } = &mut self.target else { return };
        let limit = gpu.device.limits().max_texture_dimension_2d;
        let (width, height) = (width.min(limit), height.min(limit));
        if width != 0 && height != 0 && [config.width, config.height] != [width, height] {
            config.width = width;
            config.height = height;
            surface.configure(&gpu.device, config);
            *needs_reconfigure = false;
        }
    }
}

slotmap::new_key_type! {
    /// Identifies a presentation surface owned by its renderer.
    pub struct SurfaceHandle;
}
