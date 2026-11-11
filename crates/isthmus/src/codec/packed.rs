//! Normalized and half-precision values packed into a single 32-bit word.
use crate::{
    MinMax,
    glam::{Vec2, Vec3, Vec4},
};

/// Two normalized channels stored in one 32-bit word.
#[derive(Clone, Copy, Default)]
pub struct Unorm16x2(pub(super) u32);

impl Unorm16x2 {
    /// Clamps two channels to 0..=1 and rounds each to 16 bits.
    pub fn from_vec2(value: Vec2) -> Self {
        let channel = |value: f32| (value.clamp_num(0.0, 1.0) * 65535.0 + 0.5) as u32;
        Self(channel(value.x) | (channel(value.y) << 16))
    }

    /// Decodes both channels to floats in 0..=1.
    pub fn to_vec2(self) -> Vec2 {
        Vec2::new((self.0 & 65535) as f32, (self.0 >> 16) as f32) / 65535.0
    }
}

/// Two half-precision floats stored in one 32-bit word.
#[derive(Clone, Copy, Default)]
pub struct F16x2(pub(super) u32);

impl F16x2 {
    #[cfg(not(target_arch = "spirv"))]
    /// Rounds two floats to half precision and packs them into one word.
    pub fn from_vec2(value: Vec2) -> Self {
        Self(
            u32::from(half::f16::from_f32(value.x).to_bits())
                | (u32::from(half::f16::from_f32(value.y).to_bits()) << 16),
        )
    }

    /// Decodes both half-precision channels to 32-bit floats.
    pub fn to_vec2(self) -> Vec2 {
        #[cfg(target_arch = "spirv")]
        {
            spirv_std::float::f16x2_to_vec2(self.0)
        }
        #[cfg(not(target_arch = "spirv"))]
        {
            Vec2::new(
                half::f16::from_bits(self.0 as u16).to_f32(),
                half::f16::from_bits((self.0 >> 16) as u16).to_f32(),
            )
        }
    }
}

/// Four normalized channels stored in one 32-bit word.
#[derive(Clone, Copy, Default)]
pub struct Unorm8x4(pub(super) u32);

impl Unorm8x4 {
    /// Clamps four channels to 0..=1 and rounds each to eight bits.
    pub fn from_vec4(value: Vec4) -> Self {
        let channel = |value: f32| (value.clamp_num(0.0, 1.0) * 255.0 + 0.5) as u32;
        Self(channel(value.x) | (channel(value.y) << 8) | (channel(value.z) << 16) | (channel(value.w) << 24))
    }

    /// Packs three normalized channels with an opaque fourth channel.
    pub fn from_vec3(value: Vec3) -> Self {
        Self::from_vec4(value.extend(1.0))
    }

    /// Decodes all four channels to floats in 0..=1.
    pub fn to_vec4(self) -> Vec4 {
        Vec4::new(
            (self.0 & 255) as f32,
            ((self.0 >> 8) & 255) as f32,
            ((self.0 >> 16) & 255) as f32,
            (self.0 >> 24) as f32,
        ) / 255.0
    }

    /// Decodes the first three channels, discarding the fourth.
    pub fn to_vec3(self) -> Vec3 {
        self.to_vec4().truncate()
    }
}
