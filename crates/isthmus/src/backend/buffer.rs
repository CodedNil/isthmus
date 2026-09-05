use crate::ShaderData;

pub struct UploadBuffer(pub wgpu::Buffer);

impl UploadBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self(Self::allocate(device, 4))
    }

    fn allocate(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("isthmus storage"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn upload<T: ShaderData>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, values: &[T]) {
        self.bytes(device, queue, bytemuck::cast_slice(values));
    }

    pub fn bytes(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if self.0.size() < bytes.len() as u64 {
            self.0 = Self::allocate(device, (bytes.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.0, 0, bytes);
        }
    }
}
