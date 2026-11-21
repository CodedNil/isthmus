use super::{
    buffer::{Buffer, Memory, allocation_memory},
    error::RenderError,
};
use crate::Image;
use ash::vk;
use gpu_allocator::{MemoryLocation, vulkan::Allocation};
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Weak},
};

/// Formats used by Isthmus's color attachments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8Unorm,
    Bgra8Unorm,
    Rgba8Srgb,
    Bgra8Srgb,
    Rgba16Float,
}

impl TextureFormat {
    pub const fn raw(self) -> vk::Format {
        match self {
            Self::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
            Self::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
            Self::Rgba8Srgb => vk::Format::R8G8B8A8_SRGB,
            Self::Bgra8Srgb => vk::Format::B8G8R8A8_SRGB,
            Self::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        }
    }
}

pub const COLOR_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};
pub const SAMPLED_LAYOUT: vk::ImageLayout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
pub struct Texture {
    memory: Rc<Memory>,
    allocation: Option<Allocation>,
    pub raw: vk::Image,
    pub view: vk::ImageView,
    pub size: vk::Extent2D,
    pub format: vk::Format,
    initialized: Cell<bool>,
}

impl Texture {
    fn new(
        memory: &Rc<Memory>,
        size: [u32; 2],
        format: TextureFormat,
        usage: vk::ImageUsageFlags,
    ) -> Result<Rc<Self>, RenderError> {
        let limit = memory.device.properties.limits.max_image_dimension2_d;
        if size.contains(&0) || size.iter().any(|&dimension| dimension > limit) {
            return Err(RenderError::Gpu(format!("invalid texture size {size:?}, limit {limit}")));
        }
        let device = &memory.device.raw;
        let size = vk::Extent2D { width: size[0], height: size[1] };
        let info = vk::ImageCreateInfo::default()
            .format(format.raw())
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D { width: size.width, height: size.height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage);
        // SAFETY: the create info describes a single-level 2D image with supported dimensions.
        let raw = unsafe { device.create_image(&info, None) }?;
        let mut texture = Self {
            memory: Rc::clone(memory),
            allocation: None,
            raw,
            view: vk::ImageView::null(),
            size,
            format: format.raw(),
            initialized: Cell::new(false),
        };
        // SAFETY: the image is live and was created by this device.
        let requirements = unsafe { device.get_image_memory_requirements(raw) };
        texture.allocation = Some(memory.allocate(requirements, MemoryLocation::GpuOnly, false)?);
        let allocation = texture.allocation.as_ref().unwrap();
        // SAFETY: the texture owns the allocation.
        let memory_handle = unsafe { allocation_memory(allocation) };
        // SAFETY: the allocation covers the requirements of this live image.
        unsafe { device.bind_image_memory(raw, memory_handle, allocation.offset()) }?;
        if usage.contains(vk::ImageUsageFlags::COLOR_ATTACHMENT) {
            texture.view = view(device, raw, format.raw())?;
        }
        Ok(Rc::new(texture))
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        // SAFETY: successful submissions retain every image they use until their fence signals.
        unsafe {
            self.memory.device.raw.destroy_image_view(self.view, None);
        }
        // SAFETY: the image view is gone and the image is no longer in use.
        unsafe {
            self.memory.device.raw.destroy_image(self.raw, None);
        }
        if let Some(allocation) = self.allocation.take() {
            self.memory.free(allocation);
        }
    }
}

pub fn view_info(image: vk::Image, format: vk::Format) -> vk::ImageViewCreateInfo<'static> {
    vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(COLOR_RANGE)
}

pub fn view(device: &ash::Device, image: vk::Image, format: vk::Format) -> Result<vk::ImageView, vk::Result> {
    // SAFETY: callers pass a live single-layer 2D color image and its creation format.
    unsafe { device.create_image_view(&view_info(image, format), None) }
}

type Key = (usize, [u32; 2]);

fn key(image: &Image) -> Key {
    (image.pixels.as_ptr() as usize, image.size)
}

struct Cached {
    source: Weak<[u8]>,
    texture: Rc<Texture>,
}

