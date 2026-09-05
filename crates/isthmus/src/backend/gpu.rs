use super::{buffer::UploadBuffer, image::ImageCache};
use crate::{
    Blend, Image, Program, bindings,
    contract::{DrawRecord, Primitive, ShaderSpec},
    text::PlacedGlyph,
};
use core::array::from_fn;
use std::ops::Range;
#[cfg(not(target_arch = "wasm32"))]
use wgpu::util::make_spirv;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub device_name: String,
    pub format: wgpu::TextureFormat,
    pipelines: Vec<(wgpu::RenderPipeline, u32)>,
    bind_layout: wgpu::BindGroupLayout,
    draws: Vec<DrawRecord>,
    payload: Vec<u8>,
    pub buffers: [UploadBuffer; 5],
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
        let images = ImageCache::new(&device, &queue);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("isthmus"),
            bind_group_layouts: &[Some(&bind_layout), Some(&images.layout)],
            immediate_size: 0,
        });
        let pipelines = P::SHADERS
            .iter()
            .map(|entry| {
                let (vertex, topology, count) = match entry.primitive {
                    Primitive::Quad => ("isthmus_quad", wgpu::PrimitiveTopology::TriangleStrip, 4),
                    Primitive::Triangle => ("isthmus_triangle", wgpu::PrimitiveTopology::TriangleList, 3),
                };
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
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry.name),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vertex),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry.name),
                        targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                (pipeline, count)
            })
            .collect();
        let buffers = from_fn(|_| UploadBuffer::new(&device));
        let device_name = adapter.get_info().name;
        Self {
            instance,
            adapter,
            device,
            queue,
            device_name,
            format,
            pipelines,
            bind_layout,
            draws: Vec::new(),
            payload: Vec::new(),
            buffers,
            images,
        }
    }

    pub fn image(&mut self, image: &Image) -> wgpu::BindGroup {
        self.images.image(&self.device, &self.queue, image)
    }

    pub fn begin_frame(&mut self) {
        self.draws.clear();
        self.payload.clear();
        self.images.retain_live();
    }

    pub fn emit<S: ShaderSpec>(
        &mut self,
        surface: &mut SurfacePaints,
        geometry: impl IntoIterator<Item = [glam::Vec2; 3]>,
        value: S,
        image: Option<wgpu::BindGroup>,
    ) {
        const {
            assert!(size_of::<S>().is_multiple_of(4), "shader payload must use whole words");
        }
        let start = self.draws.len() as u32;
        let payload = self.payload.len() as u32;
        self.draws.extend(geometry.into_iter().map(|geometry| DrawRecord { geometry, payload, _padding: 0 }));
        let end = self.draws.len() as u32;
        if start == end {
            return;
        }
        self.payload.extend_from_slice(bytemuck::bytes_of(&value));
        if let Some(previous) = surface.paints.last_mut()
            && previous.shader == S::INDEX
            && previous.image == image
            && previous.draws.end == start
        {
            previous.draws.end = end;
        } else {
            surface.paints.push(Paint { shader: S::INDEX, draws: start..end, image });
        }
    }

    pub fn prepare(&mut self, placed: &[PlacedGlyph]) {
        self.buffers[bindings::DRAWS as usize].upload(&self.device, &self.queue, &self.draws);
        self.buffers[bindings::PAYLOAD as usize].bytes(&self.device, &self.queue, &self.payload);
        self.buffers[bindings::PLACED_GLYPHS as usize].upload(&self.device, &self.queue, placed);
    }

    pub fn draw_surface(&self, pass: &mut wgpu::RenderPass<'_>, surface: &SurfacePaints) {
        let entries = from_fn::<_, { bindings::BUFFER_COUNT }, _>(|index| wgpu::BindGroupEntry {
            binding: index as u32,
            resource: match index as u32 {
                bindings::GLOBALS => surface.globals.binding(),
                bindings::FRAMES => surface.frame.binding(),
                _ => self.buffers[index].binding(),
            },
        });
        let frame = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &self.bind_layout,
            entries: &entries,
        });
        pass.set_bind_group(0, &frame, &[]);
        for paint in &surface.paints {
            let (pipeline, vertices) = &self.pipelines[paint.shader];
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, paint.image.as_ref().unwrap_or(&self.images.fallback), &[]);
            pass.draw(0..*vertices, paint.draws.clone());
        }
    }
}

pub(super) struct Paint {
    shader: usize,
    draws: Range<u32>,
    image: Option<wgpu::BindGroup>,
}

pub struct SurfacePaints {
    pub(super) paints: Vec<Paint>,
    pub(super) globals: UploadBuffer,
    pub(super) frame: UploadBuffer,
}

impl SurfacePaints {
    pub fn new(device: &wgpu::Device) -> Self {
        Self { paints: Vec::new(), globals: UploadBuffer::new(device), frame: UploadBuffer::new(device) }
    }
}
