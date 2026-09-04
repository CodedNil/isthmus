use glam::{UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};

/// Four normalized channels stored in one 32-bit word.
#[repr(transparent)]
#[derive(Clone, Copy, Default)]
#[cfg_attr(not(target_arch = "spirv"), derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Unorm8x4(u32);

/// Index into the shader-visible image heap.
#[repr(transparent)]
#[derive(Clone, Copy)]
#[cfg_attr(not(target_arch = "spirv"), derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct ImageHandle(u32);

impl ImageHandle {
    #[cfg(not(target_arch = "spirv"))]
    pub(crate) const fn new(image: u32) -> Self {
        Self(image)
    }

    pub(crate) const fn index(self) -> u32 {
        self.0
    }
}

impl Unorm8x4 {
    pub fn from_vec4(value: Vec4) -> Self {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        Self(channel(value.x) | (channel(value.y) << 8) | (channel(value.z) << 16) | (channel(value.w) << 24))
    }

    pub fn from_vec3(value: Vec3) -> Self {
        Self::from_vec4(value.extend(1.0))
    }

    pub fn to_vec4(self) -> Vec4 {
        #[cfg(target_arch = "spirv")]
        {
            spirv_std::float::u8x4_to_vec4_unorm(self.0)
        }
        #[cfg(not(target_arch = "spirv"))]
        {
            Vec4::new(
                (self.0 & 255) as f32,
                ((self.0 >> 8) & 255) as f32,
                ((self.0 >> 16) & 255) as f32,
                (self.0 >> 24) as f32,
            ) / 255.0
        }
    }

    pub fn to_vec3(self) -> Vec3 {
        self.to_vec4().truncate()
    }
}

/// Plain data whose Rust representation is also its scalar-layout SPIR-V ABI.
///
/// # Safety
/// Implementations must have identical Rust and scalar-layout SPIR-V representations,
/// four-byte alignment or less, and a size divisible by four.
#[cfg(target_arch = "spirv")]
pub unsafe trait ShaderData: Copy {}
/// Plain data whose Rust representation is also its scalar-layout SPIR-V ABI.
///
/// # Safety
/// Implementations must have identical Rust and scalar-layout SPIR-V representations,
/// four-byte alignment or less, and a size divisible by four.
#[cfg(not(target_arch = "spirv"))]
pub unsafe trait ShaderData: Copy + bytemuck::Pod {}

// SAFETY: u32 has identical Rust and SPIR-V scalar representations.
unsafe impl ShaderData for u32 {}
// SAFETY: i32 has identical Rust and SPIR-V scalar representations.
unsafe impl ShaderData for i32 {}
// SAFETY: f32 has identical Rust and SPIR-V scalar representations.
unsafe impl ShaderData for f32 {}
// SAFETY: The unit type is a zero-sized payload in both representations.
unsafe impl ShaderData for () {}
// SAFETY: repr(transparent) Unorm8x4 has the representation of its u32 field.
unsafe impl ShaderData for Unorm8x4 {}
// SAFETY: repr(transparent) ImageHandle has the representation of its u32 field.
unsafe impl ShaderData for ImageHandle {}
// SAFETY: Vec2 is a two-f32 shader-compatible vector with four-byte alignment.
unsafe impl ShaderData for Vec2 {}
// SAFETY: Vec3 is a three-f32 shader-compatible vector under scalar layout.
unsafe impl ShaderData for Vec3 {}
// SAFETY: Vec4 is a four-f32 shader-compatible vector under scalar layout.
unsafe impl ShaderData for Vec4 {}
// SAFETY: UVec2 is a two-u32 shader-compatible vector with four-byte alignment.
unsafe impl ShaderData for UVec2 {}
// SAFETY: UVec3 is a three-u32 shader-compatible vector under scalar layout.
unsafe impl ShaderData for UVec3 {}
// SAFETY: UVec4 is a four-u32 shader-compatible vector under scalar layout.
unsafe impl ShaderData for UVec4 {}
#[cfg(target_arch = "spirv")]
// SAFETY: An array preserves the representation of its ShaderData elements.
unsafe impl<T: ShaderData, const N: usize> ShaderData for [T; N] {}
#[cfg(not(target_arch = "spirv"))]
// SAFETY: The Pod bound verifies that the host array has no invalid padding or bit patterns.
unsafe impl<T: ShaderData, const N: usize> ShaderData for [T; N] where [T; N]: bytemuck::Pod {}
