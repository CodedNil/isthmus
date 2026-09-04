use crate::{
    Image,
    backend::Canvas,
    contract::{PushBlock, Quad, ShaderSpec, SurfaceHandle},
    data::ImageHandle,
    text::{Line, Text, TextScope},
};

pub struct Frame<'a> {
    pub time: f32,
    pub screen_size: glam::Vec2,
    pub delta_time: f32,
    text: &'a mut Text,
    canvas: &'a mut Canvas,
    surface: SurfaceHandle,
}

impl<'a> Frame<'a> {
    pub(crate) const fn new(
        push: &'a PushBlock,
        time: f32,
        delta_time: f32,
        text: &'a mut Text,
        canvas: &'a mut Canvas,
        surface: SurfaceHandle,
    ) -> Self {
        Self { time, screen_size: push.screen_size, delta_time, text, canvas, surface }
    }

    /// Sets app-defined shader data shared by every paint on this surface frame.
    pub fn set_globals<T: crate::ShaderData>(&mut self, globals: T) {
        self.canvas.set_globals(self.surface, globals);
    }

    #[doc(hidden)]
    pub fn __image(&mut self, image: &Image) -> ImageHandle {
        self.canvas.image(image.size, &image.pixels)
    }

    pub const fn text(&mut self) -> TextScope<'_> {
        TextScope::new(self.text)
    }

    /// Paints geometry with an inline shader receiving a fragment and typed captures.
    pub fn paint<S, Geometry, Payload>(&mut self, geometry: Geometry, payload: Payload)
    where
        S: ShaderSpec,
        Geometry: Copy + Into<Quad>,
        Payload: FnOnce(&mut Self, Geometry) -> S::Instance,
    {
        self.canvas.begin_payload(S::PIPELINE);
        let value = payload(self, geometry);
        self.canvas.emit::<S>(self.surface, geometry.into(), value);
    }

    /// Paints text with a shader receiving [`crate::TextFragment`] and typed captures.
    pub fn paint_text<S, Payload>(&mut self, line: Line, payload: Payload)
    where
        S: ShaderSpec,
        Payload: FnOnce(&mut Self, Line) -> S::Instance,
    {
        self.canvas.begin_payload(S::PIPELINE);
        let value = payload(self, line);
        let quads = self.text.quads(line);
        self.canvas.emit_text::<S>(self.surface, quads, value);
    }
}
