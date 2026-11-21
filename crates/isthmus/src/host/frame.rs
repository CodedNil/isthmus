use super::{gpu::Gpu, surface::SurfaceTarget};
use crate::{Program, ShaderFrame};
use core::ops::Deref;

/// Immediate-mode drawing context for one surface in the current frame.
pub struct Frame<'draw, P: Program> {
    /// Immutable host/GPU snapshot, exposed through dereferencing.
    pub(crate) state: ShaderFrame<P>,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
    pub(crate) gpu: &'draw mut Gpu,
    pub(crate) surface: &'draw mut SurfaceTarget,
}

impl<P: Program> Deref for Frame<'_, P> {
    type Target = ShaderFrame<P>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
