//! Application-owned data shared by every draw in a renderer.
use crate::ResourceData;

/// Application-owned resource preparation shared across a renderer's surfaces.
pub trait Resources {
    /// Starts a frame before any surface is recorded.
    fn begin_frame(&mut self) {}
    /// Returns encoded storage; every handle recorded this frame must address this slice.
    fn data(&self) -> ResourceData<'_>;
}

impl Resources for () {
    fn data(&self) -> ResourceData<'_> {
        &[]
    }
}
