use crate::{Image, RenderError};
use smallvec::SmallVec;
use std::{
    collections::HashMap,
    iter,
    sync::{Arc, Weak},
};
use wgpu::util::{DeviceExt, TextureDataOrder};

struct CachedImage {
    source: Weak<[u8]>,
    view: wgpu::TextureView,
}

type ImageKey = (usize, [u32; 2]);
type ImageBindings = SmallVec<[ImageKey; 2]>;

pub(super) struct ImageCache {
    images: HashMap<ImageKey, CachedImage>,
    groups: HashMap<ImageBindings, wgpu::BindGroup>,
    pub layouts: Vec<wgpu::BindGroupLayout>,
    sampler: wgpu::Sampler,
}

impl ImageCache {
    pub fn new(device: &wgpu::Device, count: usize) -> Self {
        let layouts: Vec<_> = (0..=count)
            .map(|count| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("image"),
                    entries: &iter::once(wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    })
                    .chain((0..count).map(|index| wgpu::BindGroupLayoutEntry {
                        binding: index as u32 + 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }))
                    .collect::<Vec<_>>(),
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("isthmus"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        Self { images: HashMap::new(), groups: HashMap::new(), layouts, sampler }
    }

    pub fn retain_live(&mut self) {
        self.images.retain(|_, image| image.source.strong_count() != 0);
        self.groups.retain(|key, _| key.iter().all(|image| self.images.contains_key(image)));
    }

    pub fn view(&self, image: &Image) -> Option<&wgpu::TextureView> {
        self.images.get(&(image.pixels.as_ptr() as usize, image.size)).map(|cached| &cached.view)
    }

    pub fn insert(&mut self, image: &Image, view: wgpu::TextureView) {
        self.images.insert((image.pixels.as_ptr() as usize, image.size), CachedImage {
            source: Arc::downgrade(&image.pixels),
            view,
        });
    }

    pub fn images(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[&Image],
    ) -> Result<wgpu::BindGroup, RenderError> {
        let key: ImageBindings = images.iter().map(|image| (image.pixels.as_ptr() as usize, image.size)).collect();
        if let Some(group) = self.groups.get(&key) {
            return Ok(group.clone());
        }
        for (&key, image) in key.iter().zip(images) {
            if !self.images.contains_key(&key) {
                let pixels = image.pixels().ok_or(RenderError::MissingImage)?;
                self.insert(image, upload(device, queue, image.size, pixels));
            }
        }
        let entries: Vec<_> =
            iter::once(wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&self.sampler) })
                .chain(key.iter().enumerate().map(|(index, key)| wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: wgpu::BindingResource::TextureView(&self.images[key].view),
                }))
                .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("images"),
            layout: &self.layouts[images.len()],
            entries: &entries,
        });
        self.groups.insert(key, group.clone());
        Ok(group)
    }
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2], pixels: &[u8]) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("isthmus image"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            pixels,
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}
