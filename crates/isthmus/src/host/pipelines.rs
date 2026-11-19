//! Render pipelines, all compiled at startup so that no draw waits on one.
use super::pipeline_cache::DiskCache;
use crate::{Blend, Program, program::ShaderEntry};
use web_time::Instant;

/// Compiles the program's pipelines in shader order, sharing a layout per image count.
pub(super) fn compile_all<P: Program>(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    frame_layout: &wgpu::BindGroupLayout,
    image_layouts: &[wgpu::BindGroupLayout],
    cache: Option<DiskCache>,
) -> Vec<wgpu::RenderPipeline> {
    let layouts: Vec<_> = image_layouts
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
    let driver_cache = cache.as_ref().map(|disk| &disk.cache);
    let compile = |shader: usize| compile(device, format, &layouts, driver_cache, &P::SHADERS[shader]);
    #[cfg(target_arch = "wasm32")]
    let pipelines = (0..P::SHADERS.len()).map(compile).collect();
    #[cfg(not(target_arch = "wasm32"))]
    let pipelines = compile_in_parallel(P::SHADERS.len(), compile);
    if let Some(cache) = cache {
        cache.save();
    }
    pipelines
}

#[cfg(not(target_arch = "wasm32"))]
fn compile_in_parallel(
    count: usize,
    compile: impl Fn(usize) -> wgpu::RenderPipeline + Sync,
) -> Vec<wgpu::RenderPipeline> {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::{sync::OnceLock, thread};

    let slots: Vec<_> = (0..count).map(|_| OnceLock::new()).collect();
    let threads = thread::available_parallelism().map_or(1, |threads| threads.get().min(4));
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..threads.min(count) {
            scope.spawn(|| {
                while let shader = next.fetch_add(1, Ordering::Relaxed)
                    && let Some(slot) = slots.get(shader)
                {
                    slot.get_or_init(|| compile(shader));
                }
            });
        }
    });
    slots.into_iter().map(|slot| slot.into_inner().expect("each shader was compiled")).collect()
}

fn compile(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layouts: &[wgpu::PipelineLayout],
    cache: Option<&wgpu::PipelineCache>,
    entry: &ShaderEntry,
) -> wgpu::RenderPipeline {
    let started = Instant::now();
    let [vertex, fragment] = entry.code.map(|code| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(entry.name),
            source: wgpu::ShaderSource::Wgsl(code.into()),
        })
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry.name),
        layout: Some(&layouts[entry.images]),
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
                format: entry.format.unwrap_or(format),
                blend: blend_state(entry.blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache,
    });
    tracing::debug!(shader = entry.name, elapsed = ?started.elapsed(), "Compiled pipeline");
    pipeline
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
