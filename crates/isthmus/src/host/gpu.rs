//! The device and queue, the per-frame storage the shaders read, and recording draws into it.
use super::{
    arena::Arena, buffer::UploadBuffer, error::RenderError, images::ImageCache, pipeline_cache::DiskCache,
    pipelines::Pipelines, surface::SurfaceTarget,
};
use crate::{Image, Program, ShaderData};
use core::{any::type_name, array::from_fn, mem};
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
    /// Draw records and the words they share for the frame being recorded, uploaded once by [`Gpu::prepare`].
    pub arena: Arena,
    /// A record's words, kept between draws so that each one needn't allocate.
    pub(super) scratch: Vec<u32>,
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
        let cache = DiskCache::open(&device, &adapter, type_name::<P>());
        let pipelines = Pipelines::new::<P>(&device, format, &frame_layout, &images.layouts, cache);
        Self {
            device_name: adapter.get_info().name,
            buffers: [("isthmus frame", false), ("isthmus shared", true)]
                .map(|(label, skip_unchanged)| UploadBuffer::new(&device, label, skip_unchanged)),
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
            arena: Arena::default(),
            scratch: Vec::new(),
            binding: None,
        }
    }

    /// The first asynchronous device failure, if any; device loss takes precedence over validation errors.
    pub(super) fn check(&self) -> Result<(), RenderError> {
        self.errors.loss.get().or_else(|| self.errors.validation.get()).cloned().map_or(Ok(()), Err)
    }

    pub fn begin_frame(&mut self) {
        self.arena.begin_frame();
        self.images.retain_live();
    }

    /// Takes the scratch words for a new record, zeroed up to `len` words.
    pub(crate) fn scratch(&mut self, len: usize) -> Vec<u32> {
        let mut words = mem::take(&mut self.scratch);
        words.clear();
        words.resize(len, 0);
        words
    }

    /// Fails the frame being recorded, keeping the first failure for [`Gpu::prepare`] to report.
    pub(crate) fn fail(&mut self, error: RenderError) {
        self.record_error.get_or_insert(error);
    }

    /// Records one draw of `shader` with `payload` as its record, batching it with the previous draw when possible.
    pub(crate) fn emit(&mut self, surface: &mut SurfaceTarget, shader: usize, payload: &[u32], images: &[&Image]) {
        let image = if images.is_empty() {
            None
        } else {
            match self.images.images(&self.device, &self.queue, images) {
                Ok(binding) => Some(binding),
                Err(error) => {
                    self.fail(error);
                    return;
                }
            }
        };
        // A shader's fixed-width records are addressed directly by instance index.
        // Slice uploads may precede them; padding keeps every record aligned to its stride.
        let stride = payload.len() + 1;
        let offset = self.arena.words.len().next_multiple_of(stride);
        self.arena.words.resize(offset, 0);
        surface.frame.append(&mut self.arena.words);
        self.arena.words.extend_from_slice(payload);
        surface.push_draw(shader, (offset / stride) as u32, image);
    }

    /// Uploads the frame's records, ready for [`Gpu::draw_surface`].
    pub fn prepare(&mut self) -> Result<(), RenderError> {
        if let Some(error) = self.record_error.take() {
            return Err(error);
        }
        self.buffers[abi::FRAME_BINDING as usize].upload(&self.device, &self.queue, &self.arena.words)?;
        self.buffers[abi::SHARED_BINDING as usize].upload(&self.device, &self.queue, &self.arena.shared)?;
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
            // Names each run in GPU captures, such as RenderDoc's event list, without costing release builds.
            #[cfg(debug_assertions)]
            pass.insert_debug_marker(P::SHADERS[paint.shader].name);
            pass.set_pipeline(self.pipelines.get::<P>(paint.shader));
            if let Some(image) = &paint.image {
                pass.set_bind_group(abi::IMAGE_SET, image, &[]);
            }
            pass.draw(0..4, paint.draws.clone());
        }
    }
}
