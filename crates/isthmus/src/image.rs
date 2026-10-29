use crate::glam::{self, Vec4};
#[cfg(target_arch = "spirv")]
use spirv_std::{Sampler, image::Image2d};
#[cfg(not(target_arch = "spirv"))]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use std::{cell::RefCell, collections::HashMap};

#[cfg(not(target_arch = "spirv"))]
/// Shared RGBA8 pixels captured by a shader.
#[derive(Clone)]
pub struct Image {
    pub(crate) size: [u32; 2],
    pub(crate) pixels: Arc<[u8]>,
    #[cfg(not(target_arch = "wasm32"))]
    view: Option<wgpu::TextureView>,
    #[cfg(target_arch = "wasm32")]
    view: Option<usize>,
}

#[cfg(target_arch = "wasm32")]
type GpuViews = HashMap<usize, wgpu::TextureView>;

#[cfg(target_arch = "wasm32")]
thread_local! {
    // WebGPU handles are local to their JavaScript context. Keep them out of
    // CPU image data so ordinary images remain Send + Sync on every target.
    static GPU_VIEWS: RefCell<GpuViews> = RefCell::new(GpuViews::new());
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
        Self { size, pixels, view: None }
    }

    /// Wraps a filterable, single-layer GPU texture for shader sampling.
    /// The texture must belong to the renderer's device and support `TEXTURE_BINDING`.
    pub fn from_texture(texture: &wgpu::Texture) -> Self {
        let pixels: Arc<[u8]> = vec![0].into();
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        #[cfg(target_arch = "wasm32")]
        let view = {
            let key = pixels.as_ptr() as usize;
            GPU_VIEWS.with(|views| views.borrow_mut().insert(key, view));
            key
        };
        Self {
            size: [texture.width(), texture.height()],
            // Unique identity and lifetime token for the renderer's image cache.
            pixels,
            view: Some(view),
        }
    }

    pub(crate) fn texture_view(&self) -> Option<wgpu::TextureView> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.view.clone()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.view.map(|key| {
                GPU_VIEWS.with(|views| {
                    views.borrow().get(&key).expect("GPU image must be used in its creating JavaScript context").clone()
                })
            })
        }
    }

    /// Host-side texels, or `None` for images backed only by a GPU texture.
    pub fn pixels(&self) -> Option<&[u8]> {
        self.view.is_none().then_some(&self.pixels)
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
        Self { size, pixels, view: None }
    }

    /// Samples straight RGBA after filtering premultiplied pixels to avoid transparent-edge halos.
    pub fn sample(&self, uv: glam::Vec2) -> Vec4 {
        straight(self.sample_raw(uv))
    }

    /// Filters the stored channels directly, for depth/moment data textures.
    pub fn sample_raw(&self, uv: glam::Vec2) -> Vec4 {
        assert!(self.view.is_none(), "GPU images cannot be sampled on the CPU");
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
        pixel(lower)
            .lerp(pixel(lower + glam::IVec2::X), fraction.x)
            .lerp(pixel(lower + glam::IVec2::Y).lerp(pixel(lower + glam::IVec2::ONE), fraction.x), fraction.y)
    }
}

#[cfg(all(not(target_arch = "spirv"), target_arch = "wasm32"))]
impl Drop for Image {
    fn drop(&mut self) {
        if let Some(key) = self.view
            && Arc::strong_count(&self.pixels) == 1
        {
            GPU_VIEWS.with(|views| views.borrow_mut().remove(&key));
        }
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
        straight(self.sample_raw(uv))
    }

    pub fn sample_raw(&self, uv: glam::Vec2) -> Vec4 {
        self.image.sample_by_lod(self.sampler, uv, 0.0)
    }
}

fn straight(color: Vec4) -> Vec4 {
    (color.truncate() / color.w.max(f32::MIN_POSITIVE)).extend(color.w)
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
