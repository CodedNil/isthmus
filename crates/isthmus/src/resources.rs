/// Shared storage available to captured resource handles on both CPU and GPU.
#[derive(Clone, Copy, Default)]
pub struct ResourceData<'a> {
    /// Frame-local records, uploaded only when their encoded contents change.
    pub transient: &'a [u32],
    /// Immutable records appended over the lifetime of the renderer.
    pub persistent: &'a [u32],
}

#[cfg(not(target_arch = "spirv"))]
/// Application-owned resource preparation shared across a renderer's surfaces.
pub trait Resources {
    /// Starts a frame before any surface is recorded.
    fn begin_frame(&mut self) {}
    /// Returns encoded storage; persistent records must never change or be removed.
    fn data(&self) -> ResourceData<'_>;
}

#[cfg(not(target_arch = "spirv"))]
impl Resources for () {
    fn data(&self) -> ResourceData<'_> {
        ResourceData::default()
    }
}
