use super::{
    gpu::Gpu,
    setup::{self, SetupError},
    surface::SurfaceTarget,
};
use crate::{Frame, Program, SurfaceHandle, bindings, data::FrameData, geometry::text::TextCache, glam::Vec2};
use core::marker::PhantomData;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use slotmap::SlotMap;
use smallvec::SmallVec;
use web_time::Instant;

/// GPU resources, text caches, and presentation surfaces for one shader program.
pub struct Renderer<P: Program> {
    program: PhantomData<P>,
    surfaces: SlotMap<SurfaceHandle, SurfaceTarget>,
    gpu: Gpu,
    text: TextCache,
    started: Instant,
    last_frame: f32,
}
/// Records surfaces that will be submitted together in one frame.
pub struct Render<'a, P: Program> {
    renderer: &'a mut Renderer<P>,
    time: f32,
    delta_time: f32,
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
    /// Records one surface in logical pixels; stale handles are ignored.
    pub fn surface(&mut self, surface: SurfaceHandle, screen_size: Vec2, draw: impl FnOnce(Frame<'_, P>)) {
        let renderer = &mut *self.renderer;
        let Some(surface) = renderer.surfaces.get_mut(surface) else { return };
        surface.paints.recorded = true;
        let frame_data = FrameData { screen_size, time: self.time };
        let mut globals = P::Globals::default();
        draw(Frame {
            time: self.time,
            screen_size,
            delta_time: self.delta_time,
            globals: &mut globals,
            text: &mut renderer.text,
            gpu: &mut renderer.gpu,
            surface: &mut surface.paints,
        });
        surface.paints.globals.upload(&renderer.gpu.device, &renderer.gpu.queue, &[globals]);
        surface.paints.frame.upload(&renderer.gpu.device, &renderer.gpu.queue, &[frame_data]);
    }
}
impl<P: Program> Renderer<P> {
    /// Creates a renderer and primary surface, validating both GPU and shader support.
    ///
    /// # Errors
    /// Returns GPU, surface, and shader initialization failures.
    ///
    /// # Safety
    /// The native display and window must remain alive until the surface is removed or the renderer is dropped.
    #[cfg(not(target_arch = "wasm32"))]
    pub unsafe fn new(
        surface: &(impl HasDisplayHandle + HasWindowHandle),
        [width, height]: [u32; 2],
        text: TextCache,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        // SAFETY: The caller keeps both native handles alive until this surface is removed.
        let (gpu, target) = unsafe { setup::new::<P>(surface, [width, height]) }?;
        Ok(Self::from_surface(gpu, target, text))
    }

    #[cfg(target_arch = "wasm32")]
    /// Creates a WebGPU renderer for a canvas, returning initialization failures.
    pub async fn new(
        canvas: web_sys::HtmlCanvasElement,
        size: [u32; 2],
        text: TextCache,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let (gpu, target) = setup::new::<P>(canvas, size).await?;
        Ok(Self::from_surface(gpu, target, text))
    }

    fn from_surface(gpu: Gpu, target: SurfaceTarget, text: TextCache) -> (Self, SurfaceHandle) {
        let mut surfaces = SlotMap::with_key();
        let handle = surfaces.insert(target);
        (Self { surfaces, gpu, text, started: Instant::now(), last_frame: 0.0, program: PhantomData }, handle)
    }

    /// Records and presents one frame, returning surface loss or GPU validation errors.
    ///
    /// # Errors
    /// Returns presentation or GPU validation failures.
    pub fn render(&mut self, draw: impl FnOnce(&mut Render<'_, P>)) -> Result<(), RenderError> {
        self.gpu.begin_frame();
        for surface in self.surfaces.values_mut() {
            surface.paints.paints.clear();
            surface.paints.recorded = false;
        }
        self.text.begin_frame();
        let elapsed = self.started.elapsed().as_secs_f32();
        let delta = (elapsed - self.last_frame).min(0.1);
        self.last_frame = elapsed;
        draw(&mut Render { renderer: self, time: elapsed, delta_time: delta });
        if !self.surfaces.values().any(|surface| surface.paints.recorded) {
            return Ok(());
        }
        self.text.upload_outlines(
            &mut self.gpu.buffers[bindings::OUTLINES as usize],
            &self.gpu.device,
            &self.gpu.queue,
        );
        self.gpu.prepare(&self.text.placed);
        let mut outputs = SmallVec::<[wgpu::SurfaceTexture; 2]>::new();
        let mut encoder =
            self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("isthmus frame") });
        for surface in self.surfaces.values_mut() {
            if !surface.paints.recorded {
                continue;
            }
            let Some(output) = surface.acquire(&self.gpu)? else { continue };
            let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("isthmus render pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                self.gpu.draw_surface(&mut pass, &mut surface.paints);
            }
            outputs.push(output);
        }
        if !outputs.is_empty() {
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

    /// Updates physical surface dimensions, ignoring zero dimensions and stale handles.
    pub fn resize(&mut self, surface: SurfaceHandle, [width, height]: [u32; 2]) {
        if let Some(slot) = self.surfaces.get_mut(surface) {
            slot.resize(&self.gpu, width, height);
        }
    }

    /// Adds a presentation surface when it supports the renderer's format.
    ///
    /// # Errors
    /// Returns surface initialization and format compatibility failures.
    ///
    /// # Safety
    /// The native display and window must remain alive until the surface is removed or the renderer is dropped.
    pub unsafe fn add_surface(
        &mut self,
        target: &(impl HasDisplayHandle + HasWindowHandle),
        [width, height]: [u32; 2],
    ) -> Result<SurfaceHandle, SetupError> {
        // SAFETY: The caller keeps both native handles alive until this surface is removed.
        let surface = unsafe { setup::create_surface(&self.gpu.instance, target) }?;
        let config = setup::configure_surface(&self.gpu.adapter, &surface, width, height)?;
        if config.format != self.gpu.format {
            return Err(SetupError::IncompatibleSurface);
        }
        Ok(self.surfaces.insert(SurfaceTarget::from_raw(&self.gpu.device, surface, config)))
    }

    /// Releases a surface and invalidates its handle; stale handles are ignored.
    pub fn remove_surface(&mut self, surface: SurfaceHandle) {
        self.surfaces.remove(surface);
    }
}
