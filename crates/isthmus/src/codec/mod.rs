//! `ShaderData`: fixed-size word records shared by the host and shader code.
use crate::glam::{Mat4, UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};
pub use inline::InlineVec;
pub use packed::{F16x2, Unorm8x4, Unorm16x2};
use spirv_std::arch::IndexUnchecked;

mod inline;
mod packed;

/// Encoded records addressed by captures in the current frame.
pub type ResourceData<'frame> = &'frame [u32];

/// A fixed-size word codec shared by CPU and GPU, independent of Rust's memory layout.
pub trait ShaderData: Copy {
    /// Value exposed inside a shader after resolving shared resource handles.
    type View<'view>: Copy;
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
            type View<'view> = Self;

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
// Fixed components avoid temporary arrays and dynamic indexing in shader decoders.
macro_rules! vector_codec {
    ($ty:ty, $scalar:ty, $count:expr, $($field:ident: $index:expr),+) => {
        impl ShaderData for $ty {
            type View<'view> = Self;
            const WORDS: usize = $count;
            const ZERO: Self = Self::ZERO;
            fn resolve(self, _: ResourceData<'_>) -> Self { self }
            unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
                // SAFETY: Every component lies within the complete vector record.
                Self::new($(unsafe { <$scalar>::read_unchecked(words, offset + $index) }),+)
            }
            fn write(self, words: &mut [u32], offset: usize) {
                $(self.$field.write(words, offset + $index);)+
            }
        }
    };
}
vector_codec!(Vec2, f32, 2, x: 0, y: 1);
vector_codec!(Vec3, f32, 3, x: 0, y: 1, z: 2);
vector_codec!(Vec4, f32, 4, x: 0, y: 1, z: 2, w: 3);
vector_codec!(UVec2, u32, 2, x: 0, y: 1);
vector_codec!(UVec3, u32, 3, x: 0, y: 1, z: 2);
vector_codec!(UVec4, u32, 4, x: 0, y: 1, z: 2, w: 3);
codec!(Mat4, [f32; 16], Self::ZERO, |v: [f32; 16]| Self::from_cols_array(&v), |v: Self| v.to_cols_array());

impl ShaderData for u32 {
    type View<'view> = Self;

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
    type View<'view> = Self;

    const WORDS: usize = 0;
    const ZERO: Self = ();

    fn resolve(self, _: ResourceData<'_>) -> Self {}

    unsafe fn read_unchecked(_: &[u32], _: usize) -> Self {}

    fn write(self, _: &mut [u32], _: usize) {}
}

impl<T: ShaderData, const N: usize> ShaderData for [T; N] {
    type View<'view> = Self;

    const WORDS: usize = T::WORDS * N;
    const ZERO: Self = [T::ZERO; N];

    fn resolve(self, _: ResourceData<'_>) -> Self {
        self
    }

    unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
        let mut values = Self::ZERO;
        let mut index = 0;
        while index < N {
            // SAFETY: The caller guarantees the complete encoded source.
            let value = unsafe { T::read_unchecked(words, offset + index * T::WORDS) };
            // SAFETY: The loop bounds the destination, without a shader panic path.
            unsafe {
                *values.index_unchecked_mut(index) = value;
            }
            index += 1;
        }
        values
    }

    fn write(self, words: &mut [u32], offset: usize) {
        for (index, value) in self.into_iter().enumerate() {
            value.write(words, offset + index * T::WORDS);
        }
    }
}
