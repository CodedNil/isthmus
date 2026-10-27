use super::{
    buffer::UploadBuffer,
    gpu::{Gpu, Paint},
    renderer::RenderError,
};
use crate::bindings;

pub struct SurfaceTarget {
    pub(super) surface: Option<wgpu::Surface<'static>>,
    pub(super) image: Option<wgpu::TextureView>,
    pub(super) config: wgpu::SurfaceConfiguration,
    pub(super) binding: Option<([wgpu::Buffer; bindings::BUFFER_COUNT], wgpu::BindGroup)>,
    pub(super) recorded: bool,
    pub(super) paints: Vec<Paint>,
    pub(super) frame: UploadBuffer,
    needs_reconfigure: bool,
}

impl SurfaceTarget {
    pub(super) fn from_raw(
        device: &wgpu::Device,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        surface.configure(device, &config);
        Self {
            surface: Some(surface),
            image: None,
            config,
            needs_reconfigure: false,
            binding: None,
            recorded: false,
            paints: Vec::new(),
            frame: UploadBuffer::new(device),
        }
    }

    pub(super) fn from_image(
        device: &wgpu::Device,
        config: wgpu::SurfaceConfiguration,
        view: wgpu::TextureView,
    ) -> Self {
        Self {
            surface: None,
            image: Some(view),
            config,
            binding: None,
            recorded: false,
            paints: Vec::new(),
            frame: UploadBuffer::new(device),
            needs_reconfigure: false,
        }
    }

    pub(super) fn acquire(&mut self, gpu: &Gpu) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        let Some(surface) = self.surface.as_ref() else { return Ok(None) };
        // Outdated swapchains get one immediate reconfiguration attempt.
        for _ in 0..2 {
            if self.needs_reconfigure {
                surface.configure(&gpu.device, &self.config);
                self.needs_reconfigure = false;
            }
            let output = match surface.get_current_texture() {
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
        let Some(surface) = self.surface.as_ref() else { return };
        if width != 0 && height != 0 && [self.config.width, self.config.height] != [width, height] {
            self.config.width = width;
            self.config.height = height;
            surface.configure(&gpu.device, &self.config);
            self.needs_reconfigure = false;
        }
    }
}

slotmap::new_key_type! {
    /// Identifies a presentation surface owned by its renderer.
    pub struct SurfaceHandle;
}
