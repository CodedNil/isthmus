use crate::{
    Geometry, Program,
    backend::gpu::{Gpu, SurfacePaints},
    geometry::{GeometrySample, text::Text},
    program::ShaderSpec,
};
use core::borrow::Borrow;

/// Immediate-mode drawing context for one surface in the current frame.
pub struct Frame<'a, P: Program> {
    /// Seconds elapsed since the renderer was created.
    pub time: f32,
    /// Surface dimensions in logical pixels.
    pub screen_size: glam::Vec2,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
    /// Mutable application data shared by this surface's draws.
    pub globals: &'a mut P::Globals,
    /// Shared font and text layout resources.
    pub text: &'a mut Text,
    pub(crate) gpu: &'a mut Gpu,
    pub(crate) surface: &'a mut SurfacePaints,
}

impl<P: Program> Frame<'_, P> {
    /// Paints geometry with an inline shader receiving a fragment and typed captures.
    pub fn paint<S, G, Payload>(&mut self, geometry: G, payload: Payload)
    where
        S: ShaderSpec<Program = P, Sample = G::Sample>,
        G: Geometry,
        Text: Borrow<G::Context>,
        Payload: FnOnce(&mut Gpu, <G::Sample as GeometrySample<'static>>::Payload) -> (S, Option<wgpu::BindGroup>),
    {
        let mut primitives = geometry.primitives(Text::borrow(self.text)).peekable();
        if primitives.peek().is_none() {
            return;
        }
        let (value, image) = payload(self.gpu, geometry.payload());
        self.gpu.emit::<S>(self.surface, primitives, value, image);
    }
}
