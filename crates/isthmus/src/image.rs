use spirv_std::{Sampler, image::Image2d};
#[cfg(not(target_arch = "spirv"))]
use std::sync::Arc;

/// Filtering and addressing for a sampled image; linear clamping is the default.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(usize)]
pub enum Sampling {
    #[default]
    /// Linear filtering with coordinates clamped to the image edges.
    Linear,
    /// Nearest-pixel filtering with coordinates clamped to the image edges.
    Nearest,
    /// Linear filtering with repeated image coordinates.
    LinearRepeat,
    /// Nearest-pixel filtering with repeated image coordinates.
    NearestRepeat,
}

#[cfg(not(target_arch = "spirv"))]
/// Shared RGBA8 pixels and sampling settings captured by a shader.
#[derive(Clone)]
pub struct Image {
    pub(crate) size: [u32; 2],
    pub(crate) pixels: Arc<[u8]>,
    pub(crate) sampling: Sampling,
}

#[cfg(target_arch = "spirv")]
/// Image capture marker replaced with a texture and sampler by shader generation.
pub struct Image;

#[cfg(not(target_arch = "spirv"))]
impl Image {
    /// Creates a straight-alpha RGBA8 image with default linear sampling.
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
    /// Changes sampling settings while sharing the original pixel storage.
    pub fn sampled(&self, sampling: Sampling) -> Self {
        Self { sampling, ..self.clone() }
    }

    /// Samples normalized coordinates using this image's filtering and addressing settings.
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

#[doc(hidden)]
pub struct ShaderImage<'a> {
    image: &'a Image2d,
    sampler: Sampler,
}

impl<'a> ShaderImage<'a> {
    pub const fn new(image: &'a Image2d, sampler: Sampler) -> Self {
        Self { image, sampler }
    }

    pub fn sample(&self, uv: glam::Vec2) -> glam::Vec4 {
        self.image.sample(self.sampler, uv)
    }
}
