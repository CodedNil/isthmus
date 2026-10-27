use super::{
    gpu::Gpu,
    setup::{self, SetupError},
    surface::SurfaceTarget,
};
use crate::{Frame, Program, Resources, SurfaceHandle, data::FrameData, glam::Vec2};
use slotmap::SlotMap;
use smallvec::SmallVec;
use web_time::Instant;

/// GPU resources and presentation surfaces for one shader program.
pub struct Renderer<P: Program> {
    surfaces: SlotMap<SurfaceHandle, SurfaceTarget>,
    gpu: Gpu,
    resources: P::Resources,
    started: Instant,
    last_frame: f64,
    order: SmallVec<[SurfaceHandle; 4]>,
}
/// Records surfaces that will be submitted together in one frame.
pub struct Render<'a, P: Program> {
    renderer: &'a mut Renderer<P>,
    /// Seconds elapsed since renderer creation.
    pub time: f32,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
}
/// A failure while recording or presenting a frame.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// A presentation surface was lost and must be recreated.
    #[error("presentation surface was lost")]
    SurfaceLost,
    /// GPU validation rejected the frame.
    #[error("frame failed GPU validation")]
    Validation,
}
impl<P: Program> Render<'_, P> {
    /// Records a surface once per batch in logical pixels; stale handles are ignored.
    pub fn surface(
        &mut self,
        surface: SurfaceHandle,
        screen_size: Vec2,
        globals: P::Globals,
        draw: impl FnOnce(Frame<'_, P>),
    ) {
        let renderer = &mut *self.renderer;
        let handle = surface;
        let Some(surface) = renderer.surfaces.get_mut(surface) else { return };
        assert!(!surface.recorded, "record each surface once per render batch");
        surface.recorded = true;
        renderer.order.push(handle);
        let pixel_scale = screen_size / Vec2::new(surface.config.width as f32, surface.config.height as f32);
        let frame_data = FrameData { screen_size, time: self.time, pixel_scale, globals };
        draw(Frame {
            time: self.time,
            screen_size,
            pixel_size: pixel_scale.max_element(),
            delta_time: self.delta_time,
            globals,
            resources: &mut renderer.resources,
            gpu: &mut renderer.gpu,
            surface,
        });
        surface.frame.upload(&renderer.gpu.device, &renderer.gpu.queue, frame_data);
    }
}
impl<P: Program> Renderer<P> {
    /// Creates a renderer that owns its presentation target.
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: [u32; 2],
        resources: P::Resources,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let (gpu, target) = setup::new::<P>(target, size).await?;
        let mut surfaces = SlotMap::with_key();
        let handle = surfaces.insert(target);
        Ok((
            Self { surfaces, gpu, resources, started: Instant::now(), last_frame: 0.0, order: SmallVec::new() },
            handle,
        ))
    }

    /// Records and presents one frame, returning surface loss or GPU validation errors.
    pub fn render(&mut self, draw: impl FnOnce(&mut Render<'_, P>)) -> Result<(), RenderError> {
        self.gpu.begin_frame();
        self.order.clear();
        for surface in self.surfaces.values_mut() {
            surface.paints.clear();
            surface.recorded = false;
        }
        self.resources.begin_frame();
        let elapsed = self.started.elapsed().as_secs_f64();
        let delta = (elapsed - self.last_frame).min(0.1) as f32;
        self.last_frame = elapsed;
        draw(&mut Render { renderer: self, time: elapsed as f32, delta_time: delta });
        if !self.surfaces.values().any(|surface| surface.recorded) {
            return Ok(());
        }
        self.gpu.prepare(self.resources.data());
        let mut outputs = SmallVec::<[wgpu::SurfaceTexture; 2]>::new();
        let mut encoder =
            self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("isthmus frame") });
        let mut has_passes = false;
        // Recording order defines dependencies between offscreen passes.
        for handle in &self.order {
            let surface = self.surfaces.get_mut(*handle).expect("recorded surface exists");
            let output;
            let view = if let Some(view) = surface.image.as_ref() {
                output = None;
                view.clone()
            } else {
                let Some(acquired) = surface.acquire(&self.gpu)? else { continue };
                let view = acquired.texture.create_view(&wgpu::TextureViewDescriptor::default());
                output = Some(acquired);
                view
            };
            {
                has_passes = true;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(if surface.image.is_some() { "isthmus image pass" } else { "isthmus surface pass" }),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if surface.image.is_some() {
                                wgpu::LoadOp::Load
                            } else {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                self.gpu.draw_surface(&mut pass, surface);
            }
            if let Some(output) = output {
                outputs.push(output);
            }
        }
        if has_passes {
            self.gpu.queue.submit([encoder.finish()]);
            for output in outputs {
                self.gpu.queue.present(output);
            }
        }
        Ok(())
    }

    /// Returns the selected GPU adapter's name.
    pub fn device_name(&self) -> &str {
        &self.gpu.device_name
    }

    /// Borrows the device for custom GPU resources and commands.
    pub const fn device(&self) -> &wgpu::Device {
        &self.gpu.device
    }

    /// Borrows the queue for custom uploads and submissions.
    pub const fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }

    /// Returns the format shared by this renderer's presentation surfaces.
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.gpu.format
    }

    /// Adds a persistent render target that can also be sampled by later draws.
    /// Its contents are preserved between render batches. Release its surface
    /// handle with `remove_surface` when no further updates are needed.
    pub fn add_image(&mut self, size: [u32; 2]) -> (SurfaceHandle, crate::Image) {
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("isthmus render image"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.gpu.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let image = crate::Image::from_texture(&texture);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.gpu.format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size[0],
            height: size[1],
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        let target = SurfaceTarget::from_image(&self.gpu.device, config, image.texture_view().unwrap());
        (self.surfaces.insert(target), image)
    }

    /// Updates physical surface dimensions, ignoring zero dimensions and stale handles.
    pub fn resize(&mut self, surface: SurfaceHandle, [width, height]: [u32; 2]) {
        if let Some(slot) = self.surfaces.get_mut(surface) {
            slot.resize(&self.gpu, width, height);
        }
    }

    /// Adds an owned presentation target.
    pub fn add_surface(
        &mut self,
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: [u32; 2],
    ) -> Result<SurfaceHandle, SetupError> {
        let surface = self.gpu.instance.create_surface(target)?;
        let config = setup::configure_surface(&self.gpu.adapter, &surface, size, Some(self.gpu.format))?;
        Ok(self.surfaces.insert(SurfaceTarget::from_raw(&self.gpu.device, surface, config)))
    }

    /// Releases a surface and invalidates its handle; stale handles are ignored.
    pub fn remove_surface(&mut self, surface: SurfaceHandle) {
        self.surfaces.remove(surface);
    }
}
