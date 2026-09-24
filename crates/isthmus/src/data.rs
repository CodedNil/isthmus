use crate::ResourceData;
use core::ops::Index;
use glam::{Mat4, UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};
use isthmus_macros::ShaderData;
use spirv_std::arch::IndexUnchecked;

/// A fixed-capacity vector that can be shared with shaders.
#[derive(Clone, Copy)]
// Nested shader arrays require 16-byte alignment.
#[repr(C, align(16))]
pub struct InlineVec<T, const N: usize> {
    len: u32,
    items: [T; N],
}

impl<T: ShaderData, const N: usize> InlineVec<T, N> {
    pub const fn new() -> Self {
        Self { len: 0, items: [T::ZERO; N] }
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn push(&mut self, item: T) {
        assert!((self.len as usize) < N, "inline vector is full");
        self.items[self.len as usize] = item;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len()]
    }
}

impl<T: ShaderData, const N: usize> Index<usize> for InlineVec<T, N> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        assert!(index < self.len(), "inline vector index is out of bounds");
        &self.items[index]
    }
}

impl<T: ShaderData, const N: usize> Default for InlineVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ShaderData, const N: usize> ShaderData for InlineVec<T, N> {
    type View<'a> = Self;

    const WORDS: usize = 1 + N * T::WORDS;
    const ZERO: Self = Self::new();

    fn resolve(self, _: ResourceData<'_>) -> Self {
        self
    }

    unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
        // SAFETY: The caller guarantees the complete record.
        let len = unsafe { u32::read_unchecked(words, offset) }.min(N as u32);
        let mut items = [T::ZERO; N];
        let mut index = 0;
        while index < len as usize {
            // SAFETY: Active items lie within the complete record.
            items[index] = unsafe { T::read_unchecked(words, offset + 1 + index * T::WORDS) };
            index += 1;
        }
        Self { len, items }
    }

    fn write(self, words: &mut [u32], offset: usize) {
        self.len.write(words, offset);
        for index in 0..self.len() {
            self.items[index].write(words, offset + 1 + index * T::WORDS);
        }
    }
}

/// Two normalized channels stored in one 32-bit word.
#[derive(Clone, Copy, Default)]
pub struct Unorm16x2(u32);

impl Unorm16x2 {
    /// Clamps two channels to 0..=1 and rounds each to 16 bits.
    pub fn from_vec2(value: Vec2) -> Self {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
        Self(channel(value.x) | (channel(value.y) << 16))
    }

    /// Decodes both channels to floats in 0..=1.
    pub fn to_vec2(self) -> Vec2 {
        Vec2::new((self.0 & 65535) as f32, (self.0 >> 16) as f32) / 65535.0
    }
}

/// Two half-precision floats stored in one 32-bit word.
#[derive(Clone, Copy, Default)]
pub struct F16x2(u32);

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
pub struct Unorm8x4(u32);

