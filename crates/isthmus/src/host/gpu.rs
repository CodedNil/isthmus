//! The device and queue, the per-frame storage the shaders read, and recording draws into it.
use super::{
    buffer::UploadBuffer, error::RenderError, images::ImageCache, pipelines::Pipelines, surface::SurfaceTarget,
};
use crate::{Image, Program, ResourceData, ShaderData};
use core::array::from_fn;
use isthmus_abi as abi;
use std::sync::{Arc, OnceLock};

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub device_name: String,
    pub format: wgpu::TextureFormat,
    pipelines: Pipelines,
    frame_layout: wgpu::BindGroupLayout,
    /// Draw records for the frame being recorded, uploaded once by [`Gpu::prepare`].
    pub words: Vec<u32>,
    buffers: [UploadBuffer; abi::BUFFER_COUNT],
    binding: Option<([wgpu::Buffer; abi::BUFFER_COUNT], wgpu::BindGroup)>,
    pub(super) images: ImageCache,
    record_error: Option<RenderError>,
    errors: Arc<GpuErrors>,
}

/// Asynchronous failures reported by the device, kept until the renderer next checks.
#[derive(Default)]
struct GpuErrors {
    loss: OnceLock<RenderError>,
    validation: OnceLock<RenderError>,
}

impl Gpu {
    pub fn new<P: Program>(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let errors = Arc::new(GpuErrors::default());
        let uncaptured = Arc::clone(&errors);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            drop(uncaptured.validation.set(RenderError::Gpu(error.to_string())));
        }));
        let lost = Arc::clone(&errors);
        device.set_device_lost_callback(move |reason, message| {
            drop(lost.loss.set(RenderError::DeviceLost(format!("{reason:?}: {message}"))));
        });
        let entries = from_fn::<_, { abi::BUFFER_COUNT }, _>(|binding| wgpu::BindGroupLayoutEntry {
            binding: binding as u32,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let frame_layout = device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("frame"), entries: &entries });
        let images = ImageCache::new(&device, P::max_images());
        let pipelines = Pipelines::new::<P>(&device, format, &frame_layout, &images.layouts);
        let labels: [&str; abi::BUFFER_COUNT] = ["isthmus frame", "isthmus resources"];
        Self {
            device_name: adapter.get_info().name,
            buffers: from_fn(|index| UploadBuffer::new(&device, labels[index])),
            instance,
            adapter,
            device,
            queue,
            format,
            pipelines,
            frame_layout,
            images,
            record_error: None,
            errors,
            words: Vec::new(),
            binding: None,
        }
    }

    /// The first asynchronous device failure, if any; device loss takes precedence over validation errors.
    pub(super) fn check(&self) -> Result<(), RenderError> {
        self.errors.loss.get().or_else(|| self.errors.validation.get()).cloned().map_or(Ok(()), Err)
    }

    pub fn begin_frame(&mut self) {
        self.words.clear();
        self.images.retain_live();
    }

    /// Records one draw of `shader` with `value` as its payload, batching it with the previous draw when possible.
    pub(crate) fn emit<T: ShaderData>(
        &mut self,
        surface: &mut SurfaceTarget,
        shader: usize,
        vertices: u32,
        value: T,
        images: &[&Image],
    ) {
        let image = if images.is_empty() {
            None
        } else {
            match self.images.images(&self.device, &self.queue, images) {
                Ok(binding) => Some(binding),
                Err(error) => {
                    self.record_error.get_or_insert(error);
                    return;
                }
            }
        };
        // A shader's fixed-width records are addressed directly by instance index.
        // Slice captures may precede them; padding keeps every record aligned to its stride.
        let stride = T::WORDS + 1;
        let offset = self.words.len().next_multiple_of(stride);
        self.words.resize(offset, 0);
        surface.frame.append(&mut self.words);
        value.append(&mut self.words);
        surface.push_draw(shader, vertices, (offset / stride) as u32, image);
    }

    /// Uploads the frame's records and the application's resources, ready for [`Gpu::draw_surface`].
    pub fn prepare(&mut self, resources: ResourceData<'_>) -> Result<(), RenderError> {
        if let Some(error) = self.record_error.take() {
            return Err(error);
        }
        self.buffers[abi::FRAME_BINDING as usize].upload(&self.device, &self.queue, &self.words)?;
        self.buffers[abi::RESOURCES_BINDING as usize].upload(&self.device, &self.queue, resources)?;
        let buffers = self.buffers.each_ref().map(|buffer| buffer.buffer.clone());
        if self.binding.as_ref().is_none_or(|(old, _)| *old != buffers) {
            let entries = from_fn::<_, { abi::BUFFER_COUNT }, _>(|index| wgpu::BindGroupEntry {
                binding: index as u32,
                resource: buffers[index].as_entire_binding(),
            });
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frame"),
                layout: &self.frame_layout,
                entries: &entries,
            });
            self.binding = Some((buffers, group));
        }
        self.check()
    }

    pub(crate) fn draw_surface<P: Program>(&self, pass: &mut wgpu::RenderPass<'_>, surface: &SurfaceTarget) {
        let (_, frame) = self.binding.as_ref().expect("prepare runs before drawing");
        pass.set_bind_group(abi::FRAME_SET, frame, &[]);
        for paint in surface.paints() {
            pass.set_pipeline(self.pipelines.get::<P>(paint.shader));
            if let Some(image) = &paint.image {
                pass.set_bind_group(abi::IMAGE_SET, image, &[]);
            }
            pass.draw(0..paint.vertices, paint.draws.clone());
        }
    }
}
