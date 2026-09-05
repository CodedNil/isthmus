use crate::{Image, bindings};
use core::array::from_fn;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

struct CachedImage {
    source: Weak<[u8]>,
    view: wgpu::TextureView,
    bindings: [Option<wgpu::BindGroup>; 4],
}

pub(super) struct ImageCache {
    images: HashMap<(usize, [u32; 2]), CachedImage>,
    pub layout: wgpu::BindGroupLayout,
    samplers: [wgpu::Sampler; 4],
    pub fallback: wgpu::BindGroup,
}

impl ImageCache {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: bindings::IMAGE,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: bindings::SAMPLER,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let samplers = from_fn(|index| {
            let filter = if index % 2 == 0 { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
            let address = if index < 2 { wgpu::AddressMode::ClampToEdge } else { wgpu::AddressMode::Repeat };
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("isthmus"),
                mag_filter: filter,
                min_filter: filter,
                address_mode_u: address,
                address_mode_v: address,
                ..Default::default()
            })
        });
        let fallback = bind(device, &layout, &upload(device, queue, [1, 1], &[255; 4]), &samplers[0]);
        Self { images: HashMap::new(), layout, samplers, fallback }
    }

    pub fn retain_live(&mut self) {
        self.images.retain(|_, image| image.source.strong_count() != 0);
    }

    pub fn image(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> wgpu::BindGroup {
        let key = (image.pixels.as_ptr() as usize, image.size);
        let cached = self.images.entry(key).or_insert_with(|| CachedImage {
            source: Arc::downgrade(&image.pixels),
            view: upload(device, queue, image.size, &image.pixels),
            bindings: from_fn(|_| None),
        });
        let sampling = image.sampling as usize;
        cached.bindings[sampling]
            .get_or_insert_with(|| bind(device, &self.layout, &cached.view, &self.samplers[sampling]))
            .clone()
    }
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("image"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: bindings::IMAGE, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: bindings::SAMPLER, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2], pixels: &[u8]) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("isthmus image"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
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
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
