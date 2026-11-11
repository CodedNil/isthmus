//! Render pipelines, compiled lazily per shader (or eagerly in parallel where threads exist).
use crate::{Blend, Program};
use std::sync::OnceLock;
use web_time::Instant;

/// The program's pipelines, one per shader, sharing a layout per image count.
pub(super) struct Pipelines {
    device: wgpu::Device,
    format: wgpu::TextureFormat,
    cells: Vec<OnceLock<wgpu::RenderPipeline>>,
    layouts: Vec<wgpu::PipelineLayout>,
}

impl Pipelines {
    /// Prepares pipeline slots; `image_layouts[n]` is the image bind group layout for `n` captured images.
    pub fn new<P: Program>(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        frame_layout: &wgpu::BindGroupLayout,
        image_layouts: &[wgpu::BindGroupLayout],
    ) -> Self {
        let layouts = image_layouts
            .iter()
            .enumerate()
            .map(|(images, layout)| {
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("isthmus"),
                    bind_group_layouts: &[Some(frame_layout), Some(layout)][..if images == 0 { 1 } else { 2 }],
                    immediate_size: 0,
                })
            })
            .collect();
        let this = Self {
            device: device.clone(),
            format,
            cells: (0..P::SHADERS.len()).map(|_| OnceLock::new()).collect(),
            layouts,
        };
        // Browsers have no threads, so there pipelines stay lazy.
        #[cfg(not(target_arch = "wasm32"))]
        this.compile_all::<P>();
        this
    }

    /// Compiles every pipeline now, a few at a time. Each takes tens to hundreds of milliseconds, so leaving them
    /// to the first draw that needs one hitches that frame.
    #[cfg(not(target_arch = "wasm32"))]
    fn compile_all<P: Program>(&self) {
        use core::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;
        let threads = thread::available_parallelism().map_or(1, |threads| threads.get().min(4));
        let next = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    while let shader = next.fetch_add(1, Ordering::Relaxed)
                        && shader < P::SHADERS.len()
                    {
                        self.get::<P>(shader);
                    }
                });
            }
        });
    }

    /// The pipeline for a shader, compiling it on first use.
    pub fn get<P: Program>(&self, shader: usize) -> &wgpu::RenderPipeline {
        self.cells[shader].get_or_init(|| self.compile::<P>(shader))
    }

    fn compile<P: Program>(&self, shader: usize) -> wgpu::RenderPipeline {
        let started = Instant::now();
        let entry = &P::SHADERS[shader];
        let [vertex, fragment] = entry.code.map(|code| {
            self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(entry.name),
                source: wgpu::ShaderSource::Wgsl(code.into()),
            })
        });
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
                    blend: blend_state(entry.blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        tracing::debug!(shader = entry.name, elapsed = ?started.elapsed(), "Compiled pipeline");
        pipeline
    }
}

const fn blend_state(blend: Blend) -> Option<wgpu::BlendState> {
    match blend {
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
    }
}
