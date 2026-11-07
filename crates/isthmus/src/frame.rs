use crate::{
    Fragment, Image, Primitive, Program, ShaderData, ShaderFrame,
    backend::{gpu::Gpu, surface::SurfaceTarget},
    glam,
};
use core::ops::Deref;

/// Immediate-mode drawing context for one surface in the current frame.
pub struct Frame<'a, P: Program> {
    /// Immutable host/GPU snapshot, exposed through dereferencing.
    pub(crate) state: ShaderFrame<P>,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
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
    pub fn capture_slice<T: ShaderData>(&mut self, slice: &[T]) -> [u32; 2] {
        let words = &mut self.gpu.words;
        let range = [
            u32::try_from(words.len()).expect("payload exceeds u32"),
            u32::try_from(slice.len()).expect("slice exceeds u32"),
        ];
        for &value in slice {
            value.append(words);
        }
        range
    }

    #[doc(hidden)]
    pub fn prepare<S: Primitive<P>>(
        &self,
        stage: impl FnOnce(ShaderFrame<P>) -> S,
        _: impl FnOnce(ShaderFrame<P>, Fragment<S::Sample>) -> glam::Vec4,
    ) -> u32 {
        stage(self.state).vertex_count()
    }

    #[doc(hidden)]
    pub fn record(&mut self, shader: usize, vertices: u32, value: impl ShaderData, images: &[&Image]) {
        self.gpu.emit(self.surface, shader, vertices, value, images);
    }
}

impl<P: Program> Deref for Frame<'_, P> {
    type Target = ShaderFrame<P>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
