use crate::{
    Fragment, Primitive, Program, ShaderFrame,
    backend::{gpu::Gpu, surface::SurfaceTarget},
    program::ShaderSpec,
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
    #[doc(hidden)]
    pub gpu: &'a mut Gpu,
    pub(crate) surface: &'a mut SurfaceTarget,
}

impl<P: Program> Frame<'_, P> {
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
        .vertex_count(self.pixel_size)
    }

    #[doc(hidden)]
    /// # Safety
    /// The generated shader must reconstruct this geometry and consume this payload layout.
    pub unsafe fn record<S: ShaderSpec<Program = P>>(
        &mut self,
        vertices: u32,
        value: S,
        images: Option<wgpu::BindGroup>,
    ) {
        self.gpu.emit(self.surface, vertices, value, images);
    }
}
