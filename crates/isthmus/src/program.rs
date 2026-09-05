use crate::ShaderData;
#[cfg(not(target_arch = "spirv"))]
use crate::geometry::Primitive;

/// A nominal shader program whose interfaces are generated together.
///
/// # Safety
/// Metadata and code must come from the same validated program and match its globals layout.
pub unsafe trait Program: Copy + 'static {
    type Globals: ShaderData + Default;
    #[cfg(not(target_arch = "spirv"))]
    const CODE: &'static [u8];
    #[cfg(not(target_arch = "spirv"))]
    const SHADERS: &'static [ShaderEntry];
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    Over,
    Add,
    Replace,
}

#[cfg(not(target_arch = "spirv"))]
pub struct ShaderEntry {
    pub name: &'static str,
    pub blend: Blend,
    pub primitive: Primitive,
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
    type Program: Program;
    type Geometry;
    const INDEX: usize;
}
