//! The frame state shared by host recording and GPU invocations.
use crate::{Program, ShaderData, glam::Vec2};

/// The same typed frame state is used by host recording and GPU invocations.
#[derive(Clone, Copy, ShaderData)]
pub struct FrameState<G> {
    /// Surface dimensions in logical pixels.
    pub screen_size: Vec2,
    /// Frame time in seconds.
    pub time: f32,
    /// Logical size of each physical pixel, preserving nonuniform scaling.
    pub pixel_scale: Vec2,
    /// Application data shared by every draw on this surface.
    pub globals: G,
}

impl<G> FrameState<G> {
    /// Conservative logical footprint of one framebuffer pixel.
    pub fn pixel_size(&self) -> f32 {
        self.pixel_scale.max_element()
    }
}

/// Frame state specialized to a program's globals.
pub type ShaderFrame<P> = FrameState<<P as Program>::Globals>;