impl Unorm8x4 {
    /// Clamps four channels to 0..=1 and rounds each to eight bits.
    pub fn from_vec4(value: Vec4) -> Self {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
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

/// A fixed-size word codec shared by CPU and GPU, independent of Rust's memory layout.
pub trait ShaderData: Copy {
    /// Value exposed inside a shader after resolving shared resource handles.
    type View<'a>: Copy;
    /// Resolves this capture against the current frame's resource storage.
    fn resolve(self, resources: ResourceData<'_>) -> Self::View<'_>;
    #[cfg(not(target_arch = "spirv"))]
    /// Appends a value and returns its word offset, rejecting arenas larger than u32 can address.
    fn append(self, words: &mut Vec<u32>) -> u32 {
        let offset = u32::try_from(words.len()).expect("shader arena exceeds u32");
        let end = words.len().checked_add(Self::WORDS).expect("shader arena size overflow");
        u32::try_from(end).expect("shader arena exceeds u32");
        words.resize(end, 0);
        self.write(words, offset as usize);
        offset
    }
    /// Number of 32-bit words occupied by one encoded value.
    const WORDS: usize;
    /// Fallback value returned when a read is out of bounds.
    const ZERO: Self;
    /// Reads a complete record, returning its zero value for an out-of-bounds record.
    fn read(words: &[u32], offset: usize) -> Self {
        if offset > words.len() || Self::WORDS > words.len() - offset {
            return Self::ZERO;
        }
        // SAFETY: The complete encoded record was checked above.
        unsafe { Self::read_unchecked(words, offset) }
    }
    /// Decodes a complete record without checking its bounds.
    /// # Safety
    /// The buffer must contain `Self::WORDS` words starting at offset.
    unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self;
    /// Writes a complete record at a word offset; the destination must have room for `Self::WORDS`.
    fn write(self, words: &mut [u32], offset: usize);
}

macro_rules! codec {
    ($ty:ty, $storage:ty, $zero:expr, $decode:expr, $encode:expr) => {
        impl ShaderData for $ty {
            type View<'a> = Self;

            const WORDS: usize = <$storage>::WORDS;
            const ZERO: Self = $zero;

            fn resolve(self, _: ResourceData<'_>) -> Self {
                self
            }

            unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
                // SAFETY: The storage codec occupies the same complete record.
                ($decode)(unsafe { <$storage>::read_unchecked(words, offset) })
            }

            fn write(self, words: &mut [u32], offset: usize) {
                ($encode)(self).write(words, offset);
            }
        }
    };
}
codec!(i32, u32, 0, |word: u32| word as Self, |value: Self| value as u32);
codec!(f32, u32, 0.0, Self::from_bits, Self::to_bits);
codec!(bool, u32, false, |word| word != 0, u32::from);
codec!(Unorm8x4, u32, Self(0), Self, |value: Self| value.0);
codec!(F16x2, u32, Self(0), Self, |value: Self| value.0);
codec!(Unorm16x2, u32, Self(0), Self, |value: Self| value.0);
codec!(Vec2, [f32; 2], Self::ZERO, Self::from_array, |v: Self| v.to_array());
codec!(Vec3, [f32; 3], Self::ZERO, Self::from_array, |v: Self| v.to_array());
codec!(Vec4, [f32; 4], Self::ZERO, Self::from_array, |v: Self| v.to_array());
codec!(Mat4, [f32; 16], Self::ZERO, |v: [f32; 16]| Self::from_cols_array(&v), |v: Self| v.to_cols_array());
codec!(UVec2, [u32; 2], Self::ZERO, Self::from_array, |v: Self| v.to_array());
codec!(UVec3, [u32; 3], Self::ZERO, Self::from_array, |v: Self| v.to_array());
codec!(UVec4, [u32; 4], Self::ZERO, Self::from_array, |v: Self| v.to_array());

impl ShaderData for u32 {
    type View<'a> = Self;

    const WORDS: usize = 1;
    const ZERO: Self = 0;

    fn resolve(self, _: ResourceData<'_>) -> Self {
        self
    }

    unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
        // SAFETY: The caller guarantees this word belongs to a complete record.
        unsafe { *words.index_unchecked(offset) }
    }

    fn write(self, words: &mut [u32], offset: usize) {
        words[offset] = self;
    }
}

impl ShaderData for () {
    type View<'a> = Self;

    const WORDS: usize = 0;
    const ZERO: Self = ();

    fn resolve(self, _: ResourceData<'_>) -> Self {}

    unsafe fn read_unchecked(_: &[u32], _: usize) -> Self {}

    fn write(self, _: &mut [u32], _: usize) {}
}

impl<T: ShaderData, const N: usize> ShaderData for [T; N] {
    type View<'a> = Self;

    const WORDS: usize = T::WORDS * N;
    const ZERO: Self = [T::ZERO; N];

    fn resolve(self, _: ResourceData<'_>) -> Self {
        self
    }

    #[expect(clippy::needless_range_loop, reason = "Rust-GPU cannot lower the array slice iterators")]
    unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
        let mut values = Self::ZERO;
        for index in 0..N {
            // SAFETY: Each element lies within the complete array guaranteed by the caller.
            values[index] = unsafe { T::read_unchecked(words, offset + index * T::WORDS) };
        }
        values
    }

    fn write(self, words: &mut [u32], offset: usize) {
        for (index, value) in self.into_iter().enumerate() {
            value.write(words, offset + index * T::WORDS);
        }
    }
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct FrameData<G> {
    pub screen_size: Vec2,
    pub time: f32,
    pub pixel_scale: Vec2,
    pub globals: G,
}
