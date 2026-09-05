#[cfg(not(target_arch = "spirv"))]
use std::sync::Arc;

/// Filtering and addressing for a sampled image; linear clamping is the default.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(usize)]
pub enum Sampling {
    #[default]
    Linear,
    Nearest,
    LinearRepeat,
    NearestRepeat,
}

#[cfg(not(target_arch = "spirv"))]
#[derive(Clone)]
pub struct Image {
    pub(crate) size: [u32; 2],
    pub(crate) pixels: Arc<[u8]>,
    pub(crate) sampling: Sampling,
}

#[cfg(target_arch = "spirv")]
pub struct Image;

#[cfg(not(target_arch = "spirv"))]
impl Image {
    /// # Panics
    /// Panics when either dimension is zero or `pixels` does not contain exactly four bytes per pixel.
    pub fn rgba8(size: [u32; 2], pixels: impl Into<Arc<[u8]>>) -> Self {
        assert!(size[0] > 0 && size[1] > 0, "image dimensions must be non-zero");
        let pixels = pixels.into();
        assert_eq!(
            Some(pixels.len()),
            size.into_iter().try_fold(4usize, |bytes, dimension| bytes.checked_mul(dimension as usize)),
            "RGBA8 image data has the wrong length"
        );
        Self { size, pixels, sampling: Sampling::default() }
    }

    #[must_use]
    pub fn sampled(&self, sampling: Sampling) -> Self {
        Self { sampling, ..self.clone() }
    }

    pub fn sample(&self, uv: glam::Vec2) -> glam::Vec4 {
        let size = glam::IVec2::from_array(self.size.map(|value| value as i32));
        let repeat = matches!(self.sampling, Sampling::LinearRepeat | Sampling::NearestRepeat);
        let pixel = |point: glam::IVec2| {
            let point = if repeat {
                glam::ivec2(point.x.rem_euclid(size.x), point.y.rem_euclid(size.y))
            } else {
                point.clamp(glam::IVec2::ZERO, size - 1)
            };
            let offset = (point.y * size.x + point.x) as usize * 4;
            let rgba = &self.pixels[offset..offset + 4];
            glam::Vec4::new(f32::from(rgba[0]), f32::from(rgba[1]), f32::from(rgba[2]), f32::from(rgba[3])) / 255.0
        };
        let uv = if repeat { uv - uv.floor() } else { uv.clamp(glam::Vec2::ZERO, glam::Vec2::ONE) };
        if matches!(self.sampling, Sampling::Nearest | Sampling::NearestRepeat) {
            return pixel((uv * size.as_vec2()).floor().as_ivec2());
        }
        let position = uv * size.as_vec2() - 0.5;
        let lower = position.floor().as_ivec2();
        let fraction = position - position.floor();
        pixel(lower)
            .lerp(pixel(lower + glam::IVec2::X), fraction.x)
            .lerp(pixel(lower + glam::IVec2::Y).lerp(pixel(lower + glam::IVec2::ONE), fraction.x), fraction.y)
    }
}

#[cfg(test)]
mod tests {
    use super::{Image, Sampling};
    use glam::{Vec4, vec2};

    #[test]
    fn sampling_handles_texel_centers_clamped_edges_and_repeat_seams() {
        let image = Image::rgba8([2, 1], [255, 0, 0, 255, 0, 0, 255, 255]);
        let red = Vec4::new(1.0, 0.0, 0.0, 1.0);
        let blue = Vec4::new(0.0, 0.0, 1.0, 1.0);
        let purple = red.lerp(blue, 0.5);
        for (sampling, x, expected) in [
            (Sampling::Linear, 0.25, red),
            (Sampling::Linear, 0.5, purple),
            (Sampling::Linear, -0.1, red),
            (Sampling::Nearest, 0.49, red),
            (Sampling::Nearest, 0.51, blue),
            (Sampling::Nearest, 1.0, blue),
            (Sampling::LinearRepeat, 0.0, purple),
            (Sampling::LinearRepeat, 1.0, purple),
            (Sampling::NearestRepeat, -0.25, blue),
        ] {
            assert!(image.sampled(sampling).sample(vec2(x, 0.5)).abs_diff_eq(expected, 0.0001));
        }
    }
}
