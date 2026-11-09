use super::renderer::RenderError;

/// One bounded storage allocation, uploading only the suffix after the first changed word.
pub struct UploadBuffer {
    pub buffer: wgpu::Buffer,
    submitted: Vec<u32>,
    label: &'static str,
}

impl UploadBuffer {
    pub fn new(device: &wgpu::Device, label: &'static str) -> Self {
        Self { buffer: Self::allocate(device, 4, label), submitted: Vec::new(), label }
    }

    fn allocate(device: &wgpu::Device, size: u64, label: &'static str) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, words: &[u32]) -> Result<(), RenderError> {
        let size = words.len() as u64 * 4;
        let limits = device.limits();
        let limit = limits.max_buffer_size.min(limits.max_storage_buffer_binding_size);
        if size > limit {
            return Err(RenderError::ResourceLimit { label: self.label, required: size, limit });
        }
        let grew = self.buffer.size() < size;
        if !grew && self.submitted == words {
            return Ok(());
        }
        if grew {
            let capacity = size.next_power_of_two().min(limit);
            // A bigger buffer is a new allocation and a new bind group, which can hitch the frame that needs it.
            tracing::debug!(buffer = self.label, bytes = capacity, "Grew storage buffer");
            self.buffer = Self::allocate(device, capacity, self.label);
        }
        let start = if grew { 0 } else { self.submitted.iter().zip(words).take_while(|(a, b)| a == b).count() };
        if start < words.len() {
            queue.write_buffer(&self.buffer, start as u64 * 4, bytemuck::cast_slice(&words[start..]));
        }
        self.submitted.truncate(start);
        self.submitted.extend_from_slice(&words[start..]);
        Ok(())
    }
}
