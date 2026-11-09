use super::{buffer::UploadBuffer, image::ImageCache, renderer::RenderError, surface::SurfaceTarget};
use crate::{Blend, Image, Program, ResourceData, ShaderData, bindings};
use core::array::from_fn;
use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};
use web_time::Instant;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub device_name: String,
    pub format: wgpu::TextureFormat,
    pipelines: Vec<OnceLock<wgpu::RenderPipeline>>,
    layouts: Vec<wgpu::PipelineLayout>,
    bind_layout: wgpu::BindGroupLayout,
    pub words: Vec<u32>,
    buffers: [UploadBuffer; bindings::BUFFER_COUNT],
    binding: Option<([wgpu::Buffer; bindings::BUFFER_COUNT], wgpu::BindGroup)>,
    pub(super) images: ImageCache,
    record_error: Option<RenderError>,
    errors: Arc<GpuErrors>,
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
            let _ = uncaptured.validation.set(RenderError::Gpu(error.to_string()));
        }));
        let lost = Arc::clone(&errors);
        device.set_device_lost_callback(move |reason, message| {
            let _ = lost.loss.set(RenderError::DeviceLost(format!("{reason:?}: {message}")));
        });
        let entries = from_fn::<_, { bindings::BUFFER_COUNT }, _>(|binding| wgpu::BindGroupLayoutEntry {
            binding: binding as u32,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let bind_layout = device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("frame"), entries: &entries });
        let images = ImageCache::new(&device, P::max_images());
        let layouts: Vec<_> = images
            .layouts
            .iter()
            .enumerate()
            .map(|(count, layout)| {
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("isthmus"),
                    bind_group_layouts: &[Some(&bind_layout), Some(layout)][..if count == 0 { 1 } else { 2 }],
                    immediate_size: 0,
                })
            })
            .collect();
        let labels: [&str; bindings::BUFFER_COUNT] = ["isthmus frame", "isthmus resources"];
        let buffers = from_fn(|index| UploadBuffer::new(&device, labels[index]));
        let device_name = adapter.get_info().name;
        let gpu = Self {
            instance,
            adapter,
            device,
            queue,
            device_name,
            format,
            pipelines: (0..P::SHADERS.len()).map(|_| OnceLock::new()).collect(),
            layouts,
            bind_layout,
            buffers,
            images,
            record_error: None,
            errors,
            words: Vec::new(),
            binding: None,
        };
        // Browsers have no threads, so there pipelines stay lazy.
        #[cfg(not(target_arch = "wasm32"))]
        gpu.compile_pipelines::<P>();
        gpu
    }

    /// Compiles every pipeline now, a few at a time. Each takes tens to hundreds of milliseconds, so leaving them
    /// to the first draw that needs one hitches that frame.
    #[cfg(not(target_arch = "wasm32"))]
    fn compile_pipelines<P: Program>(&self) {
        use std::{
            sync::atomic::{AtomicUsize, Ordering},
            thread,
        };
        let threads = thread::available_parallelism().map_or(1, |threads| threads.get().min(4));
        let next = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    while let shader = next.fetch_add(1, Ordering::Relaxed)
                        && shader < P::SHADERS.len()
                    {
                        self.pipeline::<P>(shader);
                    }
                });
            }
        });
    }

    fn pipeline<P: Program>(&self, shader: usize) -> &wgpu::RenderPipeline {
        self.pipelines[shader].get_or_init(|| {
            let started = Instant::now();
            let entry = &P::SHADERS[shader];
            let [vertex, fragment] = entry.code.map(|code| {
                self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(entry.name),
                    source: wgpu::ShaderSource::Wgsl(code.into()),
                })
            });
            let blend = match entry.blend {
                Blend::Over => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                Blend::Replace | Blend::Raw => None,
                Blend::Add => Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING.alpha,
                }),
            };
            let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry.name),
                layout: Some(&self.layouts[entry.images]),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: None,
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: None,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: entry.format.unwrap_or(self.format),
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            tracing::debug!(shader = entry.name, elapsed = ?started.elapsed(), "Compiled pipeline");
            pipeline
        })
    }

    pub(super) fn check(&self) -> Result<(), RenderError> {
        self.errors.loss.get().or_else(|| self.errors.validation.get()).cloned().map_or(Ok(()), Err)
    }

    pub fn begin_frame(&mut self) {
        self.words.clear();
        self.images.retain_live();
    }

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
        let start = (offset / stride) as u32;
        let end = start + 1;
        if let Some(previous) = surface.paints.last_mut()
            && previous.shader == shader
            && previous.vertices == vertices
            && previous.image == image
            && previous.draws.end == start
        {
            previous.draws.end = end;
        } else {
            surface.paints.push(Paint { shader, vertices, draws: start..end, image });
        }
    }

    pub fn prepare(&mut self, resources: ResourceData<'_>) -> Result<(), RenderError> {
        if let Some(error) = self.record_error.take() {
            return Err(error);
        }
        self.buffers[bindings::FRAME as usize].upload(&self.device, &self.queue, &self.words)?;
        self.buffers[bindings::RESOURCES as usize].upload(&self.device, &self.queue, resources)?;
        let buffers = self.buffers.each_ref().map(|buffer| buffer.buffer.clone());
        if self.binding.as_ref().is_none_or(|(old, _)| *old != buffers) {
            let entries = from_fn::<_, { bindings::BUFFER_COUNT }, _>(|index| wgpu::BindGroupEntry {
                binding: index as u32,
                resource: buffers[index].as_entire_binding(),
            });
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frame"),
                layout: &self.bind_layout,
                entries: &entries,
            });
            self.binding = Some((buffers, binding));
        }
        self.check()
    }

    pub(crate) fn draw_surface<P: Program>(&self, pass: &mut wgpu::RenderPass<'_>, surface: &SurfaceTarget) {
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        for paint in &surface.paints {
            pass.set_pipeline(self.pipeline::<P>(paint.shader));
            if let Some(image) = &paint.image {
                pass.set_bind_group(1, image, &[]);
            }
            pass.draw(0..paint.vertices, paint.draws.clone());
        }
    }
}

pub(super) struct Paint {
    shader: usize,
    vertices: u32,
    draws: Range<u32>,
    image: Option<wgpu::BindGroup>,
}

#[derive(Default)]
struct GpuErrors {
    loss: OnceLock<RenderError>,
    validation: OnceLock<RenderError>,
}
