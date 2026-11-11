//! Host-side images: shared RGBA8 pixels, or the identity of a texture a renderer owns.
use super::straight;
use crate::glam::{self, Vec4};
use std::sync::Arc;

/// Shared image pixels or an identity registered with a renderer.
#[derive(Clone)]
pub struct Image {
    pub(crate) size: [u32; 2],
    pub(crate) pixels: Arc<[u8]>,
}

impl Image {
    /// Creates straight-alpha RGBA8 data, requiring nonzero dimensions and exactly four bytes per pixel.
    pub fn rgba8(size: [u32; 2], pixels: impl Into<Arc<[u8]>>) -> Self {
        let mut image = Self::raw_rgba8(size, pixels);
        for rgba in Arc::make_mut(&mut image.pixels).as_chunks_mut::<4>().0 {
            let alpha = u16::from(rgba[3]);
            for channel in &mut rgba[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        image
    }

    /// Host-side texels, or `None` for images backed only by a GPU texture.
    pub fn pixels(&self) -> Option<&[u8]> {
        (!self.pixels.is_empty()).then_some(&self.pixels)
    }

    pub const fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Unmodified RGBA8 texels for data textures, without alpha premultiplication.
    pub fn raw_rgba8(size: [u32; 2], pixels: impl Into<Arc<[u8]>>) -> Self {
        let pixels = pixels.into();
        assert!(size[0] > 0 && size[1] > 0, "image dimensions must be non-zero");
        assert_eq!(
            Some(pixels.len()),
            size.into_iter().try_fold(4usize, |bytes, dimension| bytes.checked_mul(dimension as usize)),
            "RGBA8 image data has the wrong length"
        );
        Self { size, pixels }
    }

    /// Samples straight RGBA after filtering premultiplied pixels to avoid transparent-edge halos.
    pub fn sample(&self, uv: glam::Vec2) -> Vec4 {
        straight(self.sample_raw(uv))
    }

    /// Loads one clamped texel without interpolating packed data channels.
    pub fn texel(&self, pixel: glam::IVec2) -> Vec4 {
        self.sample_raw(
            (pixel.as_vec2() + glam::Vec2::splat(0.5)) / glam::Vec2::from_array(self.size.map(|v| v as f32)),
        )
    }

    /// Filters the stored channels directly, for depth/moment data textures.
    pub fn sample_raw(&self, uv: glam::Vec2) -> Vec4 {
        let pixels = self.pixels().expect("GPU images cannot be sampled on the CPU");
        let size = glam::IVec2::from_array(self.size.map(|value| value as i32));
        let pixel = |point: glam::IVec2| {
            let point = point.clamp(glam::IVec2::ZERO, size - 1);
            let offset = (point.y * size.x + point.x) as usize * 4;
            let rgba = &pixels[offset..offset + 4];
            Vec4::new(f32::from(rgba[0]), f32::from(rgba[1]), f32::from(rgba[2]), f32::from(rgba[3])) / 255.0
        };
        let position = uv.clamp(glam::Vec2::ZERO, glam::Vec2::ONE) * size.as_vec2() - 0.5;
        let lower = position.floor().as_ivec2();
        let fraction = position - position.floor();
        pixel(lower)
            .lerp(pixel(lower + glam::IVec2::X), fraction.x)
            .lerp(pixel(lower + glam::IVec2::Y).lerp(pixel(lower + glam::IVec2::ONE), fraction.x), fraction.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_texture_channels_are_independent_of_alpha() {
        let image = Image::raw_rgba8([1, 1], vec![128, 64, 255, 0]);
        assert_eq!(image.sample_raw(glam::Vec2::splat(0.5)), Vec4::new(128.0, 64.0, 255.0, 0.0) / 255.0);
    }
    #[test]
    fn rgba_images_still_filter_premultiplied_colour() {
        let image = Image::rgba8([2, 1], vec![255, 0, 0, 255, 0, 0, 255, 0]);
        let value = image.sample(glam::Vec2::splat(0.5));
        assert_eq!(value, Vec4::new(1.0, 0.0, 0.0, 0.5));
    }
}
