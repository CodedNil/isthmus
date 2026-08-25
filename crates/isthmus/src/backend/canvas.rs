use super::{
    context::{BufferRange, Context, IMAGE_CAPACITY},
    image::ImageCache,
};
use crate::{
    contract::{DrawRecord, PaintPipeline, PushBlock, Quad, ShaderSpec, SurfaceHandle},
    data::ImageHandle,
};
use core::{array::from_fn, mem::size_of, num::NonZeroU32};
use std::{borrow::Cow, collections::HashMap, format, rc::Rc, sync::Arc, vec, vec::Vec};

pub struct Canvas {
    context: Context,
    shader: Vec<u32>,
    format: wgpu::TextureFormat,
    root: &'static str,
    pipelines: HashMap<&'static str, wgpu::RenderPipeline>,
    layout: wgpu::PipelineLayout,
    bind_layout: wgpu::BindGroupLayout,
    draws: Vec<DrawRecord>,
    payload: Vec<u8>,
    uploaded_draws: BufferRange,
    uploaded_payload: BufferRange,
    paints: Vec<Vec<Paint>>,
    group: Option<(SurfaceHandle, [Vec<Paint>; 2])>,
    images: ImageCache,
    image_tables: Vec<Vec<Rc<super::image::Image>>>,
    payload_images: Option<usize>,
    payload_pipeline: Option<PaintPipeline>,
    text: [BufferRange; 3],
    globals: Vec<BufferRange>,
}

