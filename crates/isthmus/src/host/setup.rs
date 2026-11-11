use super::{error::SetupError, gpu::Gpu, surface::SurfaceTarget};
use crate::Program;
use isthmus_abi as abi;

pub(super) async fn new<P: Program>(
    source: impl Into<wgpu::SurfaceTarget<'static>>,
    [width, height]: [u32; 2],
) -> Result<(Gpu, SurfaceTarget), SetupError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let surface = instance.create_surface(source)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await?;
    let config = configure_surface(&adapter, &surface, [width, height], None)?;
    let gpu = device::<P>(instance, adapter, config.format).await?;
    let target = SurfaceTarget::from_raw(&gpu.device, surface, config);
    Ok((gpu, target))
}

pub(super) async fn device<P: Program>(
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    format: wgpu::TextureFormat,
) -> Result<Gpu, SetupError> {
    let images = u32::try_from(P::max_images()).unwrap_or(u32::MAX);
    let mut limits = wgpu::Limits::default().using_resolution(adapter.limits());
    limits.max_storage_buffers_per_shader_stage = abi::BUFFER_COUNT as u32;
    limits.max_sampled_textures_per_shader_stage = limits.max_sampled_textures_per_shader_stage.max(images);
    limits.max_samplers_per_shader_stage = 1;
    limits.max_bindings_per_bind_group = limits.max_bindings_per_bind_group.max(images.saturating_add(1));
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("isthmus"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        })
        .await?;
    let gpu = Gpu::new::<P>(instance, adapter, device, queue, format);
    gpu.check()?;
    Ok(gpu)
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
        None => caps
            .formats
            .iter()
            .copied()
            .find(|format| matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or(SetupError::UnsupportedSurface)?,
    };
    // WebGPU supports premultiplied canvases, although wgpu's web capabilities only list Opaque.
    let alpha_mode = if cfg!(target_arch = "wasm32") {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Auto, wgpu::CompositeAlphaMode::Opaque]
            .into_iter()
            .find(|mode| caps.alpha_modes.contains(mode))
            .or_else(|| caps.alpha_modes.first().copied())
            .ok_or(SetupError::UnsupportedSurface)?
    };
    Ok(wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: width.clamp(1, adapter.limits().max_texture_dimension_2d),
        height: height.clamp(1, adapter.limits().max_texture_dimension_2d),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    })
}
