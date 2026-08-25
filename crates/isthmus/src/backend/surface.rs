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
        })
    }
    pub(super) fn acquire(&self) -> Result<SurfaceFrame, Present> {
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated => {
                return Err(Present::Unavailable);
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err(Present::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(Present::Validation),
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(SurfaceFrame { output, view })
    }
    pub(super) fn present(&self, frame: SurfaceFrame) -> Present {
        self.context.0.queue.present(frame.output);
        Present::Rendered
    }
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width != 0 && height != 0 {
            self.config.width = width;
            self.config.height = height;
            self.extent = [width, height];
            self.surface.configure(&self.context.0.device, &self.config);
        }
    }
}
