//! Vulkan command recording and the resources kept alive by each submission.
use super::{
    arena::Arena,
    buffer::{Memory, UploadBuffer},
    error::RenderError,
    heap::FrameHeap,
    images::{self, Images, SAMPLED_LAYOUT, Texture, TextureFormat, Upload},
    pipelines::Pipelines,
    setup::Device,
    surface::{Acquired, FRAMES, Paint, SurfaceHandle, SurfaceTarget, Target},
    timings::{Queries, Timings},
};
use crate::{Image, Program, ShaderData};
use ash::vk;
use isthmus_abi as abi;
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::{collections::HashMap, mem, rc::Rc, sync::Arc, time::Duration};

struct Submission {
    device: Rc<Device>,
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    buffers: [UploadBuffer; abi::BUFFER_COUNT],
    uploads: Vec<Upload>,
    textures: Vec<Rc<Texture>>,
    heap: FrameHeap,
    queries: Option<Queries>,
}

impl Submission {
    fn new(memory: &Rc<Memory>) -> Result<Self, RenderError> {
        let device = &memory.device;
        let heap = FrameHeap::new(memory)?;
        let buffers =
            [UploadBuffer::new(memory, "isthmus frame", false)?, UploadBuffer::new(memory, "isthmus shared", true)?];
        // SAFETY: this queue family was selected when creating the device.
        let pool = unsafe {
            device.raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT)
                    .queue_family_index(device.family),
                None,
            )
        }?;
        let mut frame = Self {
            device: Rc::clone(device),
            pool,
            cmd: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            buffers,
            uploads: Vec::new(),
            textures: Vec::new(),
            heap,
            queries: None,
        };
        // SAFETY: the newly created pool is owned by frame, as are its command buffer and fence.
        frame.cmd = unsafe {
            device.raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default().command_pool(pool).command_buffer_count(1),
            )
        }?[0];
        // SAFETY: the fence is created on the frame's live device.
        frame.fence = unsafe {
            device.raw.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)
        }?;
        Ok(frame)
    }

    fn wait(&self, timeout: u64) -> Result<(), RenderError> {
        // SAFETY: this is a live fence owned by the frame slot.
        Ok(unsafe { self.device.raw.wait_for_fences(&[self.fence], true, timeout) }?)
    }

    fn begin(&mut self) -> Result<(), RenderError> {
        self.wait(u64::MAX)?;
        self.textures.clear();
        self.uploads.clear();
        // SAFETY: the fence has completed and no command buffers in this pool are pending.
        unsafe { self.device.raw.reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty()) }?;
        // SAFETY: the reset command buffer belongs to the frame and is ready to record.
        unsafe {
            self.device.raw.begin_command_buffer(
                self.cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }?;
        Ok(())
    }
}

impl Drop for Submission {
    fn drop(&mut self) {
        // SAFETY: dropping Gpu waits for submissions before dropping the frame slots.
        unsafe {
            self.device.raw.destroy_fence(self.fence, None);
        }
        // SAFETY: the queue is idle and all command buffers from the pool have completed.
        unsafe {
            self.device.raw.destroy_command_pool(self.pool, None);
        }
    }
}

pub struct Gpu {
    pub device: Rc<Device>,
    pub format: TextureFormat,
    memory: Rc<Memory>,
    pipelines: Pipelines,
    frames: [Submission; FRAMES],
    cursor: usize,
    timings: Option<Timings>,
    pending: Vec<Upload>,
    slots: HashMap<vk::Image, u32>,
    sampled: Vec<Rc<Texture>>,
    pub images: Images,
    pub arena: Arena,
    pub(super) scratch: Vec<u32>,
    record_error: Option<RenderError>,
    pub error: Option<RenderError>,
}

impl Gpu {
    pub fn new<P: Program>(device: Rc<Device>, format: TextureFormat) -> Result<Self, RenderError> {
        let memory = Memory::new(Rc::clone(&device))?;
        let pipelines = Pipelines::new::<P>(&device, format)?;
        let frames = [Submission::new(&memory)?, Submission::new(&memory)?];
        Ok(Self {
            timings: Timings::new(&device),
            device,
            format,
            memory,
            pipelines,
            frames,
            cursor: 0,
            pending: Vec::new(),
            slots: HashMap::new(),
            sampled: Vec::new(),
            images: Images::default(),
            arena: Arena::default(),
            scratch: Vec::new(),
            record_error: None,
            error: None,
        })
    }

    pub fn begin_frame(&mut self) {
        self.arena.begin_frame();
        self.pending.clear();
        self.images.retain_live();
        self.slots.clear();
        self.sampled.clear();
    }

    pub(crate) fn fail(&mut self, error: RenderError) {
        self.record_error.get_or_insert(error);
    }

