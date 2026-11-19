use crate::ShaderData;

/// A nominal shader program whose interfaces are generated together.
pub trait Program: Copy + 'static {
    /// Application data shared by the program's shaders.
    type Globals: ShaderData;
    #[cfg(not(target_arch = "spirv"))]
    /// Generated shader entry points and pipeline state, sorted by name.
    const SHADERS: &'static [ShaderEntry];
    #[cfg(not(target_arch = "spirv"))]
    /// Most image bindings declared by any of the program's shaders.
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
    /// Vertex and fragment WGSL, each containing only its stage's dependencies.
    pub code: [&'static str; 2],
    /// The name given to `frame.shader(..)`, which keys the shader and labels its GPU objects in logs.
    pub name: &'static str,
    /// Color blending mode.
    pub blend: Blend,
    /// Number of image bindings required by this shader.
    pub images: usize,
    /// Optional offscreen attachment format; otherwise the presentation format.
    pub format: Option<wgpu::TextureFormat>,
}
