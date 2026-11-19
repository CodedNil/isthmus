use super::error::RenderError;

/// One bounded storage allocation, written fresh each frame or, for words that rarely change, only from the first
/// word that differs from the last frame's.
pub struct UploadBuffer {
    pub buffer: wgpu::Buffer,
    /// What the buffer holds, kept only when uploads skip what is unchanged.
    submitted: Option<Vec<u32>>,
    label: &'static str,
    /// The most bytes the device allows one storage binding.
    limit: u64,
}

impl UploadBuffer {
    pub fn new(device: &wgpu::Device, label: &'static str, skip_unchanged: bool) -> Self {
        let limits = device.limits();
        let limit = limits.max_buffer_size.min(limits.max_storage_buffer_binding_size);
        Self { buffer: Self::allocate(device, 4, label), submitted: skip_unchanged.then(Vec::new), label, limit }
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
        if size > self.limit {
            return Err(RenderError::ResourceLimit { label: self.label, required: size, limit: self.limit });
        }
        let grew = self.buffer.size() < size;
        if grew {
            let capacity = size.next_power_of_two().min(self.limit);
            // A bigger buffer is a new allocation and a new bind group, which can hitch the frame that needs it.
            tracing::debug!(buffer = self.label, bytes = capacity, "Grew storage buffer");
            self.buffer = Self::allocate(device, capacity, self.label);
        }
        let start = self.submitted.as_mut().map_or(0, |submitted| {
            let start = if grew { 0 } else { common_prefix(submitted, words) };
            submitted.truncate(start);
            submitted.extend_from_slice(&words[start..]);
            start
        });
        if start < words.len() {
            queue.write_buffer(&self.buffer, start as u64 * 4, bytemuck::cast_slice(&words[start..]));
        }
        Ok(())
    }
}

/// How many words two slices start with in common. Whole chunks are compared as slices, which is far quicker than
/// word by word when, as is usual, almost nothing has changed.
fn common_prefix(a: &[u32], b: &[u32]) -> usize {
    const CHUNK: usize = 256;
    let shared = a.len().min(b.len());
    let mut at = 0;
    while at + CHUNK <= shared && a[at..at + CHUNK] == b[at..at + CHUNK] {
        at += CHUNK;
    }
    at + a[at..shared].iter().zip(&b[at..shared]).take_while(|(x, y)| x == y).count()
}
