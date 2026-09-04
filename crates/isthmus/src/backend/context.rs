use crate::data::ShaderData;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::rc::Rc;

#[derive(Clone, Default)]
pub struct BufferRange {
    pub(super) raw: Option<wgpu::Buffer>,
    capacity: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error(transparent)]
    Handle(raw_window_handle::HandleError),
    #[error("GPU adapter error: {0}")]
    Adapter(String),
    #[error("GPU device error: {0}")]
    Device(String),
    #[error("surface error: {0}")]
    Surface(String),
    #[error("surface is unsupported")]
    UnsupportedSurface,
    #[error("replacement surface is incompatible")]
    IncompatibleSurface,
    #[error("shader is not valid SPIR-V")]
    InvalidShader,
    #[error("WebGPU is unavailable in this browser")]
    WebGpuUnavailable,
}
impl From<raw_window_handle::HandleError> for SetupError {
    fn from(e: raw_window_handle::HandleError) -> Self {
        Self::Handle(e)
    }
}

pub(super) struct Inner {
    pub(super) instance: wgpu::Instance,
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) sampler: wgpu::Sampler,
    pub(super) device_name: String,
}

#[derive(Clone)]
pub struct Context(pub(super) Rc<Inner>);

pub(super) fn create_surface(
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
    .map_err(|e| SetupError::Surface(e.to_string()))
}

impl Context {
    #[cfg(target_os = "linux")]
    pub(super) fn new(
        source: &(impl HasDisplayHandle + HasWindowHandle),
    ) -> Result<(Self, wgpu::Surface<'static>), SetupError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = create_surface(&instance, source)?;
        pollster::block_on(Self::finish(instance, surface, wgpu::PowerPreference::HighPerformance))
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<(Self, wgpu::Surface<'static>), SetupError> {
        if !wgpu::util::is_browser_webgpu_supported().await {
            return Err(SetupError::WebGpuUnavailable);
        }
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|error| SetupError::Surface(error.to_string()))?;
        Self::finish(instance, surface, wgpu::PowerPreference::None).await
    }

    async fn finish(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        power_preference: wgpu::PowerPreference,
    ) -> Result<(Self, wgpu::Surface<'static>), SetupError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| SetupError::Adapter(error.to_string()))?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("isthmus"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|error| SetupError::Device(error.to_string()))?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("isthmus image sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let device_name = info.name;
        Ok((Self(Rc::new(Inner { instance, adapter, device, queue, sampler, device_name })), surface))
    }

    pub(crate) fn upload<T: ShaderData>(&self, values: &[T]) -> BufferRange {
        self.upload_bytes(bytemuck::cast_slice(values))
    }

    pub(crate) fn upload_into<T: ShaderData>(&self, target: &mut BufferRange, values: &[T]) {
        self.upload_bytes_into(target, bytemuck::cast_slice(values));
    }

    pub(super) fn upload_bytes(&self, bytes: &[u8]) -> BufferRange {
        let mut range = BufferRange::default();
        self.upload_bytes_into(&mut range, bytes);
        range
    }

    pub(super) fn upload_bytes_into(&self, target: &mut BufferRange, bytes: &[u8]) {
        let size = bytes.len().max(4) as u64;
        if target.capacity < size {
            let capacity = size.next_power_of_two();
            target.raw = Some(self.0.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("isthmus upload"),
                size: capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            target.capacity = capacity;
        }
        if !bytes.is_empty() {
            self.0.queue.write_buffer(target.raw.as_ref().unwrap(), 0, bytes);
        }
    }

    pub(super) fn configure_surface(
        &self,
        surface: &wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<wgpu::SurfaceConfiguration, SetupError> {
        let caps = surface.get_capabilities(&self.0.adapter);
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
}
