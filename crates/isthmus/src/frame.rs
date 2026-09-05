use crate::{
    Image, Program, Triangle,
    backend::gpu::{Gpu, SurfacePaints},
    contract::{PushBlock, Quad, ShaderSpec},
    text::{Line, Text},
};
use core::iter::once;

pub struct Frame<'a, P: Program> {
    pub time: f32,
    pub screen_size: glam::Vec2,
    pub delta_time: f32,
    pub globals: &'a mut P::Globals,
    text: &'a mut Text,
    gpu: &'a mut Gpu,
    surface: &'a mut SurfacePaints,
}

impl<'a, P: Program> Frame<'a, P> {
    pub(crate) const fn new(
        push: &PushBlock,
        delta_time: f32,
        globals: &'a mut P::Globals,
        text: &'a mut Text,
        gpu: &'a mut Gpu,
        surface: &'a mut SurfacePaints,
    ) -> Self {
        Self { time: push.time, screen_size: push.screen_size, delta_time, globals, text, gpu, surface }
    }

    #[doc(hidden)]
    pub fn __image(&mut self, image: &Image) -> wgpu::BindGroup {
        self.gpu.image(image)
    }

    pub const fn text(&mut self) -> &mut Text {
        self.text
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

/// Geometry accepted by paint, with a statically matched shader input kind.
pub trait Geometry: Copy {
    type Kind;
    fn primitives(self, text: &Text) -> impl Iterator<Item = [glam::Vec2; 3]>;
}

impl<T: Copy + Into<Quad>> Geometry for T {
    type Kind = Quad;

    fn primitives(self, _: &Text) -> impl Iterator<Item = [glam::Vec2; 3]> {
        once(self.into().data())
    }
}

impl Geometry for Line {
    type Kind = Self;

    fn primitives(self, text: &Text) -> impl Iterator<Item = [glam::Vec2; 3]> {
        text.quads(self).map(Quad::data)
    }
}

impl Geometry for Triangle {
    type Kind = Self;

    fn primitives(self, _: &Text) -> impl Iterator<Item = [glam::Vec2; 3]> {
        once(self.data())
    }
}
