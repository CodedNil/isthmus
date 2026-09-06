use super::{
    gpu::{Gpu, SurfacePaints},
    renderer::RenderError,
};

pub(super) struct SurfaceTarget {
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) config: wgpu::SurfaceConfiguration,
    pub(super) paints: SurfacePaints,
    needs_reconfigure: bool,
}

impl SurfaceTarget {
    pub(super) fn from_raw(
        device: &wgpu::Device,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        surface.configure(device, &config);
        Self { paints: SurfacePaints::new(device), surface, config, needs_reconfigure: false }
    }

    pub(super) fn acquire(&mut self, gpu: &Gpu) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        // Outdated swapchains get one immediate reconfiguration attempt.
        for _ in 0..2 {
            if self.needs_reconfigure {
                self.surface.configure(&gpu.device, &self.config);
                self.needs_reconfigure = false;
            }
            let output = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(output) => output,
                wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                    // Reconfiguration must wait until this texture is released.
                    self.needs_reconfigure = true;
                    output
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    self.needs_reconfigure = true;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(None);
                }
                wgpu::CurrentSurfaceTexture::Lost => return Err(RenderError::SurfaceLost),
                wgpu::CurrentSurfaceTexture::Validation => return Err(RenderError::Validation),
            };
            return Ok(Some(output));
        }
        Ok(None)
    }

    pub(crate) fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width != 0 && height != 0 && [self.config.width, self.config.height] != [width, height] {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&gpu.device, &self.config);
            self.needs_reconfigure = false;
        }
    }
}

slotmap::new_key_type! {
    /// Identifies a presentation surface owned by its renderer.
    pub struct SurfaceHandle;
}
