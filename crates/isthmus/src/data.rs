use glam::{UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};

/// Four normalized channels stored in one 32-bit word.
#[repr(transparent)]
#[derive(Clone, Copy, Default)]
#[cfg_attr(not(target_arch = "spirv"), derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Unorm8x4(u32);

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

/// Data with identical Rust and scalar-layout SPIR-V representations and four-byte granularity.
///
/// # Safety
/// Implementations must have no padding, alignment at most four, and identical host/shader layouts.
#[cfg(target_arch = "spirv")]
pub unsafe trait ShaderData: Copy {}
/// Data with identical Rust and scalar-layout SPIR-V representations and four-byte granularity.
///
/// # Safety
/// Implementations must have no padding, alignment at most four, and identical host/shader layouts.
#[cfg(not(target_arch = "spirv"))]
pub unsafe trait ShaderData: Copy + bytemuck::Pod {}

macro_rules! shader_data {
    ($($ty:ty),*) => { $(
        #[expect(clippy::undocumented_unsafe_blocks, reason = "listed types have identical host and scalar-layout SPIR-V representations")]
        unsafe impl ShaderData for $ty {}
    )* };
}
shader_data!(u32, i32, f32, (), Unorm8x4, Vec2, Vec3, Vec4, UVec2, UVec3, UVec4);
#[cfg(target_arch = "spirv")]
// SAFETY: An array preserves the representation of its ShaderData elements.
unsafe impl<T: ShaderData, const N: usize> ShaderData for [T; N] {}
#[cfg(not(target_arch = "spirv"))]
// SAFETY: The Pod bound verifies that the host array has no invalid padding or bit patterns.
unsafe impl<T: ShaderData, const N: usize> ShaderData for [T; N] where [T; N]: bytemuck::Pod {}

/// Loads a generated shader value from its recorded byte-addressed buffer offset.
#[doc(hidden)]
/// # Safety
/// The offset must address a complete, correctly aligned value of T in the buffer.
pub unsafe fn load<T: ShaderData>(buffer: &[u32], byte_index: u32) -> T {
    // SAFETY: Generated shaders only request recorded, correctly aligned values of T.
    unsafe { spirv_std::ByteAddressableBuffer::from_slice(buffer).load_unchecked(byte_index) }
}

#[doc(hidden)]
#[repr(C)]
#[derive(Clone, Copy, Default, crate::ShaderData)]
pub struct PushBlock {
    pub screen_size: Vec2,
    pub time: f32,
    pub(crate) _padding: f32,
}

/// Straight-alpha color operations; shader output conversion belongs to Isthmus.
pub trait ColorExt {
    #[must_use]
    fn opacity(self, opacity: f32) -> Self;
}

impl ColorExt for Vec4 {
    fn opacity(self, opacity: f32) -> Self {
        self.truncate().extend(self.w * opacity)
    }
}
