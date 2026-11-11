//! Failures while setting up or driving the renderer.

/// A failure while recording or presenting a frame.
#[derive(Clone, Debug, thiserror::Error)]
pub enum RenderError {
    /// A presentation surface was lost and must be recreated.
    #[error("presentation surface was lost")]
    SurfaceLost,
    /// A GPU image belongs to another renderer or needs reimporting after recovery.
    #[error("GPU image is unavailable on this renderer")]
    MissingImage,
    /// GPU validation rejected the frame.
    #[error("frame failed GPU validation")]
    Validation,
    /// The device was lost; recover GPU resources before reuse.
    #[error("GPU device was lost: {0}")]
    DeviceLost(String),
    /// A GPU operation failed; the renderer requires recovery before reuse.
    #[error("GPU operation failed: {0}")]
    Gpu(String),
    /// Refused an upload before allocating or binding an oversized resource.
    #[error("{label} needs {required} bytes, exceeding the device limit of {limit}")]
    ResourceLimit { label: &'static str, required: u64, limit: u64 },
}

/// A failure while selecting a GPU or creating a presentation surface.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// No suitable GPU adapter could be selected.
    #[error("GPU adapter error: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// GPU resource initialization failed.
    #[error(transparent)]
    Gpu(#[from] RenderError),
    /// The GPU device could not be created with the required capabilities.
    #[error("GPU device error: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    /// The presentation surface could not be created.
    #[error("surface error: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    /// The surface has no supported presentation configuration.
    #[error("surface is unsupported")]
    UnsupportedSurface,
    /// An additional surface requires a different render target format.
    #[error("replacement surface is incompatible")]
    IncompatibleSurface,
}
