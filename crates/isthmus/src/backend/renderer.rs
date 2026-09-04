use super::{
    Canvas, Context, SetupError,
    surface::{Present, SurfaceTarget},
};
use crate::{
    Frame, SurfaceHandle,
    contract::{Program, PushBlock},
    glam::{Vec2, Vec3},
    text::Text,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use web_time::Instant;

pub struct Renderer {
    context: Context,
    surfaces: Vec<SurfaceEntry>,
    canvas: Canvas,
    text: Text,
    started: Instant,
    last_frame: f32,
}
struct SurfaceEntry {
    generation: u32,
    slot: Option<SurfaceSlot>,
}
struct SurfaceSlot {
    target: SurfaceTarget,
    push: PushBlock,
}
fn surface_slot(entries: &mut [SurfaceEntry], handle: SurfaceHandle) -> Option<&mut SurfaceSlot> {
    entries.get_mut(handle.index()).filter(|e| e.generation == handle.generation())?.slot.as_mut()
}
pub struct Render<'a> {
    surfaces: &'a mut [SurfaceEntry],
    time: f32,
    delta_time: f32,
    text: &'a mut Text,
    canvas: &'a mut Canvas,
}
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("presentation surface was lost")]
    SurfaceLost,
    #[error("frame failed GPU validation")]
    Validation,
}
impl Render<'_> {
    pub fn surface(&mut self, surface: SurfaceHandle, screen_size: Vec2, draw: impl FnOnce(Frame<'_>)) {
        let Some(slot) = surface_slot(self.surfaces, surface) else {
            return;
        };
        slot.push.screen_size = screen_size;
        slot.push.time = self.time;
        draw(Frame::new(&slot.push, self.time, self.delta_time, self.text, self.canvas, surface));
        self.canvas.set_frame(surface, slot.push);
        self.canvas.ensure_globals(surface);
    }
}
impl Renderer {
    /// Creates a renderer and primary surface, validating both GPU and shader support.
    ///
    /// # Errors
    /// Returns GPU, surface, and shader initialization failures.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        program: Program,
        surface: &(impl HasDisplayHandle + HasWindowHandle),
        [width, height]: [u32; 2],
        font: &[u8],
        text_color: Vec3,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let (context, raw_surface) = Context::new(surface)?;
        Self::from_surface(program, context, raw_surface, [width, height], font, text_color)
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new(
        program: Program,
        canvas: web_sys::HtmlCanvasElement,
        size: [u32; 2],
        font: &[u8],
        text_color: Vec3,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let (context, raw_surface) = Context::new(canvas).await?;
        Self::from_surface(program, context, raw_surface, size, font, text_color)
    }

    fn from_surface(
        program: Program,
        context: Context,
        raw_surface: wgpu::Surface<'static>,
        [width, height]: [u32; 2],
        font: &[u8],
        text_color: Vec3,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let target = SurfaceTarget::from_raw(&context, raw_surface, width, height)?;
        #[cfg(not(target_arch = "wasm32"))]
        if !program.bytes.len().is_multiple_of(4) {
            return Err(SetupError::InvalidShader);
        }
        #[cfg(target_arch = "wasm32")]
        std::str::from_utf8(program.bytes).map_err(|_| SetupError::InvalidShader)?;
        let mut canvas = Canvas::new(&context, program.bytes, target.format);
        let text = Text::new(&context, &mut canvas, font, text_color);
        Ok((
            Self {
                context,
                surfaces: vec![SurfaceEntry {
                    generation: 0,
                    slot: Some(SurfaceSlot { target, push: PushBlock::default() }),
                }],
                canvas,
                text,
                started: Instant::now(),
                last_frame: 0.0,
            },
            SurfaceHandle::new(0, 0),
        ))
    }

    fn begin_frame(&mut self) -> (f32, f32) {
        self.canvas.begin_frame();
        self.text.begin_frame();
        let elapsed = self.started.elapsed().as_secs_f32();
        let delta = (elapsed - self.last_frame).min(0.1);
        self.last_frame = elapsed;
        (elapsed, delta)
    }

    /// Records and presents one frame, returning surface loss or GPU validation errors.
    ///
    /// # Errors
    /// Returns presentation or GPU validation failures.
    pub fn render(&mut self, draw: impl FnOnce(&mut Render<'_>)) -> Result<(), RenderError> {
        let (elapsed, delta) = self.begin_frame();
        {
            let mut render = Render {
                surfaces: &mut self.surfaces,
                time: elapsed,
                delta_time: delta,
                text: &mut self.text,
                canvas: &mut self.canvas,
            };
            draw(&mut render);
        }
        let placed = self.text.finish_frame();
        self.canvas.prepare(placed);
        for index in 0..self.surfaces.len() {
            let Some(generation) = self.surfaces[index].slot.as_ref().map(|_| self.surfaces[index].generation) else {
                continue;
            };
            let handle = SurfaceHandle::new(index, generation);
            if !self.canvas.has_draws(handle) {
                continue;
            }
            match self.present(handle) {
                Present::Lost => return Err(RenderError::SurfaceLost),
                Present::Validation => return Err(RenderError::Validation),
                Present::Rendered | Present::Unavailable => {}
            }
        }
        Ok(())
    }

    fn present(&mut self, surface: SurfaceHandle) -> Present {
        let Some(slot) = surface_slot(&mut self.surfaces, surface) else {
            return Present::Unavailable;
        };
        let frame = match slot.target.acquire() {
            Ok(f) => f,
            Err(e) => return e,
        };
        let mut encoder = self
            .context
            .0
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("isthmus frame") });
        let extent = slot.target.extent;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("isthmus render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
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
            pass.set_viewport(0.0, 0.0, extent[0] as f32, extent[1] as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, extent[0], extent[1]);
            self.canvas.draw_surface(&mut pass, surface);
        }
        self.context.0.queue.submit([encoder.finish()]);
        slot.target.present(frame)
    }

    pub fn device_name(&self) -> &str {
        &self.context.0.device_name
    }

    pub fn resize(&mut self, surface: SurfaceHandle, [width, height]: [u32; 2]) {
        if let Some(slot) = surface_slot(&mut self.surfaces, surface) {
            slot.target.resize(width, height);
        }
    }

    /// Adds a presentation surface when it supports the renderer's format.
    ///
    /// # Errors
    /// Returns surface initialization and format compatibility failures.
    pub fn add_surface(
        &mut self,
        target: &(impl HasDisplayHandle + HasWindowHandle),
        [width, height]: [u32; 2],
    ) -> Result<SurfaceHandle, SetupError> {
        let target = SurfaceTarget::new(&self.context, target, width, height)?;
        if target.format != self.canvas.format() {
            return Err(SetupError::IncompatibleSurface);
        }
        let slot = SurfaceSlot { target, push: PushBlock::default() };
        if let Some((index, entry)) = self.surfaces.iter_mut().enumerate().find(|(_, e)| e.slot.is_none()) {
            entry.slot = Some(slot);
            Ok(SurfaceHandle::new(index, entry.generation))
        } else {
            let index = self.surfaces.len();
            self.surfaces.push(SurfaceEntry { generation: 0, slot: Some(slot) });
            Ok(SurfaceHandle::new(index, 0))
        }
    }

    pub fn remove_surface(&mut self, surface: SurfaceHandle) {
        if let Some(entry) = self.surfaces.get_mut(surface.index())
            && entry.generation == surface.generation()
        {
            entry.slot = None;
            entry.generation = entry.generation.wrapping_add(1);
        }
    }
}
