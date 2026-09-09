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

    pub fn upload<T: ShaderData>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, value: T) {
        self.words.resize(T::WORDS, 0);
        value.write(&mut self.words, 0);
        self.flush(device, queue);
    }

    pub fn upload_if_changed(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, words: &[u32]) {
        if self.words != words {
            self.words.clear();
            self.words.extend_from_slice(words);
            self.flush(device, queue);
        }
    }

    /// Uploads the new suffix of an immutable, append-only resource arena.
    pub fn upload_appended(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, words: &[u32]) {
        assert!(words.len() >= self.uploaded, "persistent resources must be append-only");
        if words.len() == self.uploaded {
            return;
        }
        self.grow(device, words.len());
        queue.write_buffer(&self.buffer, self.uploaded as u64 * 4, bytemuck::cast_slice(&words[self.uploaded..]));
        self.uploaded = words.len();
    }

    pub fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.grow(device, self.words.len());
        let bytes = bytemuck::cast_slice(&self.words);
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
    }

    fn grow(&mut self, device: &wgpu::Device, words: usize) {
        let size = words as u64 * 4;
        if self.buffer.size() < size {
            self.buffer = Self::allocate(device, size.next_power_of_two());
            self.uploaded = 0;
        }
    }
}
