use super::{error::RenderError, setup::Device};
use ash::vk::{self, Handle as _};
use ash_allocator::vk::{self as alloc_vk, Handle as _};
use gpu_allocator::{
    AllocationSizes, AllocatorDebugSettings, MemoryLocation,
    vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator, AllocatorCreateDesc},
};
use std::{cell::RefCell, rc::Rc};

pub struct Memory {
    allocator: RefCell<Allocator>,
    _loader: ash_allocator::Entry,
    pub device: Rc<Device>,
}

impl Memory {
    pub fn new(device: Rc<Device>) -> Result<Rc<Self>, RenderError> {
        // SAFETY: this loader is held until after the allocator is destroyed.
        let loader = unsafe { ash_allocator::Entry::load() }.map_err(|error| RenderError::Gpu(error.to_string()))?;
        // SAFETY: both ash versions wrap the same Vulkan handles. These borrowed dispatch tables
        // never destroy the underlying instance or device, which the retained Device owns.
        let instance = unsafe {
            ash_allocator::Instance::load(
                loader.static_fn(),
                alloc_vk::Instance::from_raw(device.vulkan.instance.handle().as_raw()),
            )
        };
        // SAFETY: the device belongs to that instance and the retained Device owns it.
        let raw = unsafe {
            ash_allocator::Device::load(instance.fp_v1_0(), alloc_vk::Device::from_raw(device.raw.handle().as_raw()))
        };
        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance,
            device: raw,
            physical_device: alloc_vk::PhysicalDevice::from_raw(device.physical.as_raw()),
            debug_settings: AllocatorDebugSettings::default(),
            buffer_device_address: true,
            allocation_sizes: AllocationSizes::new(8 << 20, 4 << 20)
                .with_max_device_memblock_size(64 << 20)
                .with_max_host_memblock_size(32 << 20),
        })
        .map_err(|error| RenderError::Gpu(error.to_string()))?;
        Ok(Rc::new(Self { allocator: RefCell::new(allocator), _loader: loader, device }))
    }

    pub fn allocate(
        &self,
        requirements: vk::MemoryRequirements,
        location: MemoryLocation,
        linear: bool,
    ) -> Result<Allocation, RenderError> {
        self.allocator
            .borrow_mut()
            .allocate(&AllocationCreateDesc {
                name: "isthmus",
                requirements: alloc_vk::MemoryRequirements {
                    size: requirements.size,
                    alignment: requirements.alignment,
                    memory_type_bits: requirements.memory_type_bits,
                },
                location,
                linear,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|error| RenderError::Gpu(error.to_string()))
    }

    pub fn free(&self, allocation: Allocation) {
        if let Err(error) = self.allocator.borrow_mut().free(allocation) {
            tracing::error!(%error, "Vulkan allocation could not be released");
        }
    }
}

/// Returns a live native memory handle from the allocator's bindings.
///
/// # Safety
/// The allocation must remain alive throughout the Vulkan operation.
pub unsafe fn allocation_memory(allocation: &Allocation) -> vk::DeviceMemory {
    // SAFETY: the caller retains this allocation.
    vk::DeviceMemory::from_raw(unsafe { allocation.memory() }.as_raw())
}

pub struct Buffer {
    memory: Rc<Memory>,
    allocation: Option<Allocation>,
    pub raw: vk::Buffer,
    pub size: u64,
}

impl Buffer {
    pub fn new(memory: &Rc<Memory>, size: u64, usage: vk::BufferUsageFlags) -> Result<Self, RenderError> {
        let device = &memory.device.raw;
        let info = vk::BufferCreateInfo::default().size(size).usage(usage);
        // SAFETY: the create info is valid for this device and has a nonzero size.
        let raw = unsafe { device.create_buffer(&info, None) }?;
        let mut buffer = Self { memory: Rc::clone(memory), allocation: None, raw, size };
        // SAFETY: the buffer is live and was created by this device.
        let requirements = unsafe { device.get_buffer_memory_requirements(raw) };
        buffer.allocation = Some(memory.allocate(requirements, MemoryLocation::CpuToGpu, true)?);
        let allocation = buffer.allocation.as_ref().unwrap();
        // SAFETY: the allocator satisfies this buffer's memory requirements and alignment.
        let memory_handle = unsafe { allocation_memory(allocation) };
        // SAFETY: the allocation is live and meets this buffer's requirements.
        unsafe { device.bind_buffer_memory(raw, memory_handle, allocation.offset()) }?;
        Ok(buffer)
    }

    pub fn address_range(&self) -> vk::DeviceAddressRangeEXT {
        // SAFETY: this buffer is bound to addressable memory and was created with SHADER_DEVICE_ADDRESS.
        let address = unsafe {
            self.memory.device.raw.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(self.raw))
        };
        vk::DeviceAddressRangeEXT { address, size: self.size }
    }

    pub fn mapped(&mut self) -> &mut [u8] {
        &mut self.allocation.as_mut().unwrap().mapped_slice_mut().unwrap()[..self.size as usize]
    }

    pub fn write(&mut self, offset: usize, bytes: &[u8]) {
        self.allocation.as_mut().unwrap().mapped_slice_mut().unwrap()[offset..offset + bytes.len()]
            .copy_from_slice(bytes);
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: callers retain buffers until their submission has completed.
        unsafe {
            self.memory.device.raw.destroy_buffer(self.raw, None);
        }
        if let Some(allocation) = self.allocation.take() {
            self.memory.free(allocation);
        }
    }
}

pub struct UploadBuffer {
    mapped: Buffer,
    submitted: Option<Vec<u32>>,
    label: &'static str,
    pub(super) limit: u64,
}

impl UploadBuffer {
    pub fn new(memory: &Rc<Memory>, label: &'static str, skip_unchanged: bool) -> Result<Self, RenderError> {
        Ok(Self {
            mapped: Buffer::new(memory, 4, STORAGE)?,
            submitted: skip_unchanged.then(Vec::new),
            label,
            limit: u64::from(memory.device.properties.limits.max_storage_buffer_range),
        })
    }

    pub fn upload(&mut self, words: &[u32]) -> Result<(), RenderError> {
        let size = words.len() as u64 * 4;
        let limit = self.limit;
        if size > limit {
            return Err(RenderError::ResourceLimit { label: self.label, required: size, limit });
        }
        let grew = self.mapped.size < size;
        if grew {
            self.mapped = Buffer::new(&self.mapped.memory, size.next_power_of_two().min(limit), STORAGE)?;
        }
        let start = self.submitted.as_mut().map_or(0, |submitted| {
            let start = if grew { 0 } else { common_prefix(submitted, words) };
            submitted.truncate(start);
            submitted.extend_from_slice(&words[start..]);
            start
        });
        self.mapped.write(start * 4, bytemuck::cast_slice(&words[start..]));
        Ok(())
    }

    pub fn address_range(&self) -> vk::DeviceAddressRangeEXT {
        self.mapped.address_range()
    }
}

const STORAGE: vk::BufferUsageFlags = vk::BufferUsageFlags::from_raw(
    vk::BufferUsageFlags::STORAGE_BUFFER.as_raw() | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS.as_raw(),
);

/// Compares whole chunks first when most of the shared buffer is unchanged.
fn common_prefix(a: &[u32], b: &[u32]) -> usize {
    const CHUNK: usize = 256;
    let shared = a.len().min(b.len());
    let mut at = 0;
    while at + CHUNK <= shared && a[at..at + CHUNK] == b[at..at + CHUNK] {
        at += CHUNK;
    }
    at + a[at..shared].iter().zip(&b[at..shared]).take_while(|(x, y)| x == y).count()
}
