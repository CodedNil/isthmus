/// Encoded records addressed by captures in the current frame.
pub type ResourceData<'a> = &'a [u32];

#[cfg(not(target_arch = "spirv"))]
/// Application-owned resource preparation shared across a renderer's surfaces.
pub trait Resources {
    /// Starts a frame before any surface is recorded.
    fn begin_frame(&mut self) {}
    /// Returns encoded storage; every handle recorded this frame must address this slice.
    fn data(&self) -> ResourceData<'_>;
}

#[cfg(not(target_arch = "spirv"))]
impl Resources for () {
    fn data(&self) -> ResourceData<'_> {
        &[]
    }
}
