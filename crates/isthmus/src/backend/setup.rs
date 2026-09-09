use super::{gpu::Gpu, surface::SurfaceTarget};
use crate::Program;

/// A failure while selecting a GPU or creating a presentation surface.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// No suitable GPU adapter could be selected.
    #[error("GPU adapter error: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// The GPU device could not be created with the required capabilities.
    #[error("GPU device error: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    /// The presentation surface could not be created.
    #[error("surface error: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    /// The surface has no supported presentation configuration.
    #[error("surface is unsupported")]
    UnsupportedSurface,
    /// An additional surface requires a different render target format.
    #[error("replacement surface is incompatible")]
    IncompatibleSurface,
}

pub(super) async fn new<P: Program>(
    source: impl Into<wgpu::SurfaceTarget<'static>>,
    [width, height]: [u32; 2],
) -> Result<(Gpu, SurfaceTarget), SetupError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance.create_surface(source)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await?;
    let images = u32::try_from(P::SHADERS.iter().map(|entry| entry.images).max().unwrap_or(0)).unwrap_or(u32::MAX);
    let mut limits = wgpu::Limits::default().using_resolution(adapter.limits());
    limits.max_sampled_textures_per_shader_stage = limits.max_sampled_textures_per_shader_stage.max(images);
    limits.max_samplers_per_shader_stage = limits.max_samplers_per_shader_stage.max(images);
    limits.max_bindings_per_bind_group = limits.max_bindings_per_bind_group.max(images.saturating_mul(2));
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("isthmus"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        })
        .await?;
    let config = configure_surface(&adapter, &surface, [width, height], None)?;
    let gpu = Gpu::new::<P>(instance, adapter, device, queue, config.format);
    let target = SurfaceTarget::from_raw(&gpu.device, surface, config);
    Ok((gpu, target))
}

pub(super) fn configure_surface(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'static>,
    [width, height]: [u32; 2],
    format: Option<wgpu::TextureFormat>,
) -> Result<wgpu::SurfaceConfiguration, SetupError> {
    let caps = surface.get_capabilities(adapter);
    let format = match format {
        Some(format) if !caps.formats.contains(&format) => return Err(SetupError::IncompatibleSurface),
        Some(format) => format,
        None => [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
            .into_iter()
            .find(|format| caps.formats.contains(format))
            .or_else(|| caps.formats.first().copied())
            .ok_or(SetupError::UnsupportedSurface)?,
    };
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
