use crate::{
    Geometry, Image, Program,
    backend::gpu::{Gpu, SurfacePaints},
    geometry::text::Text,
    program::ShaderSpec,
};

pub struct Frame<'a, P: Program> {
    pub time: f32,
    pub screen_size: glam::Vec2,
    pub delta_time: f32,
    pub globals: &'a mut P::Globals,
    pub text: &'a mut Text,
    pub(crate) gpu: &'a mut Gpu,
    pub(crate) surface: &'a mut SurfacePaints,
}

impl<P: Program> Frame<'_, P> {
    #[doc(hidden)]
    pub fn __image(&mut self, image: &Image) -> wgpu::BindGroup {
        self.gpu.image(image)
    }

    /// Paints geometry with an inline shader receiving a fragment and typed captures.
    pub fn paint<S, G, Payload>(&mut self, geometry: G, payload: Payload)
    where
        S: ShaderSpec<Program = P, Geometry = G::Kind>,
        G: Geometry,
        Payload: FnOnce(&mut Self, G) -> (S, Option<wgpu::BindGroup>),
    {
        let (value, image) = payload(self, geometry);
        self.gpu.emit::<S>(self.surface, geometry.primitives(self.text), value, image);
    }
}
