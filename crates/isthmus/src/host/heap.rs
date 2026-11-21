//! Descriptor bytes and binding offsets shared by all graphics pipelines.
use super::{
    buffer::{Buffer, Memory, UploadBuffer},
    error::RenderError,
    images::{SAMPLED_LAYOUT, Texture, view_info},
    setup::Device,
};
use ash::vk;
use isthmus_abi as abi;
use std::{mem, rc::Rc};

type Properties = vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static>;

#[derive(Clone, Copy)]
pub struct Layout {
    buffer_stride: u64,
    image_offset: u64,
    image_stride: u64,
}

impl Layout {
    pub const fn new(properties: &Properties) -> Self {
        let buffer_stride = properties.buffer_descriptor_size;
        Self {
            buffer_stride,
            image_offset: (buffer_stride * abi::BUFFER_COUNT as u64)
                .next_multiple_of(properties.image_descriptor_alignment),
            image_stride: properties.image_descriptor_size,
        }
    }

    pub fn max_images(properties: &Properties) -> u32 {
        let layout = Self::new(properties);
        let reserved = properties.min_resource_heap_reserved_range;
        let mut heap_bytes = properties.max_resource_heap_size.saturating_sub(reserved);
        if reserved != 0 {
            heap_bytes -= heap_bytes % resource_reserved_alignment(properties);
        }
        (heap_bytes.saturating_sub(layout.image_offset) / layout.image_stride).min(4096) as u32
    }

    pub fn mappings(self) -> [vk::DescriptorSetAndBindingMappingEXT<'static>; 3] {
        let mapping = |set, binding, count, mask, offset, stride| {
            vk::DescriptorSetAndBindingMappingEXT::default()
                .descriptor_set(set)
                .first_binding(binding)
                .binding_count(count)
                .resource_mask(mask)
                .source(vk::DescriptorMappingSourceEXT::HEAP_WITH_CONSTANT_OFFSET)
                .source_data(vk::DescriptorMappingSourceDataEXT {
                    constant_offset: vk::DescriptorMappingSourceConstantOffsetEXT::default()
                        .heap_offset(offset)
                        .heap_array_stride(stride),
                })
        };
        [
            mapping(
                abi::FRAME_SET,
                abi::FRAME_BINDING,
                abi::BUFFER_COUNT as u32,
                vk::SpirvResourceTypeFlagsEXT::READ_ONLY_STORAGE_BUFFER,
                0,
                self.buffer_stride as u32,
            ),
            mapping(abi::IMAGE_SET, abi::SAMPLER_BINDING, 1, vk::SpirvResourceTypeFlagsEXT::SAMPLER, 0, 0),
            mapping(
                abi::IMAGE_SET,
                abi::IMAGE_BINDING,
                1,
                vk::SpirvResourceTypeFlagsEXT::SAMPLED_IMAGE,
                self.image_offset as u32,
                self.image_stride as u32,
            ),
        ]
    }
}

struct HeapBuffer {
    buffer: Buffer,
    binding: vk::BindHeapInfoEXT<'static>,
}

impl HeapBuffer {
    fn new(
        memory: &Rc<Memory>,
        used: u64,
        address_alignment: u64,
        reserved_alignment: u64,
        reserved: u64,
    ) -> Result<Self, RenderError> {
        let offset = if reserved == 0 { 0 } else { used.next_multiple_of(reserved_alignment) };
        let buffer = Buffer::new(
            memory,
            used.max(offset + reserved),
            vk::BufferUsageFlags::DESCRIPTOR_HEAP_EXT | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        )?;
        let range = buffer.address_range();
        if !range.address.is_multiple_of(address_alignment) {
            return Err(RenderError::Gpu("Vulkan allocated an unaligned descriptor heap".into()));
        }
        let binding = vk::BindHeapInfoEXT::default()
            .heap_range(range)
            .reserved_range_offset(offset)
            .reserved_range_size(reserved);
        Ok(Self { buffer, binding })
    }
}

