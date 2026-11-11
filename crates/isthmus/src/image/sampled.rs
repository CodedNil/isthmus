//! Shader-side images: a bound texture and the shared sampler.
use super::straight;
use crate::glam::{self, Vec4};
use spirv_std::{Sampler, image::Image2d};

/// A texture and sampler bound for one captured image.
pub struct Image<'a> {
    image: &'a Image2d,
    sampler: Sampler,
}

impl<'a> Image<'a> {
    pub const fn new(image: &'a Image2d, sampler: Sampler) -> Self {
        Self { image, sampler }
    }

    pub fn sample(&self, uv: glam::Vec2) -> Vec4 {
        straight(self.sample_raw(uv))
    }

    pub fn sample_raw(&self, uv: glam::Vec2) -> Vec4 {
        self.image.sample_by_lod(self.sampler, uv, 0.0)
    }

    pub fn texel(&self, pixel: glam::IVec2) -> Vec4 {
        self.image.fetch(pixel)
    }
}
