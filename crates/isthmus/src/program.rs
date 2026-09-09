use crate::ShaderData;

/// A nominal shader program whose interfaces are generated together.
pub trait Program: Copy + 'static {
    /// Application data shared by the program's shaders.
    type Globals: ShaderData;
    #[cfg(not(target_arch = "spirv"))]
    /// Shared host resources used by this program's captured handles.
    type Resources: crate::Resources;
    #[cfg(not(target_arch = "spirv"))]
    /// Compiled shader module embedded by the program macro.
    const CODE: &'static str;
    #[cfg(not(target_arch = "spirv"))]
    /// Generated shader entry points and pipeline state.
    const SHADERS: &'static [ShaderEntry];
}

/// How a shader's straight-alpha output combines with the render target.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    /// Composites the source over the destination using source alpha.
    Over,
    /// Adds alpha-weighted source RGB to the destination.
    Add,
    /// Replaces the destination with the source output.
    Replace,
}

#[cfg(not(target_arch = "spirv"))]
/// Generated entry points and resource requirements for one shader pipeline.
pub struct ShaderEntry {
    /// Vertex entry point name.
    pub vertex: &'static str,
    /// Fragment entry point name.
    pub name: &'static str,
    /// Color blending mode.
    pub blend: Blend,
    /// Number of image bindings required by this shader.
    pub images: usize,
}

#[cfg(not(target_arch = "spirv"))]
/// Fails constant evaluation when a shader is absent from its generated program.
pub const fn shader_index(entries: &[ShaderEntry], name: &str) -> usize {
    let mut index = 0;
    while index < entries.len() {
        if entries[index].name.eq(name) {
            return index;
        }
        index += 1;
    }
    panic!("shader was not extracted into this program");
}
