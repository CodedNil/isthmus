use super::{gpu::Gpu, surface::SurfaceTarget};
use crate::Program;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error(transparent)]
    Handle(#[from] raw_window_handle::HandleError),
    #[error("GPU adapter error: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("GPU device error: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("surface error: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("surface is unsupported")]
    UnsupportedSurface,
    #[error("replacement surface is incompatible")]
    IncompatibleSurface,
    #[error("WebGPU is unavailable in this browser")]
    WebGpuUnavailable,
}

pub(super) unsafe fn create_surface(
    instance: &wgpu::Instance,
    source: &(impl HasDisplayHandle + HasWindowHandle),
) -> Result<wgpu::Surface<'static>, SetupError> {
    // SAFETY: The caller keeps both raw handles alive for the returned surface.
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(source.display_handle()?.as_raw()),
            raw_window_handle: source.window_handle()?.as_raw(),
        })
    }
    .map_err(Into::into)
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) unsafe fn new<P: Program>(
    source: &(impl HasDisplayHandle + HasWindowHandle),
    size: [u32; 2],
) -> Result<(Gpu, SurfaceTarget), SetupError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // SAFETY: The caller keeps both native handles alive for the returned surface.
    let surface = unsafe { create_surface(&instance, source) }?;
    pollster::block_on(finish::<P>(instance, surface, size, wgpu::PowerPreference::HighPerformance))
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn new<P: Program>(
    canvas: web_sys::HtmlCanvasElement,
    size: [u32; 2],
) -> Result<(Gpu, SurfaceTarget), SetupError> {
    if !wgpu::util::is_browser_webgpu_supported().await {
        return Err(SetupError::WebGpuUnavailable);
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas))?;
    finish::<P>(instance, surface, size, wgpu::PowerPreference::None).await
}

async fn finish<P: Program>(
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    [width, height]: [u32; 2],
    power_preference: wgpu::PowerPreference,
) -> Result<(Gpu, SurfaceTarget), SetupError> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("isthmus"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        })
        .await?;
    let config = configure_surface(&adapter, &surface, width, height)?;
    let gpu = Gpu::new::<P>(instance, adapter, device, queue, config.format);
    let target = SurfaceTarget::from_raw(&gpu.device, surface, config);
    Ok((gpu, target))
}

pub(super) fn configure_surface(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'static>,
    width: u32,
    height: u32,
) -> Result<wgpu::SurfaceConfiguration, SetupError> {
    let caps = surface.get_capabilities(adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|format| *format == wgpu::TextureFormat::Bgra8Unorm)
        .or_else(|| caps.formats.iter().copied().find(|format| *format == wgpu::TextureFormat::Rgba8Unorm))
        .or_else(|| caps.formats.first().copied())
        .ok_or(SetupError::UnsupportedSurface)?;
    let alpha_mode =
        [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Auto, wgpu::CompositeAlphaMode::Opaque]
            .into_iter()
            .find(|mode| caps.alpha_modes.contains(mode))
            .or_else(|| caps.alpha_modes.first().copied())
            .ok_or(SetupError::UnsupportedSurface)?;
    Ok(wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    })
}
