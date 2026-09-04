use super::context::{Context, SetupError, create_surface};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub(super) enum Present {
    Rendered,
    Unavailable,
    Lost,
    Validation,
}

pub(super) struct SurfaceFrame {
    pub(super) output: wgpu::SurfaceTexture,
    pub(super) view: wgpu::TextureView,
}
pub(super) struct SurfaceTarget {
    context: Context,
    pub(super) surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    needs_reconfigure: bool,
    pub(super) format: wgpu::TextureFormat,
    pub(super) extent: [u32; 2],
}

impl SurfaceTarget {
    pub(crate) fn new(
        context: &Context,
        source: &(impl HasDisplayHandle + HasWindowHandle),
        width: u32,
        height: u32,
    ) -> Result<Self, SetupError> {
        let surface = create_surface(&context.0.instance, source)?;
        Self::from_raw(context, surface, width, height)
    }

    pub(super) fn from_raw(
        context: &Context,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, SetupError> {
        let config = context.configure_surface(&surface, width, height)?;
        surface.configure(&context.0.device, &config);
        Ok(Self {
            context: context.clone(),
            format: config.format,
            extent: [config.width, config.height],
            surface,
            config,
            needs_reconfigure: false,
        })
    }

    pub(super) fn acquire(&mut self) -> Result<SurfaceFrame, Present> {
        // Outdated swapchains get one immediate reconfiguration attempt.
        for _ in 0..2 {
            if self.needs_reconfigure {
                self.surface.configure(&self.context.0.device, &self.config);
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
                    return Err(Present::Unavailable);
                }
                wgpu::CurrentSurfaceTexture::Lost => return Err(Present::Lost),
                wgpu::CurrentSurfaceTexture::Validation => return Err(Present::Validation),
            };
            let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
            return Ok(SurfaceFrame { output, view });
        }
        Err(Present::Unavailable)
    }

    pub(super) fn present(&self, frame: SurfaceFrame) -> Present {
        self.context.0.queue.present(frame.output);
        Present::Rendered
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width != 0 && height != 0 && self.extent != [width, height] {
            self.config.width = width;
            self.config.height = height;
            self.extent = [width, height];
            self.surface.configure(&self.context.0.device, &self.config);
            self.needs_reconfigure = false;
        }
    }
}
