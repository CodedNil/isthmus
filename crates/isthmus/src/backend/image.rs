use crate::Image;
use core::array::from_fn;
use smallvec::SmallVec;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use wgpu::util::{DeviceExt, TextureDataOrder};

struct CachedImage {
    source: Weak<[u8]>,
    view: wgpu::TextureView,
}

type ImageKey = (usize, [u32; 2]);
type ImageBindings = SmallVec<[(ImageKey, usize); 2]>;

pub(super) struct ImageCache {
    images: HashMap<ImageKey, CachedImage>,
    groups: HashMap<ImageBindings, wgpu::BindGroup>,
    pub layouts: Vec<wgpu::BindGroupLayout>,
    samplers: [wgpu::Sampler; 4],
}

impl ImageCache {
    pub fn new(device: &wgpu::Device, count: usize) -> Self {
        let layouts: Vec<_> = (0..=count)
            .map(|count| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("image"),
                    entries: &(0..count)
                        .flat_map(|index| {
                            [
                                wgpu::BindGroupLayoutEntry {
                                    binding: index as u32 * 2,
                                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                                    ty: wgpu::BindingType::Texture {
                                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                        view_dimension: wgpu::TextureViewDimension::D2,
                                        multisampled: false,
                                    },
                                    count: None,
                                },
                                wgpu::BindGroupLayoutEntry {
                                    binding: index as u32 * 2 + 1,
                                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                    count: None,
                                },
                            ]
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
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
        Self { images: HashMap::new(), groups: HashMap::new(), layouts, samplers }
    }

    pub fn retain_live(&mut self) {
        self.images.retain(|_, image| image.source.strong_count() != 0);
        self.groups.retain(|key, _| key.iter().all(|(image, _)| self.images.contains_key(image)));
    }

    pub fn images(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, images: &[&Image]) -> wgpu::BindGroup {
        let key: ImageBindings = images
            .iter()
            .map(|image| ((image.pixels.as_ptr() as usize, image.size), image.sampling as usize))
            .collect();
        if let Some(group) = self.groups.get(&key) {
            return group.clone();
        }
        for (&(key, _), image) in key.iter().zip(images) {
            self.images.entry(key).or_insert_with(|| CachedImage {
                source: Arc::downgrade(&image.pixels),
                view: upload(device, queue, image.size, &image.pixels),
            });
        }
        let entries: Vec<_> = key
            .iter()
            .enumerate()
            .flat_map(|(index, (key, sampling))| {
                [
                    wgpu::BindGroupEntry {
                        binding: index as u32 * 2,
                        resource: wgpu::BindingResource::TextureView(&self.images[key].view),
                    },
                    wgpu::BindGroupEntry {
                        binding: index as u32 * 2 + 1,
                        resource: wgpu::BindingResource::Sampler(&self.samplers[*sampling]),
                    },
                ]
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("images"),
            layout: &self.layouts[images.len()],
            entries: &entries,
        });
        self.groups.insert(key, group.clone());
        group
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
