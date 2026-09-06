use crate::ShaderData;

pub struct UploadBuffer {
    pub buffer: wgpu::Buffer,
    pub words: Vec<u32>,
    uploaded: usize,
}

impl UploadBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self { buffer: Self::allocate(device, 4), words: Vec::new(), uploaded: 0 }
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
        self.words.resize(values.len() * T::WORDS, 0);
        for (index, value) in values.iter().enumerate() {
            value.write(&mut self.words, index * T::WORDS);
        }
        self.flush(device, queue);
    }

    pub fn upload_if_changed<T: ShaderData + PartialEq>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        values: &[T],
    ) {
        if self.words.len() != values.len() * T::WORDS
            || values.iter().enumerate().any(|(index, value)| T::read(&self.words, index * T::WORDS) != *value)
        {
            self.upload(device, queue, values);
        }
    }

    pub fn push<T: ShaderData>(&mut self, value: T) {
        let offset = self.words.len();
        self.words.resize(offset + T::WORDS, 0);
        value.write(&mut self.words, offset);
    }

    /// Uploads the new suffix of an immutable, append-only resource arena.
    pub fn upload_appended(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, words: &[u32]) {
        if words.len() == self.uploaded {
            return;
        }
        let size = words.len() as u64 * 4;
        if self.buffer.size() < size {
            self.buffer = Self::allocate(device, size.next_power_of_two());
            self.uploaded = 0;
        }
        queue.write_buffer(&self.buffer, self.uploaded as u64 * 4, bytemuck::cast_slice(&words[self.uploaded..]));
        self.uploaded = words.len();
    }

    pub fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let bytes = bytemuck::cast_slice(&self.words);
        if self.buffer.size() < bytes.len() as u64 {
            self.buffer = Self::allocate(device, (bytes.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
    }
}
