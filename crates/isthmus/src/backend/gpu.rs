use super::{buffer::UploadBuffer, image::ImageCache, surface::SurfaceTarget};
use crate::{Blend, Image, Program, ResourceData, ShaderData as _, bindings, program::ShaderSpec};
use core::array::from_fn;
use std::ops::Range;
#[cfg(not(target_arch = "wasm32"))]
use wgpu::util::make_spirv;

#[doc(hidden)]
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub device_name: String,
    pub format: wgpu::TextureFormat,
    pipelines: Vec<wgpu::RenderPipeline>,
    bind_layout: wgpu::BindGroupLayout,
    pub buffers: [UploadBuffer; 4],
    images: ImageCache,
}

impl Gpu {
    pub fn new<P: Program>(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let source = make_spirv(P::CODE);
        #[cfg(target_arch = "wasm32")]
        let source = wgpu::ShaderSource::Wgsl(str::from_utf8(P::CODE).expect("build produced invalid WGSL").into());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("isthmus"), source });
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
        let images = ImageCache::new(&device, P::SHADERS.iter().map(|entry| entry.images).max().unwrap_or(0));
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
        let pipelines = P::SHADERS
            .iter()
            .map(|entry| {
                let blend = match entry.blend {
                    Blend::Over => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    Blend::Replace => None,
                    Blend::Add => Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING.alpha,
                    }),
                };
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry.name),
                    layout: Some(&layouts[entry.images]),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(entry.vertex),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry.name),
                        targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
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
                })
            })
            .collect();
        let buffers = from_fn(|_| UploadBuffer::new(&device));
        let device_name = adapter.get_info().name;
        Self { instance, adapter, device, queue, device_name, format, pipelines, bind_layout, buffers, images }
    }

    pub fn images(&mut self, images: &[&Image]) -> wgpu::BindGroup {
        self.images.images(&self.device, &self.queue, images)
    }

    #[doc(hidden)]
    pub fn capture_buffer<T: crate::ShaderData>(&mut self, buffer: crate::Buffer<'_, T>) -> [u32; 2] {
        let payload = &mut self.buffers[bindings::PAYLOAD as usize];
        let range = [
            u32::try_from(payload.words.len()).expect("payload exceeds u32"),
            u32::try_from(buffer.values.len()).expect("buffer exceeds u32"),
        ];
        for &value in buffer.values {
            value.append(&mut payload.words);
        }
        range
    }

    pub fn begin_frame(&mut self) {
        self.buffers[bindings::DRAWS as usize].words.clear();
        self.buffers[bindings::PAYLOAD as usize].words.clear();
        self.images.retain_live();
    }

    pub(crate) fn emit<S: ShaderSpec>(
        &mut self,
        surface: &mut SurfaceTarget,
        vertices: u32,
        value: S,
        image: Option<wgpu::BindGroup>,
    ) {
        let payload = value.append(&mut self.buffers[bindings::PAYLOAD as usize].words);
        let draws = &mut self.buffers[bindings::DRAWS as usize];
        let start = payload.append(&mut draws.words);
        let end = start + 1;
        if let Some(previous) = surface.paints.last_mut()
            && previous.shader == S::INDEX
            && previous.vertices == vertices
            && previous.image == image
            && previous.draws.end == start
        {
            previous.draws.end = end;
        } else {
            surface.paints.push(Paint { shader: S::INDEX, vertices, draws: start..end, image });
        }
    }

    pub fn prepare(&mut self, resources: ResourceData<'_>) {
        self.buffers[bindings::DRAWS as usize].flush(&self.device, &self.queue);
        self.buffers[bindings::PAYLOAD as usize].flush(&self.device, &self.queue);
        self.buffers[bindings::TRANSIENT as usize].upload_if_changed(&self.device, &self.queue, resources.transient);
        self.buffers[bindings::PERSISTENT as usize].upload_appended(&self.device, &self.queue, resources.persistent);
    }

    pub(crate) fn draw_surface(&self, pass: &mut wgpu::RenderPass<'_>, surface: &mut SurfaceTarget) {
        let buffers = from_fn::<_, { bindings::BUFFER_COUNT }, _>(|index| match index as u32 {
            bindings::GLOBALS => &surface.globals,
            bindings::FRAMES => &surface.frame,
            _ => &self.buffers[index],
        });
        if surface
            .binding
            .as_ref()
            .is_none_or(|(previous, _)| buffers.iter().zip(previous).any(|(buffer, old)| buffer.buffer != *old))
        {
            let entries = from_fn::<_, { bindings::BUFFER_COUNT }, _>(|index| wgpu::BindGroupEntry {
                binding: index as u32,
                resource: buffers[index].buffer.as_entire_binding(),
            });
            let frame = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frame"),
                layout: &self.bind_layout,
                entries: &entries,
            });
            surface.binding = Some((buffers.map(|buffer| buffer.buffer.clone()), frame));
        }
        pass.set_bind_group(0, &surface.binding.as_ref().unwrap().1, &[]);
        for paint in &surface.paints {
            pass.set_pipeline(&self.pipelines[paint.shader]);
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
