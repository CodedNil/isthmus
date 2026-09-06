use crate::ShaderData;
#[cfg(not(target_arch = "spirv"))]
use crate::geometry::GeometrySample;

/// A nominal shader program whose interfaces are generated together.
///
/// # Safety
/// Metadata and code must come from the same validated program and match its globals layout.
pub unsafe trait Program: Copy + 'static {
    /// Application data shared by the program's shaders.
    type Globals: ShaderData + Default;
    #[cfg(not(target_arch = "spirv"))]
    /// Compiled shader module embedded by the program macro.
    const CODE: &'static [u8];
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
    /// Fragment entry point name.
    pub name: &'static str,
    /// Vertex entry point name.
    pub vertex: &'static str,
    /// Color blending mode.
    pub blend: Blend,
    /// Number of image bindings required by this shader.
    pub images: usize,
}

#[cfg(not(target_arch = "spirv"))]
/// # Panics
/// Fails constant evaluation when a shader is absent from its generated program.
pub const fn shader_index(entries: &[ShaderEntry], name: &str) -> usize {
    let mut index = 0;
    while index < entries.len() {
        let a = entries[index].name.as_bytes();
        let b = name.as_bytes();
        let mut byte = 0;
        if a.len() == b.len() {
            while byte < a.len() && a[byte] == b[byte] {
                byte += 1;
            }
            if byte == a.len() {
                return index;
            }
        }
        index += 1;
    }
    panic!("shader was not extracted into this program");
}

/// Describes the host-visible interface and fixed state of one shader.
///
/// # Safety
/// The entry must use this payload layout, globals type and geometry in the generated program.
#[cfg(not(target_arch = "spirv"))]
pub unsafe trait ShaderSpec: ShaderData {
    /// Program containing this shader's generated entry points.
    type Program: Program;
    /// Geometry queries available to the fragment shader.
    type Sample: GeometrySample<'static>;
    /// Index into the program's shader metadata.
    const INDEX: usize;
}