    pub(crate) fn emit(&mut self, surface: &mut SurfaceTarget, shader: usize, payload: &[u32], images: &[&Image]) {
        let mut indices = SmallVec::<[u32; 4]>::new();
        for &image in images {
            if let Target::Image(target) = &surface.target
                && Arc::ptr_eq(&image.pixels, &target.pixels)
            {
                self.fail(RenderError::Gpu("a pass samples its own color attachment".into()));
                return;
            }
            match self.image_slot(image) {
                Ok(index) => indices.push(index),
                Err(error) => {
                    self.fail(error);
                    return;
                }
            }
        }
        let stride = payload.len() + indices.len() + 1;
        let offset = self.arena.words.len().next_multiple_of(stride);
        self.arena.words.resize(offset, 0);
        surface.frame.append(&mut self.arena.words);
        self.arena.words.extend_from_slice(payload);
        self.arena.words.extend_from_slice(&indices);
        let draw = (offset / stride) as u32;
        if let Some(previous) = surface.paints.last_mut()
            && previous.shader == shader
            && previous.draws.end == draw
        {
            previous.draws.end += 1;
        } else {
            surface.paints.push(Paint { shader, draws: draw..draw + 1 });
        }
    }

    fn image_slot(&mut self, image: &Image) -> Result<u32, RenderError> {
        let texture = self.images.resolve(&self.memory, image, &mut self.pending)?;
        if let Some(&index) = self.slots.get(&texture.raw) {
            return Ok(index);
        }
        let index = self.sampled.len() as u32;
        let limit = self.device.max_images;
        if index >= limit {
            return Err(RenderError::ImageLimit(limit));
        }
        self.slots.insert(texture.raw, index);
        self.sampled.push(texture);
        Ok(index)
    }

    pub fn wait(&self, timeout: Duration) -> Result<(), RenderError> {
        let latest = (self.cursor + FRAMES - 1) % FRAMES;
        self.frames[latest]
            .wait(u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX))
            .inspect_err(|error| self.device.report_fault(error))
    }

    pub fn submit<P: Program>(
        &mut self,
        surfaces: &mut SlotMap<SurfaceHandle, SurfaceTarget>,
        order: &[SurfaceHandle],
    ) -> Result<(), RenderError> {
        if let Some(error) = self.record_error.take() {
            return Err(error);
        }
        let mut targets = SmallVec::<[Option<Rc<Texture>>; 4]>::new();
        for &handle in order {
            let surface = &mut surfaces[handle];
            let (texture, format) = match &mut surface.target {
                Target::Window(window) => {
                    window.prepare(&self.device)?;
                    (None, self.format)
                }
                Target::Image(image) => (
                    Some(self.images.resolve(&self.memory, image, &mut self.pending)?),
                    self.images.target_format(image),
                ),
            };
            for paint in &surface.paints {
                let entry = &P::SHADERS[paint.shader];
                if entry.format.unwrap_or(self.format) != format {
                    return Err(RenderError::Gpu(format!("incompatible color attachment for {:?}", entry.name)));
                }
            }
            targets.push(texture);
        }

        let frame = &mut self.frames[self.cursor];
        frame.begin()?;
        let mut shader_timings = false;
        let timed = if let Some(timings) = &mut self.timings {
            if let Some(queries) = &mut frame.queries {
                timings.collect(queries);
            }
            shader_timings = timings.shaders;
            let mut count = order.len();
            if shader_timings {
                count += order
                    .iter()
                    .map(|&handle| surfaces[handle].paints.chunk_by(|a, b| a.shader == b.shader).count())
                    .sum::<usize>();
            }
            Queries::prepare(&mut frame.queries, &self.device, frame.cmd, count)
        } else {
            false
        };
        frame.buffers[abi::FRAME_BINDING as usize].upload(&self.arena.words)?;
        frame.buffers[abi::SHARED_BINDING as usize].upload(&self.arena.shared)?;
        frame.heap.bind(&self.device, frame.cmd, &frame.buffers, &mut self.sampled)?;
        frame.uploads = mem::take(&mut self.pending);
        for upload in &frame.uploads {
            upload.record(&self.device.raw, frame.cmd);
        }

        let mut outputs = SmallVec::<[(SurfaceHandle, Acquired); 4]>::new();
        let mut error = None;
        for (&handle, texture) in order.iter().zip(targets) {
            let surface = &mut surfaces[handle];
            let output = match &mut surface.target {
                Target::Window(window) => match window.acquire(&self.device, self.cursor) {
                    Ok(Some(output)) => Some(output),
                    Ok(None) => continue,
                    Err(cause) => {
                        error.get_or_insert(cause);
                        continue;
                    }
                },
                Target::Image(_) => None,
            };
            let (image, view, size, previous, next) = output.as_ref().map_or_else(
                || {
                    let texture = texture.as_ref().unwrap();
                    (texture.raw, texture.view, texture.size, SAMPLED_LAYOUT, SAMPLED_LAYOUT)
                },
                |output| (output.image, output.view, output.size, output.previous, vk::ImageLayout::PRESENT_SRC_KHR),
            );
            let labels = self.device.label(frame.cmd, surface.name);
            let mut queries = timed.then(|| frame.queries.as_mut().unwrap());
            let start = queries.as_mut().map(|queries| queries.start(frame.cmd, (surface.name, None)));
            images::transition(&self.device.raw, frame.cmd, image, previous, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            draw::<P>(
                &self.device,
                &self.pipelines,
                frame.cmd,
                surface,
                view,
                size,
                output.is_some(),
                queries.as_deref_mut().filter(|_| shader_timings),
            );
            images::transition(&self.device.raw, frame.cmd, image, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, next);
            if let Some((queries, start)) = queries.zip(start) {
                queries.end(frame.cmd, start);
            }
            if labels {
                self.device.end_label(frame.cmd);
            }
            if let Some(texture) = texture {
                frame.textures.push(texture);
            }
            if let Some(output) = output {
                outputs.push((handle, output));
            }
        }

        let device = &self.device.raw;
        let waits = outputs
            .iter()
            .map(|(_, out)| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(out.acquire)
                    .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            })
            .collect::<SmallVec<[_; 4]>>();
        let signals = outputs
            .iter()
            .map(|(_, out)| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(out.present)
                    .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            })
            .collect::<SmallVec<[_; 4]>>();
        let commands = [vk::CommandBufferSubmitInfo::default().command_buffer(frame.cmd)];
        let submit = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .signal_semaphore_infos(&signals)
            .command_buffer_infos(&commands);
        // SAFETY: this frame's fence was waited. The frame retains every allocation referenced by its commands.
        unsafe { device.end_command_buffer(frame.cmd) }?;
        // SAFETY: frame.begin waited for the previous use of this fence.
        unsafe { device.reset_fences(&[frame.fence]) }?;
        // SAFETY: all resources needed by the recorded submission are retained by the frame and the surfaces.
        unsafe { device.queue_submit2(self.device.queue, &[submit], frame.fence) }?;
        if timed {
            frame.queries.as_mut().unwrap().finish();
        }
        self.cursor = (self.cursor + 1) % FRAMES;
        for (handle, out) in outputs {
            if let Target::Window(window) = &mut surfaces[handle].target
                && let Err(cause) = window.present(out.index)
            {
                error.get_or_insert(cause);
            }
        }
        error.map_or(Ok(()), Err)
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        if self.device.idle().is_ok()
            && let Some(timings) = &mut self.timings
        {
            let (newer, older) = self.frames.split_at_mut(self.cursor);
            for frame in older.iter_mut().chain(newer) {
                if let Some(queries) = &mut frame.queries {
                    timings.collect(queries);
                }
            }
            timings.report();
        }
    }
}

