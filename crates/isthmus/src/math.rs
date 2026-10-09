use crate::glam::{Vec2, Vec3, Vec4};

/// Numeric minimum and maximum, using native instructions on the GPU.
pub trait MinMax: Sized {
    fn min_num(self, other: Self) -> Self;
    fn max_num(self, other: Self) -> Self;
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
