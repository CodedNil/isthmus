//! Failures while setting up or driving the renderer.
use ash::vk;

#[derive(Clone, Debug, thiserror::Error)]
pub enum RenderError {
    #[error("presentation surface was lost")]
    SurfaceLost,
    #[error("GPU image is unavailable on this renderer")]
    MissingImage,
    #[error("GPU device was lost: {0}")]
    DeviceLost(String),
    #[error("GPU operation failed: {0}")]
    Gpu(String),
    #[error("shader {0:?} was not built into this program")]
    UnknownShader(&'static str),
    #[error("{label} needs {required} bytes, exceeding the device limit of {limit}")]
    ResourceLimit { label: &'static str, required: u64, limit: u64 },
    #[error("the frame exceeds the device's table of {0} sampled images")]
    ImageLimit(u32),
}

impl From<vk::Result> for RenderError {
    fn from(result: vk::Result) -> Self {
        match result {
            vk::Result::ERROR_DEVICE_LOST => Self::DeviceLost(result.to_string()),
            vk::Result::ERROR_SURFACE_LOST_KHR => Self::SurfaceLost,
            _ => Self::Gpu(result.to_string()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("Vulkan setup failed: {0}")]
    Vulkan(String),
    #[error(transparent)]
    Gpu(#[from] RenderError),
    #[error("surface is unsupported by the selected device")]
    UnsupportedSurface,
}

impl From<vk::Result> for SetupError {
    fn from(result: vk::Result) -> Self {
        Self::Gpu(result.into())
    }
}
