use super::{
    context::{BufferRange, Context},
    image::ImageCache,
};
use crate::{
    contract::{DrawRecord, PaintPipeline, PushBlock, Quad, ShaderSpec, SurfaceHandle},
    data::ImageHandle,
};
use core::array::from_fn;
use std::{borrow::Cow, collections::HashMap, rc::Rc, sync::Arc};

pub struct Canvas {
    context: Context,
    shader: wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    pipelines: HashMap<&'static str, wgpu::RenderPipeline>,
    layout: wgpu::PipelineLayout,
    bind_layout: wgpu::BindGroupLayout,
    draws: Vec<DrawRecord>,
    payload: Vec<u8>,
    uploaded_draws: BufferRange,
    uploaded_payload: BufferRange,
    paints: Vec<Vec<Paint>>,
    images: ImageCache,
    paint_images: Vec<Rc<super::image::Image>>,
    payload_images: Option<usize>,
    payload_pipeline: Option<PaintPipeline>,
    text: [BufferRange; 3],
    globals: Vec<BufferRange>,
    globals_set: Vec<bool>,
    frames: Vec<BufferRange>,
}

impl Canvas {
    pub(crate) fn new(context: &Context, shader: &'static [u8], format: wgpu::TextureFormat) -> Self {
        let device = &context.0.device;
        #[cfg(not(target_arch = "wasm32"))]
        let source = {
            let (words, _) = shader.as_chunks::<4>();
            wgpu::ShaderSource::SpirV(Cow::Owned(words.iter().map(|bytes| u32::from_le_bytes(*bytes)).collect()))
        };
        #[cfg(target_arch = "wasm32")]
        let source =
            wgpu::ShaderSource::Wgsl(Cow::Borrowed(str::from_utf8(shader).expect("build produced invalid WGSL")));
        let shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("isthmus shader"), source });
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
            binding: 8,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        all.push(wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
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
            immediate_size: 0,
        });
        Self {
            context: context.clone(),
            shader,
            format,
            pipelines: HashMap::new(),
            layout,
            bind_layout,
            draws: Vec::new(),
            payload: Vec::new(),
            uploaded_draws: BufferRange::default(),
            uploaded_payload: BufferRange::default(),
            paints: Vec::new(),
            images: ImageCache::new(context),
            paint_images: Vec::new(),
            payload_images: None,
            payload_pipeline: None,
            text: from_fn(|_| BufferRange::default()),
            globals: Vec::new(),
            globals_set: Vec::new(),
            frames: Vec::new(),
        }
    }

    pub(crate) fn image(&mut self, size: [u32; 2], pixels: &Arc<[u8]>) -> ImageHandle {
        self.payload_pipeline.expect("image captured outside a paint payload");
        let image = self.images.image(&self.context, size, pixels);
        if let Some(index) = self.payload_images {
            assert!(Rc::ptr_eq(&self.paint_images[index], &image), "one paint may capture only one image");
            return ImageHandle::new(0);
        }
        self.payload_images = Some(self.paint_images.len());
        self.paint_images.push(image);
        ImageHandle::new(0)
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
            self.globals_set.resize(i + 1, false);
        }
        self.context.upload_into(&mut self.globals[i], &[globals]);
        self.globals_set[i] = true;
    }

    pub(crate) fn ensure_globals(&mut self, surface: SurfaceHandle) {
        if !self.globals_set.get(surface.index()).copied().unwrap_or(false) {
            self.set_globals(surface, 0u32);
        }
    }

    pub(crate) fn set_frame(&mut self, surface: SurfaceHandle, frame: PushBlock) {
        let index = surface.index();
        if self.frames.len() <= index {
            self.frames.resize(index + 1, BufferRange::default());
        }
        self.context.upload_into(&mut self.frames[index], &[frame]);
    }

    pub(super) fn begin_frame(&mut self) {
        self.draws.clear();
        self.payload.clear();
        self.globals_set.fill(false);
        for paints in &mut self.paints {
            paints.clear();
        }
        self.images.begin_frame();
        self.paint_images.clear();
        self.payload_pipeline = None;
        self.payload_images = None;
    }

    pub(crate) fn emit<S: ShaderSpec>(&mut self, surface: SurfaceHandle, quad: Quad, value: S::Instance) {
        let images = self.finish_payload::<S>();
        let paint = self.record(S::PIPELINE, quad, bytemuck::bytes_of(&value), images);
        self.surface(surface).push(paint);
    }

    pub(crate) fn emit_text<S: ShaderSpec>(
        &mut self,
        surface: SurfaceHandle,
        quads: impl IntoIterator<Item = Quad>,
        value: S::Instance,
    ) {
        let images = self.finish_payload::<S>();
        let paints = self.record_text(S::PIPELINE, quads, bytemuck::bytes_of(&value), images);
        self.surface(surface).extend(paints);
    }

    fn finish_payload<S: ShaderSpec>(&mut self) -> Option<usize> {
        const {
            assert!(size_of::<S::Instance>() % 4 == 0, "shader payload size must be a multiple of four");
        };
        assert_eq!(
            self.payload_pipeline.take().map(|pipeline| pipeline.entry),
            Some(S::PIPELINE.entry),
            "payload pipeline must match the emitted shader",
        );
        self.payload_images.take()
    }

    fn record(&mut self, spec: PaintPipeline, quad: Quad, value: &[u8], images: Option<usize>) -> Paint {
        if !self.pipelines.contains_key(spec.entry) {
            let pipeline = self.context.0.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(spec.entry),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("isthmus_vertex"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some(spec.entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        // Premultiply the paint shader's straight RGBA once at this boundary.
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        self.draws.push(DrawRecord { quad, payload, _padding: 0 });
        Paint { pipeline: spec.entry, draw, images }
    }

    fn record_text(
        &mut self,
        spec: PaintPipeline,
        quads: impl IntoIterator<Item = Quad>,
        value: &[u8],
        images: Option<usize>,
    ) -> Vec<Paint> {
        let mut quads = quads.into_iter();
        let Some(first_quad) = quads.next() else {
            return Vec::new();
        };
        let first = self.record(spec, first_quad, value, images);
        let payload = self.draws[first.draw as usize].payload;
        let mut paints = vec![first];
        for quad in quads {
            let draw = self.draws.len() as u32;
            self.draws.push(DrawRecord { quad, payload, _padding: 0 });
            paints.push(Paint { pipeline: spec.entry, draw, images });
        }
        paints
    }

    fn surface(&mut self, surface: SurfaceHandle) -> &mut Vec<Paint> {
        if self.paints.len() <= surface.index() {
            self.paints.resize_with(surface.index() + 1, Vec::new);
        }
        &mut self.paints[surface.index()]
    }

    pub(crate) fn prepare(&mut self, placed_glyphs: BufferRange) {
        self.text[0] = placed_glyphs;
        self.context.upload_into(&mut self.uploaded_draws, &self.draws);
        self.context.upload_bytes_into(&mut self.uploaded_payload, &self.payload);
    }

    pub(super) fn has_draws(&self, surface: SurfaceHandle) -> bool {
        self.paints.get(surface.index()).is_some_and(|p| !p.is_empty())
    }

    pub(super) fn draw_surface(&self, pass: &mut wgpu::RenderPass<'_>, surface: SurfaceHandle) {
        let Some(paints) = self.paints.get(surface.index()) else {
            return;
        };
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
            &self.frames[surface.index()],
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
                        binding: match binding {
                            5 => 6,
                            6 => 8,
                            _ => binding as u32,
                        },
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: raw,
                            offset: 0,
                            size: None,
                        }),
                    });
                }
            }
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(
                    paint.images.map_or(&self.images.fallback().view, |image| &self.paint_images[image].view),
                ),
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
        while start < paints.len() {
            let first = paints[start];
            let mut end = start + 1;
            while end < paints.len()
                && paints[end].pipeline == first.pipeline
                && paints[end].images == first.images
                && paints[end].draw == paints[end - 1].draw + 1
            {
                end += 1;
            }
            pass.set_pipeline(&self.pipelines[first.pipeline]);
            pass.set_bind_group(0, &binds[&(first.pipeline, first.images)], &[]);
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
