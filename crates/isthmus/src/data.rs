use glam::{UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};
use spirv_std::arch::IndexUnchecked;

/// A borrowed array capture; shaders decode only the elements they read.
#[derive(Clone, Copy)]
pub struct Buffer<'a, T: ShaderData> {
    #[cfg(not(target_arch = "spirv"))]
    pub(crate) values: &'a [T],
    #[cfg(target_arch = "spirv")]
    words: &'a [u32],
    #[cfg(target_arch = "spirv")]
    range: [u32; 2],
    #[cfg(target_arch = "spirv")]
    marker: core::marker::PhantomData<T>,
}

impl<'a, T: ShaderData> Buffer<'a, T> {
    /// Returns the number of captured elements.
    pub const fn len(self) -> usize {
        #[cfg(not(target_arch = "spirv"))]
        {
            self.values.len()
        }
        #[cfg(target_arch = "spirv")]
        {
            self.range[1] as usize
        }
    }

    /// Reports whether the captured array has no elements.
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[cfg(not(target_arch = "spirv"))]
    /// Borrows a host slice for capture by a shader.
    pub const fn new(values: &'a [T]) -> Self {
        Self { values }
    }

    #[cfg(target_arch = "spirv")]
    #[doc(hidden)]
    pub fn from_words(words: &'a [u32], range: [u32; 2]) -> Self {
        Self { words, range, marker: core::marker::PhantomData }
    }

    /// Returns zero when the index is outside the captured array.
    pub fn load(self, index: usize) -> T {
        #[cfg(not(target_arch = "spirv"))]
        {
            self.values.get(index).copied().unwrap_or(T::ZERO)
        }
        #[cfg(target_arch = "spirv")]
        {
            if index >= self.range[1] as usize || T::WORDS == 0 {
                return T::ZERO;
            }
            let offset = self.range[0] as usize;
            if offset > self.words.len() || index >= (self.words.len() - offset) / T::WORDS {
                return T::ZERO;
            }
            // SAFETY: The offset and complete record were checked above.
            unsafe { T::read_unchecked(self.words, offset + index * T::WORDS) }
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
        #[cfg(target_arch = "spirv")]
        {
            spirv_std::float::u16x2_to_vec2_unorm(self.0)
        }
        #[cfg(not(target_arch = "spirv"))]
        {
            Vec2::new((self.0 & 65535) as f32, (self.0 >> 16) as f32) / 65535.0
        }
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

    /// Decodes the first three channels, discarding the fourth.
    pub fn to_vec3(self) -> Vec3 {
        self.to_vec4().truncate()
    }
}

/// A fixed-size word codec shared by CPU and GPU, independent of Rust's memory layout.
pub trait ShaderData: Copy {
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

macro_rules! scalar {
    ($ty:ty, $zero:expr, $decode:expr, $encode:expr) => {
        impl ShaderData for $ty {
            const WORDS: usize = 1;
            const ZERO: Self = $zero;

            unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
                // SAFETY: The caller guarantees this word belongs to a complete record.
                ($decode)(unsafe { *words.index_unchecked(offset) })
            }

            fn write(self, words: &mut [u32], offset: usize) {
                words[offset] = ($encode)(self);
            }
        }
    };
}
scalar!(u32, 0, |word| word, |value| value);
scalar!(i32, 0, |word: u32| word as Self, |value: Self| value as u32);
scalar!(f32, 0.0, Self::from_bits, Self::to_bits);
scalar!(bool, false, |word| word != 0, u32::from);
scalar!(Unorm8x4, Self(0), Self, |value: Self| value.0);
scalar!(F16x2, Self(0), Self, |value: Self| value.0);
scalar!(Unorm16x2, Self(0), Self, |value: Self| value.0);

impl ShaderData for () {
    const WORDS: usize = 0;
    const ZERO: Self = ();

    unsafe fn read_unchecked(_: &[u32], _: usize) -> Self {}

    fn write(self, _: &mut [u32], _: usize) {}
}

#[expect(clippy::needless_range_loop, reason = "Rust-GPU cannot lower the array slice iterators")]
impl<T: ShaderData, const N: usize> ShaderData for [T; N] {
    const WORDS: usize = T::WORDS * N;
    const ZERO: Self = [T::ZERO; N];

    unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
        let mut values = Self::ZERO;
        for index in 0..N {
            // SAFETY: Each element lies within the complete array guaranteed by the caller.
            let value = unsafe { T::read_unchecked(words, offset + index * T::WORDS) };
            // SAFETY: The loop index is strictly below the array length.
            unsafe {
                *values.index_unchecked_mut(index) = value;
            }
        }
        values
    }

    fn write(self, words: &mut [u32], offset: usize) {
        for index in 0..N {
            self[index].write(words, offset + index * T::WORDS);
        }
    }
}

macro_rules! vector {
    ($ty:ty, $scalar:ty, $length:literal, $($field:ident: $index:literal),+) => {
        impl ShaderData for $ty {
            const WORDS: usize = $length;
            const ZERO: Self = Self::ZERO;

            unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
                // SAFETY: Each fixed component lies within the caller's complete vector record.
                unsafe { Self::new($(<$scalar>::read_unchecked(words, offset + $index)),+) }
            }

            fn write(self, words: &mut [u32], offset: usize) {
                $(self.$field.write(words, offset + $index);)+
            }
        }
    };
}
vector!(Vec2, f32, 2, x: 0, y: 1);
vector!(Vec3, f32, 3, x: 0, y: 1, z: 2);
vector!(Vec4, f32, 4, x: 0, y: 1, z: 2, w: 3);
vector!(UVec2, u32, 2, x: 0, y: 1);
vector!(UVec3, u32, 3, x: 0, y: 1, z: 2);
vector!(UVec4, u32, 4, x: 0, y: 1, z: 2, w: 3);

/// Reads one element from a packed array of shader values.
#[doc(hidden)]
pub fn load<T: ShaderData>(words: &[u32], index: u32) -> T {
    if T::WORDS == 0 || index as usize >= words.len() / T::WORDS {
        return T::ZERO;
    }
    // SAFETY: The index is below the number of complete encoded records.
    unsafe { T::read_unchecked(words, index as usize * T::WORDS) }
}

/// # Safety
/// The indexed record must have been encoded in full using this codec.
#[doc(hidden)]
pub unsafe fn load_unchecked<T: ShaderData>(words: &[u32], index: u32) -> T {
    // SAFETY: The caller guarantees a complete encoded record at this index.
    unsafe { T::read_unchecked(words, index as usize * T::WORDS) }
}

#[doc(hidden)]
#[derive(Clone, Copy, Default, crate::ShaderData)]
pub struct FrameData {
    pub screen_size: Vec2,
    pub time: f32,
}

/// Straight-alpha color operations; shader output conversion belongs to Isthmus.
pub trait ColorExt {
    #[must_use]
    /// Multiplies alpha by `opacity`, leaving straight RGB unchanged.
    fn opacity(self, opacity: f32) -> Self;
}

impl ColorExt for Vec4 {
    fn opacity(self, opacity: f32) -> Self {
        self.truncate().extend(self.w * opacity)
    }
}
