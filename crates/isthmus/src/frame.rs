use crate::{
    Buffer, Fragment, Image, Primitive, Program, ShaderData, ShaderFrame,
    backend::{gpu::Gpu, surface::SurfaceTarget},
    bindings,
};

/// Immediate-mode drawing context for one surface in the current frame.
pub struct Frame<'a, P: Program> {
    /// Seconds elapsed since the renderer was created.
    pub time: f32,
    /// Surface dimensions in logical pixels.
    pub screen_size: glam::Vec2,
    pub(crate) pixel_size: f32,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
    /// Immutable application snapshot shared by this surface's draws.
    pub globals: P::Globals,
    /// Shared application resources for this frame.
    pub resources: &'a mut P::Resources,
    pub(crate) gpu: &'a mut Gpu,
    pub(crate) surface: &'a mut SurfaceTarget,
}

impl<P: Program> Frame<'_, P> {
    #[doc(hidden)]
    pub const fn reborrow(&mut self) -> &mut Self {
        self
    }

    #[doc(hidden)]
    pub fn capture_buffer<T: ShaderData>(&mut self, buffer: Buffer<'_, T>) -> [u32; 2] {
        let payload = &mut self.gpu.buffers[bindings::PAYLOAD as usize];
        let range = [
            u32::try_from(payload.words.len()).expect("payload exceeds u32"),
            u32::try_from(buffer.values.len()).expect("buffer exceeds u32"),
        ];
        for &value in buffer.values {
            value.append(&mut payload.words);
        }
        range
    }

    #[doc(hidden)]
    pub fn prepare<S: Primitive<P>>(
        &self,
        stage: impl FnOnce(ShaderFrame<P>) -> S,
        _: impl FnOnce(ShaderFrame<P>, Fragment<S::Sample>) -> glam::Vec4,
    ) -> u32 {
        stage(ShaderFrame {
            time: self.time,
            screen_size: self.screen_size,
            pixel_size: self.pixel_size,
            globals: self.globals,
        })
        .vertex_count()
    }

    #[doc(hidden)]
    pub fn record(&mut self, shader: usize, vertices: u32, value: impl ShaderData, images: &[&Image]) {
        self.gpu.emit(self.surface, shader, vertices, value, images);
    }
}
