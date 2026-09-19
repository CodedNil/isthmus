use glam::Vec4;
#[cfg(target_arch = "spirv")]
use spirv_std::{Sampler, image::Image2d};
#[cfg(not(target_arch = "spirv"))]
use std::sync::Arc;

#[cfg(not(target_arch = "spirv"))]
/// Shared RGBA8 pixels captured by a shader.
#[derive(Clone)]
pub struct Image {
    pub(crate) size: [u32; 2],
    pub(crate) pixels: Arc<[u8]>,
}

#[cfg(not(target_arch = "spirv"))]
impl Image {
    /// Creates straight-alpha RGBA8 data, requiring nonzero dimensions and exactly four bytes per pixel.
    pub fn rgba8(size: [u32; 2], pixels: impl Into<Arc<[u8]>>) -> Self {
        assert!(size[0] > 0 && size[1] > 0, "image dimensions must be non-zero");
        let mut pixels = pixels.into();
        assert_eq!(
            Some(pixels.len()),
            size.into_iter().try_fold(4usize, |bytes, dimension| bytes.checked_mul(dimension as usize)),
            "RGBA8 image data has the wrong length"
        );
        for rgba in Arc::make_mut(&mut pixels).as_chunks_mut::<4>().0 {
            let alpha = u16::from(rgba[3]);
            for channel in &mut rgba[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        Self { size, pixels }
    }

    /// Samples straight RGBA after filtering premultiplied pixels to avoid transparent-edge halos.
    pub fn sample(&self, uv: glam::Vec2) -> Vec4 {
        let size = glam::IVec2::from_array(self.size.map(|value| value as i32));
        let pixel = |point: glam::IVec2| {
            let point = point.clamp(glam::IVec2::ZERO, size - 1);
            let offset = (point.y * size.x + point.x) as usize * 4;
            let rgba = &self.pixels[offset..offset + 4];
            Vec4::new(f32::from(rgba[0]), f32::from(rgba[1]), f32::from(rgba[2]), f32::from(rgba[3])) / 255.0
        };
        let position = uv.clamp(glam::Vec2::ZERO, glam::Vec2::ONE) * size.as_vec2() - 0.5;
        let lower = position.floor().as_ivec2();
        let fraction = position - position.floor();
        straight(
            pixel(lower)
                .lerp(pixel(lower + glam::IVec2::X), fraction.x)
                .lerp(pixel(lower + glam::IVec2::Y).lerp(pixel(lower + glam::IVec2::ONE), fraction.x), fraction.y),
        )
    }
}

#[cfg(target_arch = "spirv")]
/// A texture and sampler bound for one captured image.
pub struct Image<'a> {
    image: &'a Image2d,
    sampler: Sampler,
}

#[cfg(target_arch = "spirv")]
impl<'a> Image<'a> {
    pub const fn new(image: &'a Image2d, sampler: Sampler) -> Self {
        Self { image, sampler }
    }

    pub fn sample(&self, uv: glam::Vec2) -> Vec4 {
        straight(self.image.sample_by_lod(self.sampler, uv, 0.0))
    }
}

fn straight(color: Vec4) -> Vec4 {
    (color.truncate() / color.w.max(f32::MIN_POSITIVE)).extend(color.w)
}
