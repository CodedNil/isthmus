use crate::{contract, data::ShaderData};
use core::{error::Error, fmt};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{
    rc::Rc,
    string::{String, ToString},
    vec,
};

pub const IMAGE_CAPACITY: u32 = contract::IMAGE_CAPACITY as u32;

#[derive(Clone, Default)]
pub struct BufferRange {
    pub(super) raw: Option<wgpu::Buffer>,
    pub(super) offset: u64,
}

#[derive(Debug)]
pub enum SetupError {
    Handle(raw_window_handle::HandleError),
    Adapter,
    Device(String),
    Surface(String),
    UnsupportedSurface,
    IncompatibleSurface,
    InvalidShader,
    MissingFeature(&'static str),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handle(e) => e.fmt(f),
            Self::Adapter => f.write_str("no suitable GPU adapter found"),
            Self::Device(e) => write!(f, "GPU device error: {e}"),
            Self::Surface(e) => write!(f, "surface error: {e}"),
            Self::UnsupportedSurface => f.write_str("surface is unsupported"),
            Self::IncompatibleSurface => f.write_str("replacement surface is incompatible"),
            Self::InvalidShader => f.write_str("shader is not valid SPIR-V"),
            Self::MissingFeature(feature) => write!(f, "GPU is missing required feature `{feature}`"),
        }
    }
}
impl Error for SetupError {}
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
    // The caller owns the display/window handles for the lifetime of the
    // returned surface; wgpu cannot express that relationship for generic
    // raw handles, so this is the required API boundary.
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
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|_| SetupError::Adapter)?;
        let required = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::IMMEDIATES
            | wgpu::Features::PASSTHROUGH_SHADERS;
        let supported = adapter.features();
        if !supported.contains(required) {
            return Err(SetupError::MissingFeature("descriptor arrays and immediate data"));
        }
        let mut limits = adapter.limits();
        limits.max_binding_array_elements_per_shader_stage = IMAGE_CAPACITY;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("isthmus"),
            required_features: required,
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| SetupError::Device(e.to_string()))?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("isthmus image sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let device_name = adapter.get_info().name;
        Ok((
            Self(Rc::new(Inner {
                instance,
                adapter,
                device,
                queue,
                sampler,
                device_name,
            })),
            surface,
        ))
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn new(
        _source: &(impl HasDisplayHandle + HasWindowHandle),
    ) -> Result<(Self, wgpu::Surface<'static>), SetupError> {
        Err(SetupError::UnsupportedSurface)
    }

    pub(crate) fn upload<T: ShaderData>(&self, values: &[T]) -> BufferRange {
        self.upload_bytes(bytemuck::cast_slice(values))
    }
    pub(super) fn upload_bytes(&self, bytes: &[u8]) -> BufferRange {
        let size = bytes.len().max(4) as u64;
        let buffer = self.0.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("isthmus upload"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !bytes.is_empty() {
            self.0.queue.write_buffer(&buffer, 0, bytes);
        }
        BufferRange {
            raw: Some(buffer),
            offset: 0,
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
            .or_else(|| {
                caps.formats
                    .iter()
                    .copied()
                    .find(|format| *format == wgpu::TextureFormat::Rgba8Unorm)
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or(SetupError::UnsupportedSurface)?;
        if !caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            return Err(SetupError::UnsupportedSurface);
        }
        Ok(wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        })
    }
}
