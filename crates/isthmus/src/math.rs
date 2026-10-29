use crate::glam::{Vec2, Vec3, Vec4};

/// Numeric minimum and maximum, using native instructions on the GPU.
pub trait MinMax: Sized {
    fn min_num(self, other: Self) -> Self;
    fn max_num(self, other: Self) -> Self;

    /// Clamp with ordered finite bounds, without a host panic path in shaders.
    /// Rust-GPU must inline any function that can abort; `f32::clamp` therefore
    /// duplicates otherwise shared SDF dispatch even for constant valid bounds.
    #[inline]
    fn clamp_num(self, min: Self, max: Self) -> Self {
        self.max_num(min).min_num(max)
    }
}

#[cfg(target_arch = "spirv")]
#[inline(always)]
fn native<T: Copy + Default, const OP: u32>(a: T, b: T) -> T {
    let mut result = T::default();
    // SAFETY: Callers use matching float/vector types; NMin/NMax only access the supplied values.
    unsafe {
        core::arch::asm!(
            "%glsl = OpExtInstImport \"GLSL.std.450\"",
            "%a = OpLoad _ {a}",
            "%b = OpLoad _ {b}",
            "%value = OpExtInst typeof*{result} %glsl {op} %a %b",
            "OpStore {result} %value",
            a = in(reg) &a,
            b = in(reg) &b,
            result = in(reg) &mut result,
            op = const OP,
        );
    }
    result
}

macro_rules! impl_min_max {
    ($($ty:ty),+) => {$(
        impl MinMax for $ty {
            #[inline(always)]
            fn min_num(self, other: Self) -> Self {
                #[cfg(target_arch = "spirv")]
                { native::<_, 79>(self, other) }
                #[cfg(not(target_arch = "spirv"))]
                { self.min(other) }
            }
            #[inline(always)]
            fn max_num(self, other: Self) -> Self {
                #[cfg(target_arch = "spirv")]
                { native::<_, 80>(self, other) }
                #[cfg(not(target_arch = "spirv"))]
                { self.max(other) }
            }
        }
    )+};
}
impl_min_max!(f32, Vec2, Vec3, Vec4);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_clamp_matches_ordered_scalar_and_vector_ranges() {
        for lower in [-100.0f32, -1.0, 0.0, 0.5] {
            for width in [0.0, 0.001, 1.0, 200.0] {
                let upper = lower + width;
                for step in -500..=500 {
                    let value = step as f32 * 0.31;
                    assert_eq!(value.clamp_num(lower, upper), value.clamp(lower, upper));
                    let vector = Vec3::new(value, -value, value * 0.5);
                    assert_eq!(
                        vector.clamp_num(Vec3::splat(lower), Vec3::splat(upper)),
                        vector.clamp(Vec3::splat(lower), Vec3::splat(upper))
                    );
                }
            }
        }
    }
}
