use super::{
    canvas::Canvas,
    context::{Context, SetupError},
    surface::{Present, SurfaceTarget},
};
use crate::{
    Frame, SurfaceHandle,
    contract::{Program, PushBlock},
    glam::{Vec2, Vec3},
    text::Text,
};
use core::{error::Error, fmt};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{time::Instant, vec, vec::Vec};

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
#[derive(Debug)]
pub enum RenderError {
    SurfaceLost,
    Validation,
}
impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceLost => f.write_str("presentation surface was lost"),
            Self::Validation => f.write_str("frame failed GPU validation"),
        }
    }
}
impl Error for RenderError {}
impl Render<'_> {
    pub fn surface(&mut self, surface: SurfaceHandle, screen_size: Vec2, draw: impl FnOnce(Frame<'_>)) {
        let Some(slot) = surface_slot(self.surfaces, surface) else { return };
        slot.push.screen_size = screen_size;
        slot.push.time = self.time;
        draw(Frame::new(&slot.push, self.time, self.delta_time, self.text, self.canvas, surface));
        self.canvas.ensure_globals(surface);
    }
}
pub struct ShaderModule {
    bytes: &'static [u8],
    root: &'static str,
}
impl ShaderModule {
    pub const fn new(bytes: &'static [u8], root: &'static str) -> Self {
        Self { bytes, root }
    }
}

impl Renderer {
    /// Creates a renderer and its primary presentation surface.
    ///
    /// # Errors
    /// Returns an error if GPU or surface initialization fails, or if the shader is invalid.
    pub fn new(
        program: Program,
        surface: &(impl HasDisplayHandle + HasWindowHandle),
        [width, height]: [u32; 2],
        font: &[u8],
        text_color: Vec3,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let (context, raw_surface) = Context::new(surface)?;
        let target = SurfaceTarget::from_raw(&context, raw_surface, width, height)?;
        let module = program.shader;
        if !module.bytes.len().is_multiple_of(4) {
            return Err(SetupError::InvalidShader);
        }
        let (words, _) = module.bytes.as_chunks::<4>();
        let shader = words.iter().map(|b| u32::from_le_bytes(*b)).collect();
        let mut canvas = Canvas::new(&context, shader, target.format, module.root);
        let text = Text::new(&mut canvas, font, text_color);
        Ok((
            Self {
                context,
                surfaces: vec![SurfaceEntry {
                    generation: 0,
                    slot: Some(SurfaceSlot {
                        target,
                        push: PushBlock::default(),
                    }),
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
    /// Records and presents one frame.
    ///
    /// # Errors
    /// Returns an error if a presentation surface is lost or GPU validation fails.
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
        let shared = bytemuck::bytes_of(&slot.push);
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
            self.canvas.draw_surface(&mut pass, surface, shared);
        }
        self.context.0.queue.submit([encoder.finish()]);
        slot.target.present(frame)
    }
    pub fn device_name(&self) -> &str {
        self.context.device_name()
    }
    pub fn resize(&mut self, surface: SurfaceHandle, [width, height]: [u32; 2]) {
        if let Some(slot) = surface_slot(&mut self.surfaces, surface) {
            slot.target.resize(width, height);
        }
    }
    /// Adds another presentation surface.
    ///
    /// # Errors
    /// Returns an error if the surface cannot be initialized or uses a different format.
    pub fn add_surface(&mut self, target: &(impl HasDisplayHandle + HasWindowHandle), [width, height]: [u32; 2]) -> Result<SurfaceHandle, SetupError> {
        let target = SurfaceTarget::new(&self.context, target, width, height)?;
        if target.format != self.canvas.format() {
            return Err(SetupError::IncompatibleSurface);
        }
        let slot = SurfaceSlot {
            target,
            push: PushBlock::default(),
        };
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
