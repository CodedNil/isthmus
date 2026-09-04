use super::context::Context;
use std::{
    rc::Rc,
    sync::{Arc, Weak},
};

pub(super) struct Image {
    /// Keeps the backing resource alive for the view used by bind groups.
    #[expect(dead_code, reason = "the texture owns the resource sampled by the view")]
    texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
}

struct CachedImage {
    source: Weak<[u8]>,
    image: Rc<Image>,
}

pub(super) struct ImageCache {
    images: Vec<CachedImage>,
    fallback: CachedImage,
}

impl ImageCache {
    pub(crate) fn new(context: &Context) -> Self {
        let fallback = Arc::from([255, 255, 255, 255]);
        Self { images: Vec::new(), fallback: upload(context, [1, 1], &fallback) }
    }

    pub(crate) fn begin_frame(&mut self) {
        self.images.retain(|image| image.source.strong_count() != 0);
    }

    pub(crate) fn image(&mut self, context: &Context, size: [u32; 2], pixels: &Arc<[u8]>) -> Rc<Image> {
        let source = Arc::downgrade(pixels);
        if let Some(cached) = self.images.iter().find(|cached| cached.source.ptr_eq(&source)) {
            return Rc::clone(&cached.image);
        }
        let cached = upload(context, size, pixels);
        let image = Rc::clone(&cached.image);
        self.images.push(cached);
        image
    }

    pub(crate) const fn fallback(&self) -> &Rc<Image> {
        &self.fallback.image
    }
}

fn upload(context: &Context, size: [u32; 2], pixels: &Arc<[u8]>) -> CachedImage {
    let texture = context.0.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("isthmus image"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    context.0.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * 4), rows_per_image: Some(size[1]) },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let image = Rc::new(Image { texture, view });
    CachedImage { source: Arc::downgrade(pixels), image }
}
