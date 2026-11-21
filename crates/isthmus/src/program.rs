use crate::ShaderData;

/// A nominal shader program whose interfaces are generated together.
pub trait Program: Copy + 'static {
    /// Application data shared by the program's shaders.
    type Globals: ShaderData;
    #[cfg(not(target_arch = "spirv"))]
    /// Generated shader entry points and pipeline state, sorted by name.
    const SHADERS: &'static [ShaderEntry];
    #[cfg(not(target_arch = "spirv"))]
    /// Largest number of images captured by one draw.
    fn max_images() -> usize {
        Self::SHADERS.iter().map(|entry| entry.images).max().unwrap_or(0)
    }
}

/// How a shader's colour combines with the render target.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    /// Composites the source over the destination using source alpha.
    Over,
    /// Adds alpha-weighted source RGB to the destination.
    Add,
    /// Replaces the destination with the source output.
    Replace,
    /// Stores raw fragment channels without blending, alpha premultiplication,
    /// or coverage multiplication. Intended for offscreen data textures.
    Raw,
}

#[cfg(not(target_arch = "spirv"))]
/// Generated entry points and resource requirements for one shader pipeline.
pub struct ShaderEntry {
    /// Vertex and fragment SPIR-V, each containing only its stage's dependencies.
    pub(crate) code: [&'static [u8]; 2],
    /// NUL-terminated SPIR-V entry point names in stage order.
    pub(crate) entries: [&'static str; 2],
    /// The name given to `frame.shader(..)`, which keys the shader and labels its GPU objects in logs.
    pub(crate) name: &'static str,
    /// Color blending mode.
    pub(crate) blend: Blend,
    /// Number of image indices in the draw record.
    pub(crate) images: usize,
    /// Optional offscreen attachment format; otherwise the presentation format.
    pub(crate) format: Option<crate::TextureFormat>,
}

#[cfg(not(target_arch = "spirv"))]
impl ShaderEntry {
    /// Describes a compiled shader in a generated program manifest.
    ///
    /// # Safety
    /// Both binaries must be valid Vulkan 1.4 SPIR-V with the specified entry points,
    /// generated for Isthmus's buffer, texture table, and draw record layout.
    pub const unsafe fn new(
        name: &'static str,
        blend: Blend,
        images: usize,
        format: Option<crate::TextureFormat>,
        entries: [&'static str; 2],
        code: [&'static [u8]; 2],
    ) -> Self {
        Self { code, entries, name, blend, images, format }
    }
}