fn resource_reserved_alignment(properties: &Properties) -> u64 {
    properties.buffer_descriptor_alignment.max(properties.image_descriptor_alignment)
}

pub struct FrameHeap {
    layout: Layout,
    resources: HeapBuffer,
    sampler: HeapBuffer,
    sampled: Vec<Rc<Texture>>,
}

impl FrameHeap {
    pub fn new(memory: &Rc<Memory>) -> Result<Self, RenderError> {
        let device = &memory.device;
        let properties = &device.heap_properties;
        let layout = Layout::new(properties);
        let resources = HeapBuffer::new(
            memory,
            layout.image_offset + layout.image_stride * u64::from(device.max_images),
            properties.resource_heap_alignment,
            resource_reserved_alignment(properties),
            properties.min_resource_heap_reserved_range,
        )?;
        let sampler_size = properties.sampler_descriptor_size;
        let mut sampler = HeapBuffer::new(
            memory,
            sampler_size,
            properties.sampler_heap_alignment,
            properties.sampler_descriptor_alignment,
            properties.min_sampler_heap_reserved_range,
        )?;
        let info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
        let slot = vk::HostAddressRangeEXT::default().address(&mut sampler.buffer.mapped()[..sampler_size as usize]);
        // SAFETY: the new heap is mapped, has room for this sampler, and is idle.
        unsafe { device.heap.write_sampler_descriptors(&[info], &[slot]) }?;
        Ok(Self { layout, resources, sampler, sampled: Vec::new() })
    }

    pub fn bind(
        &mut self,
        device: &Device,
        cmd: vk::CommandBuffer,
        buffers: &[UploadBuffer; abi::BUFFER_COUNT],
        sampled: &mut Vec<Rc<Texture>>,
    ) -> Result<(), RenderError> {
        let properties = &device.heap_properties;
        let mapped = self.resources.buffer.mapped();
        for (index, buffer) in buffers.iter().enumerate() {
            let offset = self.layout.buffer_stride as usize * index;
            let address = buffer.address_range();
            let descriptor = vk::ResourceDescriptorInfoEXT::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .data(vk::ResourceDescriptorDataEXT { p_address_range: &raw const address });
            let slot = vk::HostAddressRangeEXT::default()
                .address(&mut mapped[offset..][..properties.buffer_descriptor_size as usize]);
            // SAFETY: the frame fence has signalled, the buffer remains alive, and the slot fits the heap.
            unsafe { device.heap.write_resource_descriptors(&[descriptor], &[slot]) }?;
        }
        let start = self.sampled.iter().zip(sampled.iter()).take_while(|(a, b)| Rc::ptr_eq(a, b)).count();
        for (index, texture) in sampled.iter().enumerate().skip(start) {
            let offset = self.layout.image_offset as usize + index * self.layout.image_stride as usize;
            let view = view_info(texture.raw, texture.format);
            let image = vk::ImageDescriptorInfoEXT::default().view(&view).layout(SAMPLED_LAYOUT);
            let descriptor = vk::ResourceDescriptorInfoEXT::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .data(vk::ResourceDescriptorDataEXT { p_image: &raw const image });
            let slot = vk::HostAddressRangeEXT::default()
                .address(&mut mapped[offset..][..properties.image_descriptor_size as usize]);
            // SAFETY: the frame fence has signalled and recording enforces the texture table capacity.
            unsafe { device.heap.write_resource_descriptors(&[descriptor], &[slot]) }?;
        }
        mem::swap(&mut self.sampled, sampled);
        sampled.clear();
        // SAFETY: the recording command buffer and both addressable heaps belong to this device.
        // The frame owns the heaps and referenced resources until its fence signals.
        unsafe {
            device.heap.cmd_bind_resource_heap(cmd, &self.resources.binding);
        }
        // SAFETY: the frame also owns its sampler heap until the fence signals.
        unsafe {
            device.heap.cmd_bind_sampler_heap(cmd, &self.sampler.binding);
        }
        Ok(())
    }
}