impl Canvas {
    pub(crate) fn new(context: &Context, shader: Vec<u32>, format: wgpu::TextureFormat, root: &'static str) -> Self {
        let device = &context.0.device;
        let entries = [0, 1, 2, 3, 4, 6].map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let mut all = entries.to_vec();
        all.push(wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: Some(NonZeroU32::new(IMAGE_CAPACITY).unwrap()),
        });
        all.push(wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("isthmus bindings"),
            entries: &all,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("isthmus pipeline layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: size_of::<PushBlock>() as u32,
        });
        Self {
            context: context.clone(),
            shader,
            format,
            root,
            pipelines: HashMap::new(),
            layout,
            bind_layout,
            draws: Vec::new(),
            payload: Vec::new(),
            uploaded_draws: BufferRange::default(),
            uploaded_payload: BufferRange::default(),
            paints: Vec::new(),
            group: None,
            images: ImageCache::new(context),
            image_tables: Vec::new(),
            payload_images: None,
            payload_pipeline: None,
            text: from_fn(|_| BufferRange::default()),
            globals: Vec::new(),
        }
    }
    pub(crate) fn image(&mut self, size: [u32; 2], pixels: &Arc<[u8]>) -> ImageHandle {
        self.payload_pipeline.expect("image captured outside a paint payload");
        let image = self.images.image(&self.context, size, pixels);
        let table = if let Some(table) = self.payload_images {
            table
        } else {
            let table = self.image_tables.len();
            self.image_tables.push(Vec::new());
            self.payload_images = Some(table);
            table
        };
        let images = &mut self.image_tables[table];
        if let Some(index) = images.iter().position(|candidate| Rc::ptr_eq(candidate, &image)) {
            return ImageHandle::new(index as u32);
        }
        assert!(images.len() < IMAGE_CAPACITY as usize, "one paint shader uses more than {IMAGE_CAPACITY} images");
        let index = images.len() as u32;
        images.push(image);
        ImageHandle::new(index)
    }
    pub(crate) const fn begin_payload(&mut self, pipeline: PaintPipeline) {
        self.payload_pipeline = Some(pipeline);
    }
    pub(crate) const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }
    pub(crate) fn register_text(&mut self, glyphs: BufferRange, edges: BufferRange) {
        self.text[1] = glyphs;
        self.text[2] = edges;
    }
    pub(crate) fn set_globals<T: crate::ShaderData>(&mut self, surface: SurfaceHandle, globals: T) {
        let i = surface.index();
        if self.globals.len() <= i {
            self.globals.resize(i + 1, BufferRange::default());
        }
        self.globals[i] = self.context.upload(&[globals]);
    }
    pub(crate) fn ensure_globals(&mut self, surface: SurfaceHandle) {
        if self.globals.get(surface.index()).is_none_or(|g| g.raw.is_none()) {
            self.set_globals(surface, 0u32);
        }
    }
    pub(super) fn begin_frame(&mut self) {
        self.draws.clear();
        self.payload.clear();
        self.uploaded_draws = BufferRange::default();
        self.uploaded_payload = BufferRange::default();
        self.globals.clear();
        for paints in &mut self.paints {
            paints.clear();
        }
        self.group = None;
        self.images.begin_frame();
        self.image_tables.clear();
        self.payload_pipeline = None;
        self.payload_images = None;
    }
    pub(crate) fn emit<S: ShaderSpec>(&mut self, surface: SurfaceHandle, quad: Quad, value: S::Instance) {
        const {
            assert!(size_of::<S::Instance>() % 4 == 0);
        }
        assert_eq!(self.payload_pipeline.take().map(|pipeline| pipeline.entry), Some(S::PIPELINE.entry));
        let images = self.payload_images.take();
        let paint = self.record(S::PIPELINE, quad, bytemuck::bytes_of(&value), images);
        self.surface(surface).push(paint);
    }
    pub(crate) fn emit_layer<S: ShaderSpec>(&mut self, layer: u8, quad: Quad, value: S::Instance) {
        const {
            assert!(size_of::<S::Instance>() % 4 == 0);
        }
        assert_eq!(self.payload_pipeline.take().map(|pipeline| pipeline.entry), Some(S::PIPELINE.entry));
        let images = self.payload_images.take();
        let paint = self.record(S::PIPELINE, quad, bytemuck::bytes_of(&value), images);
        self.group.as_mut().expect("paint layer outside a group").1[layer as usize].push(paint);
    }
    fn record(&mut self, spec: PaintPipeline, quad: Quad, value: &[u8], images: Option<usize>) -> Paint {
        if !self.pipelines.contains_key(spec.entry) {
            let vertex_entry = format!("{}::__isthmus_quad::vertex", self.root);
            let fragment_entry = format!("{}::fragment", spec.entry);
            // The SPIR-V is generated from the same Rust-GPU sources as the
            // entry-point names below; passthrough is required because naga
            // cannot represent Rust-GPU's non-uniform descriptor operations.
            let module = unsafe {
                self.context.0.device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                    label: Some("isthmus shader"),
                    entry_points: Cow::Owned(vec![
                        wgpu::PassthroughShaderEntryPoint {
                            name: Cow::Owned(vertex_entry),
                            workgroup_size: (0, 0, 0),
                        },
                        wgpu::PassthroughShaderEntryPoint {
                            name: Cow::Owned(fragment_entry),
                            workgroup_size: (0, 0, 0),
                        },
                    ]),
                    spirv: Some(Cow::Borrowed(&self.shader)),
                    ..Default::default()
                })
            };
            let pipeline = self.context.0.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(spec.entry),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(&format!("{}::__isthmus_quad::vertex", self.root)),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(&format!("{}::fragment", spec.entry)),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            self.pipelines.insert(spec.entry, pipeline);
        }
        let payload = self.payload.len() as u32;
        self.payload.extend_from_slice(value);
        let draw = self.draws.len() as u32;
        self.draws.push(DrawRecord { quad, payload });
        Paint {
            pipeline: spec.entry,
            draw,
            images,
        }
    }
    fn surface(&mut self, surface: SurfaceHandle) -> &mut Vec<Paint> {
        if self.paints.len() <= surface.index() {
            self.paints.resize_with(surface.index() + 1, Vec::new);
        }
        &mut self.paints[surface.index()]
    }
    pub(crate) fn begin_group(&mut self, surface: SurfaceHandle) {
        assert!(self.group.is_none());
        self.group = Some((surface, from_fn(|_| Vec::new())));
    }
    pub(crate) fn end_group(&mut self) {
        let (surface, layers) = self.group.take().expect("paint group was not started");
        self.surface(surface).extend(layers.into_iter().flatten());
    }
    pub(crate) fn prepare(&mut self, placed_glyphs: BufferRange) {
        self.text[0] = placed_glyphs;
        self.uploaded_draws = self.context.upload(&self.draws);
        self.uploaded_payload = self.context.upload_bytes(&self.payload);
    }
    pub(super) fn has_draws(&self, surface: SurfaceHandle) -> bool {
        self.paints.get(surface.index()).is_some_and(|p| !p.is_empty())
    }
    pub(super) fn draw_surface(&self, pass: &mut wgpu::RenderPass<'_>, surface: SurfaceHandle, shared: &[u8]) {
        let Some(paints) = self.paints.get(surface.index()) else { return };
        if paints.is_empty() {
            return;
        }
        let buffers = [
            &self.uploaded_draws,
            &self.uploaded_payload,
            &self.text[0],
            &self.text[1],
            &self.text[2],
            &self.globals[surface.index()],
        ];
        let mut binds = HashMap::new();
        for paint in paints {
            if binds.contains_key(&(paint.pipeline, paint.images)) {
                continue;
            }
            let mut entries = Vec::new();
            for (binding, range) in buffers.iter().enumerate() {
                if let Some(raw) = &range.raw {
                    entries.push(wgpu::BindGroupEntry {
                        binding: if binding == 5 { 6 } else { binding as u32 },
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: raw,
                            offset: range.offset,
                            size: None,
                        }),
                    });
                }
            }
            let mut views: Vec<_> = paint
                .images
                .map(|table| self.image_tables[table].iter().map(|image| &image.view).collect::<Vec<_>>())
                .unwrap_or_default();
            views.resize(IMAGE_CAPACITY as usize, &self.images.fallback().view);
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureViewArray(&views),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(&self.context.0.sampler),
            });
            binds.insert(
                (paint.pipeline, paint.images),
                self.context.0.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(paint.pipeline),
                    layout: &self.bind_layout,
                    entries: &entries,
                }),
            );
        }
        let mut start = 0;
        let mut immediates_set = false;
        while start < paints.len() {
            let first = paints[start];
            let mut end = start + 1;
            while end < paints.len() && paints[end].pipeline == first.pipeline && paints[end].images == first.images && paints[end].draw == paints[end - 1].draw + 1 {
                end += 1;
            }
            pass.set_pipeline(&self.pipelines[first.pipeline]);
            pass.set_bind_group(0, &binds[&(first.pipeline, first.images)], &[]);
            if !immediates_set {
                pass.set_immediates(0, shared);
                immediates_set = true;
            }
            pass.draw(0..4, first.draw..first.draw + (end - start) as u32);
            start = end;
        }
    }
}
#[derive(Clone, Copy)]
struct Paint {
    pipeline: &'static str,
    draw: u32,
    images: Option<usize>,
}