#[derive(Default)]
pub struct Images {
    cache: HashMap<Key, Cached>,
    targets: HashMap<Key, (Weak<[u8]>, TextureFormat)>,
}

impl Images {
    pub fn register(&mut self, image: &Image, format: TextureFormat) {
        self.targets.insert(key(image), (Arc::downgrade(&image.pixels), format));
    }

    pub fn target_format(&self, image: &Image) -> TextureFormat {
        self.targets[&key(image)].1
    }

    pub fn retain_live(&mut self) {
        self.cache.retain(|_, entry| entry.source.strong_count() != 0 && entry.texture.initialized.get());
        self.targets.retain(|_, (source, _)| source.strong_count() != 0);
    }

    pub fn resolve(
        &mut self,
        memory: &Rc<Memory>,
        image: &Image,
        uploads: &mut Vec<Upload>,
    ) -> Result<Rc<Texture>, RenderError> {
        let key = key(image);
        if let Some(cached) = self.cache.get(&key) {
            return Ok(Rc::clone(&cached.texture));
        }
        let target = self.targets.get(&key).map(|(_, format)| *format);
        let mut usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
        let staging = if target.is_some() {
            usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
            None
        } else {
            let pixels = image.pixels().ok_or(RenderError::MissingImage)?;
            let mut staging = Buffer::new(memory, pixels.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC)?;
            staging.write(0, pixels);
            Some(staging)
        };
        let texture = Texture::new(memory, image.size, target.unwrap_or(TextureFormat::Rgba8Unorm), usage)?;
        uploads.push(Upload { texture: Rc::clone(&texture), staging });
        self.cache.insert(key, Cached { source: Arc::downgrade(&image.pixels), texture: Rc::clone(&texture) });
        Ok(texture)
    }
}

pub struct Upload {
    pub texture: Rc<Texture>,
    pub staging: Option<Buffer>,
}

impl Upload {
    pub fn record(&self, device: &ash::Device, cmd: vk::CommandBuffer) {
        let texture = &self.texture;
        transition(device, cmd, texture.raw, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        if let Some(staging) = &self.staging {
            let copy = vk::BufferImageCopy::default()
                .image_subresource(
                    vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1),
                )
                .image_extent(vk::Extent3D { width: texture.size.width, height: texture.size.height, depth: 1 });
            // SAFETY: the staging buffer and image are retained and the barrier enables transfer writes.
            unsafe {
                device.cmd_copy_buffer_to_image(
                    cmd,
                    staging.raw,
                    texture.raw,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[copy],
                );
            }
        } else {
            // SAFETY: the image is retained and the barrier enables transfer writes.
            unsafe {
                device.cmd_clear_color_image(
                    cmd,
                    texture.raw,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &vk::ClearColorValue { float32: [0.0; 4] },
                    &[COLOR_RANGE],
                );
            }
        }
        transition(device, cmd, texture.raw, vk::ImageLayout::TRANSFER_DST_OPTIMAL, SAMPLED_LAYOUT);
        texture.initialized.set(true);
    }
}

pub fn transition(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    from: vk::ImageLayout,
    to: vk::ImageLayout,
) {
    let scope = |layout| match layout {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => (vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_WRITE),
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => (
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        ),
        SAMPLED_LAYOUT => (vk::PipelineStageFlags2::FRAGMENT_SHADER, vk::AccessFlags2::SHADER_SAMPLED_READ),
        _ => (vk::PipelineStageFlags2::NONE, vk::AccessFlags2::NONE),
    };
    let (src_stage, src_access) = scope(from);
    let (dst_stage, dst_access) = scope(to);
    let barriers = [vk::ImageMemoryBarrier2::default()
        .image(image)
        .subresource_range(COLOR_RANGE)
        .old_layout(from)
        .new_layout(to)
        .src_stage_mask(src_stage)
        .src_access_mask(src_access)
        .dst_stage_mask(dst_stage)
        .dst_access_mask(dst_access)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)];
    // SAFETY: callers pass the actual previous layout and the commands using the image share one queue.
    unsafe {
        device.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&barriers));
    }
}