fn draw<P: Program>(
    device: &Device,
    pipelines: &Pipelines,
    cmd: vk::CommandBuffer,
    surface: &SurfaceTarget,
    view: vk::ImageView,
    size: vk::Extent2D,
    clear: bool,
    mut queries: Option<&mut Queries>,
) {
    let raw = &device.raw;
    let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: size };
    let viewport = vk::Viewport {
        x: 0.0,
        y: size.height as f32,
        width: size.width as f32,
        height: -(size.height as f32),
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let attachments = [vk::RenderingAttachmentInfo::default()
        .image_view(view)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(if clear { vk::AttachmentLoadOp::CLEAR } else { vk::AttachmentLoadOp::LOAD })
        .store_op(vk::AttachmentStoreOp::STORE)
        .clear_value(vk::ClearValue { color: vk::ClearColorValue { float32: [0.0; 4] } })];
    let rendering = vk::RenderingInfo::default().render_area(area).layer_count(1).color_attachments(&attachments);
    // SAFETY: the attachment was transitioned and is retained until submission completes.
    unsafe {
        raw.cmd_begin_rendering(cmd, &rendering);
    }
    // SAFETY: the command buffer is recording and has one dynamic viewport.
    unsafe {
        raw.cmd_set_viewport(cmd, 0, &[viewport]);
    }
    // SAFETY: the command buffer is recording and has one dynamic scissor.
    unsafe {
        raw.cmd_set_scissor(cmd, 0, &[area]);
    }
    for group in surface.paints.chunk_by(|a, b| a.shader == b.shader) {
        let shader = group[0].shader;
        let name = P::SHADERS[shader].name;
        let label = device.label(cmd, name);
        let start = queries.as_mut().map(|queries| queries.start(cmd, (surface.name, Some(name))));
        // SAFETY: the pipeline format and descriptor mappings match the active pass.
        unsafe {
            raw.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipelines.shaders[shader]);
        }
        for paint in group {
            // SAFETY: the pipeline and retained descriptors cover these recorded instances.
            unsafe {
                raw.cmd_draw(cmd, 4, paint.draws.end - paint.draws.start, 0, paint.draws.start);
            }
        }
        if let Some((queries, start)) = queries.as_ref().zip(start) {
            queries.end(cmd, start);
        }
        if label {
            device.end_label(cmd);
        }
    }
    // SAFETY: the rendering instance was begun above on this command buffer.
    unsafe {
        raw.cmd_end_rendering(cmd);
    }
}
