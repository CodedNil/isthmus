use super::{
    gpu::Gpu,
    setup::{self, SetupError},
    surface::{SurfaceTarget, Target},
};
use crate::{Frame, Program, Resources, ShaderData, ShaderFrame, SurfaceHandle, glam::Vec2};
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::collections::HashSet;
use web_time::Instant;

/// GPU resources and presentation surfaces for one shader program.
pub struct Renderer<P: Program> {
    surfaces: SlotMap<SurfaceHandle, SurfaceTarget>,
    gpu: Gpu,
    resources: P::Resources,
    started: Instant,
    last_frame: f64,
    order: SmallVec<[SurfaceHandle; 4]>,
}

/// Records surfaces that will be submitted together in one frame.
pub struct Render<'a, P: Program> {
    renderer: &'a mut Renderer<P>,
    /// Seconds elapsed since renderer creation.
    pub time: f32,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
}
/// A failure while recording or presenting a frame.
#[derive(Clone, Debug, thiserror::Error)]
pub enum RenderError {
    /// A presentation surface was lost and must be recreated.
    #[error("presentation surface was lost")]
    SurfaceLost,
    /// A GPU image belongs to another renderer or needs reimporting after recovery.
    #[error("GPU image is unavailable on this renderer")]
    MissingImage,
    /// GPU validation rejected the frame.
    #[error("frame failed GPU validation")]
    Validation,
    /// The device was lost; recover GPU resources before reuse.
    #[error("GPU device was lost: {0}")]
    DeviceLost(String),
    /// A GPU operation failed; the renderer requires recovery before reuse.
    #[error("GPU operation failed: {0}")]
    Gpu(String),
    /// Refused an upload before allocating or binding an oversized resource.
    #[error("{label} needs {required} bytes, exceeding the device limit of {limit}")]
    ResourceLimit { label: &'static str, required: u64, limit: u64 },
}
impl<P: Program> Render<'_, P> {
    /// Records a surface once per batch in logical pixels; stale handles are ignored.
    pub fn surface(
        &mut self,
        surface: SurfaceHandle,
        screen_size: Vec2,
        globals: P::Globals,
        draw: impl FnOnce(Frame<'_, P>),
    ) {
        if !screen_size.is_finite() || screen_size.min_element() <= 0.0 {
            return;
        }
        let renderer = &mut *self.renderer;
        let handle = surface;
        let Some(surface) = renderer.surfaces.get_mut(surface) else { return };
        assert!(!renderer.order.contains(&handle), "record each surface once per render batch");
        renderer.order.push(handle);
        let pixel_scale = screen_size / Vec2::from_array(surface.size().map(|v| v as f32));
        let state = ShaderFrame::<P> { screen_size, time: self.time, pixel_scale, globals };
        surface.frame = state.append(&mut renderer.gpu.words);
        draw(Frame {
            state,
            delta_time: self.delta_time,
            resources: &mut renderer.resources,
            gpu: &mut renderer.gpu,
            surface,
        });
    }
}
impl<P: Program> Renderer<P> {
    /// Creates a renderer that owns its presentation target.
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: [u32; 2],
        resources: P::Resources,
    ) -> Result<(Self, SurfaceHandle), SetupError> {
        let (gpu, target) = setup::new::<P>(target, size).await?;
        let mut surfaces = SlotMap::with_key();
        let handle = surfaces.insert(target);
        Ok((
            Self { surfaces, gpu, resources, started: Instant::now(), last_frame: 0.0, order: SmallVec::new() },
            handle,
        ))
    }

    /// Records and presents one frame, returning surface loss or GPU validation errors.
    pub fn render(&mut self, draw: impl FnOnce(&mut Render<'_, P>)) -> Result<(), RenderError> {
        let result = self.record(draw);
        if result.is_err() && self.gpu.check().is_ok() {
            self.gpu.queue.submit([]);
        }
        result
    }

    fn record(&mut self, draw: impl FnOnce(&mut Render<'_, P>)) -> Result<(), RenderError> {
        self.gpu.check()?;
        self.gpu.begin_frame();
        self.order.clear();
        for surface in self.surfaces.values_mut() {
            surface.paints.clear();
        }
        self.resources.begin_frame();
        let elapsed = self.started.elapsed().as_secs_f64();
        let delta = (elapsed - self.last_frame).min(0.1) as f32;
        self.last_frame = elapsed;
        draw(&mut Render { renderer: self, time: elapsed as f32, delta_time: delta });
        self.gpu.check()?;
        self.gpu.prepare(self.resources.data())?;
        self.gpu.check()?;
        let mut outputs = SmallVec::<[wgpu::SurfaceTexture; 2]>::new();
        let mut encoder =
            self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("isthmus frame") });
        // Recording order defines dependencies between offscreen passes.
        for handle in &self.order {
            let surface = self.surfaces.get_mut(*handle).expect("recorded surface exists");
            let Some((view, output)) = surface.acquire(&self.gpu)? else { continue };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("isthmus pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if output.is_none() {
                                wgpu::LoadOp::Load
                            } else {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                self.gpu.draw_surface::<P>(&mut pass, surface);
            }
            if let Some(output) = output {
                outputs.push(output);
            }
        }
        // Submit even if every presentation target was occluded: queue uploads
        // retain their staging allocations until a submission consumes them.
        self.gpu.check()?;
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.check()?;
        for output in outputs {
            self.gpu.queue.present(output);
        }
        self.gpu.check()
    }

    /// Recreates device resources while preserving surface handles, clocks and host data.
    /// Offscreen images and aliases keep their identity but must have their contents redrawn.
    /// Resources created directly through `device()` remain the caller's responsibility.
    pub async fn recover(&mut self) -> Result<(), SetupError> {
        let mut gpu = setup::device::<P>(self.gpu.instance.clone(), self.gpu.adapter.clone(), self.gpu.format).await?;
        let mut images = HashSet::new();
        for target in self.surfaces.values_mut() {
            match &target.target {
                Target::Window { surface, config, .. } => surface.configure(&gpu.device, config),
                Target::Image(image) if images.insert(image.pixels.as_ptr() as usize) => {
                    let format = self.gpu.images.view(image).ok_or(RenderError::MissingImage)?.texture().format();
                    let texture = Self::image_texture(&gpu.device, image.size(), format);
                    gpu.images.insert(image, texture.create_view(&wgpu::TextureViewDescriptor::default()));
                }
                Target::Image(_) => {}
            }
        }
        gpu.check()?;
        self.gpu = gpu;
        Ok(())
    }

    /// Creates a renderer without a window for offscreen rendering and GPU tests.
    pub async fn headless(resources: P::Resources, format: wgpu::TextureFormat) -> Result<Self, SetupError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await?;
        let gpu = setup::device::<P>(instance, adapter, format).await?;
        Ok(Self {
            surfaces: SlotMap::with_key(),
            gpu,
            resources,
            started: Instant::now(),
            last_frame: 0.0,
            order: SmallVec::new(),
        })
    }

    /// Describes the selected adapter, including CPU software fallbacks.
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }

    /// Returns the selected GPU adapter's name.
    pub fn device_name(&self) -> &str {
        &self.gpu.device_name
    }

    /// Borrows the device for custom GPU resources and commands.
    pub const fn device(&self) -> &wgpu::Device {
        &self.gpu.device
    }

    /// Borrows the queue for custom uploads and submissions.
    pub const fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }

    /// Returns the format shared by this renderer's presentation surfaces.
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.gpu.format
    }

    /// Returns this renderer's texture view once the image is resident.
    pub fn image_view(&self, image: &crate::Image) -> Option<&wgpu::TextureView> {
        self.gpu.images.view(image)
    }

    /// Imports a filterable texture from this renderer's device for shader sampling.
    /// Import external textures again after recovery and replace their image handles.
    pub fn import_texture(&mut self, texture: &wgpu::Texture) -> crate::Image {
        let image = crate::Image { size: [texture.width(), texture.height()], pixels: [].into() };
        self.gpu.images.insert(&image, texture.create_view(&wgpu::TextureViewDescriptor::default()));
        image
    }

    /// Adds a persistent render target that can also be sampled by later draws.
    /// Its contents are preserved between render batches. Release its surface
    /// handle with `remove_surface` when no further updates are needed.
    pub fn add_image(&mut self, size: [u32; 2]) -> (SurfaceHandle, crate::Image) {
        self.add_image_format(size, self.gpu.format)
    }

    /// Adds another recording target for the same offscreen image, without
    /// allocating another texture. This allows ordered passes to reuse scratch
    /// storage within a batch. Remove both handles when replacing the image.
    /// Returns `None` for stale handles or presentation surfaces.
    pub fn alias_image(&mut self, source: SurfaceHandle) -> Option<SurfaceHandle> {
        let Target::Image(image) = &self.surfaces.get(source)?.target else { return None };
        let target = SurfaceTarget::from_image(image.clone());
        Some(self.surfaces.insert(target))
    }

    /// Adds a persistent offscreen image with an explicit filterable, renderable format.
    /// Its draw shader must declare the matching `.format(...)`.
    pub fn add_image_format(&mut self, size: [u32; 2], format: wgpu::TextureFormat) -> (SurfaceHandle, crate::Image) {
        let texture = Self::image_texture(&self.gpu.device, size, format);
        let image = self.import_texture(&texture);
        let target = SurfaceTarget::from_image(image.clone());
        (self.surfaces.insert(target), image)
    }

    fn image_texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("isthmus render image"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Updates physical surface dimensions, ignoring zero dimensions and stale handles.
    pub fn resize(&mut self, surface: SurfaceHandle, [width, height]: [u32; 2]) {
        if let Some(slot) = self.surfaces.get_mut(surface) {
            slot.resize(&self.gpu, width, height);
        }
    }

    /// Adds an owned presentation target.
    pub fn add_surface(
        &mut self,
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: [u32; 2],
    ) -> Result<SurfaceHandle, SetupError> {
        let surface = self.gpu.instance.create_surface(target)?;
        let config = setup::configure_surface(&self.gpu.adapter, &surface, size, Some(self.gpu.format))?;
        Ok(self.surfaces.insert(SurfaceTarget::from_raw(&self.gpu.device, surface, config)))
    }

    /// Releases a surface and invalidates its handle; stale handles are ignored.
    pub fn remove_surface(&mut self, surface: SurfaceHandle) {
        self.surfaces.remove(surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Blend, backend::buffer::UploadBuffer, program::ShaderEntry};
    use std::{sync::mpsc, time::Duration};

    const IMAGE_VERTEX: &str = "
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, };
@vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let points = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    );
    let point = points[index];
    return VertexOutput(vec4(point, 0.0, 1.0), point * 0.5 + 0.5);
}
";
    const IMAGE_FRAGMENT: &str = "
@group(1) @binding(0) var image_sampler: sampler;
@group(1) @binding(1) var image_texture: texture_2d<f32>;
@fragment fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(image_texture, image_sampler, uv);
}
";

    #[derive(Clone, Copy)]
    struct ImageProgram;

    impl Program for ImageProgram {
        type Globals = ();
        type Resources = ();

        const SHADERS: &'static [ShaderEntry] = &[ShaderEntry {
            name: "image",
            blend: Blend::Replace,
            images: 1,
            format: None,
            code: [IMAGE_VERTEX, IMAGE_FRAGMENT],
        }];
    }

    #[derive(Clone, Copy)]
    struct EmptyProgram;

    impl Program for EmptyProgram {
        type Globals = ();
        type Resources = ();

        const SHADERS: &'static [ShaderEntry] = &[];
    }

    fn headless() -> Renderer<EmptyProgram> {
        pollster::block_on(Renderer::headless((), wgpu::TextureFormat::Rgba8Unorm)).unwrap()
    }

    #[test]
    fn frames_without_a_presentation_texture_still_submit_uploads() {
        let mut renderer = headless();
        let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluded upload regression"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        renderer.queue().write_buffer(&buffer, 0, &[7; 4]);
        renderer.render(|_| {}).unwrap();
        let (send, recv) = mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
        renderer.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
        recv.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(&*buffer.slice(..).get_mapped_range().unwrap(), &[7; 4]);
    }

    #[test]
    fn storage_growth_stops_at_the_binding_limit_without_poisoning_the_device() {
        let renderer = headless();
        let (device, queue) = pollster::block_on(renderer.gpu.adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits { max_storage_buffer_binding_size: 96, ..Default::default() },
            ..Default::default()
        }))
        .unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut buffer = UploadBuffer::new(&device, "bounded storage regression");
        buffer.upload(&device, &queue, &[1; 23]).unwrap();
        assert_eq!(buffer.buffer.size(), 96);
        assert!(matches!(buffer.upload(&device, &queue, &[2; 25]), Err(RenderError::ResourceLimit { .. })));
        buffer.upload(&device, &queue, &[3]).unwrap();
        buffer.upload(&device, &queue, &[4; 23]).unwrap();
        queue.submit([]);
        assert!(pollster::block_on(scope.pop()).is_none());
    }

    #[test]
    fn invalid_buffer_preserves_original_error_and_stops_rendering() {
        let mut renderer = headless();
        let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("invalid storage regression"),
            size: renderer.device().limits().max_buffer_size + 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Reproduce the secondary error in the crash report without panicking.
        renderer.queue().write_buffer(&buffer, 0, &[0; 4]);
        for _ in 0..2 {
            let error = renderer.render(|_| panic!("failed renderer must not record another frame")).unwrap_err();
            let RenderError::Gpu(message) = error else { panic!("expected GPU validation diagnostics") };
            assert!(message.contains("Device::create_buffer"), "original allocation error was lost: {message}");
            assert!(message.contains("invalid storage regression"), "buffer label was lost: {message}");
        }
    }

    #[test]
    fn device_loss_returns_error_instead_of_recording() {
        let mut renderer = headless();
        let error = renderer
            .render(|render| {
                render.renderer.device().destroy();
                render.renderer.device().poll(wgpu::PollType::Poll).unwrap();
            })
            .unwrap_err();
        assert!(matches!(error, RenderError::DeviceLost(_)), "expected device loss during frame: {error}");
        let error = renderer.render(|_| panic!("lost device must not record another frame")).unwrap_err();
        assert!(matches!(error, RenderError::DeviceLost(_)), "expected device loss: {error}");
    }

    #[test]
    fn foreign_images_fail_without_poisoning_later_passes_or_recovery() {
        let mut owner =
            pollster::block_on(Renderer::<ImageProgram>::headless((), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
        let (_, foreign) = owner.add_image([1, 1]);
        let mut renderer =
            pollster::block_on(Renderer::<ImageProgram>::headless((), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
        let (first, first_image) = renderer.add_image([1, 1]);
        let (second, second_image) = renderer.add_image([1, 1]);
        let source = crate::Image::rgba8([1, 1], vec![255, 128, 0, 255]);
        let error = renderer
            .render(|render| {
                render.surface(first, Vec2::ONE, (), |mut frame| {
                    frame.record(0, 6, (), &[&foreign]);
                });
            })
            .unwrap_err();
        assert!(matches!(error, RenderError::MissingImage));
        renderer
            .render(|render| {
                render.surface(first, Vec2::ONE, (), |mut frame| {
                    frame.record(0, 6, (), &[&source]);
                });
                render.surface(second, Vec2::ONE, (), |mut frame| {
                    frame.record(0, 6, (), &[&first_image]);
                });
            })
            .unwrap();
        assert_eq!(pixel(&renderer, &first_image), [255, 128, 0, 255]);
        assert_eq!(pixel(&renderer, &second_image), [255, 128, 0, 255]);
        renderer.device().destroy();
        renderer.device().poll(wgpu::PollType::Poll).unwrap();
        pollster::block_on(renderer.recover()).unwrap();
        renderer
            .render(|render| {
                render.surface(first, Vec2::ONE, (), |mut frame| {
                    frame.record(0, 6, (), &[&source]);
                });
                render.surface(second, Vec2::ONE, (), |mut frame| {
                    frame.record(0, 6, (), &[&first_image]);
                });
            })
            .unwrap();
        assert_eq!(pixel(&renderer, &first_image), [255, 128, 0, 255]);
        assert_eq!(pixel(&renderer, &second_image), [255, 128, 0, 255]);
    }

    fn pixel(renderer: &Renderer<ImageProgram>, image: &crate::Image) -> [u8; 4] {
        let texture = renderer.image_view(image).unwrap().texture();
        let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("image pass regression"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.device().create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None },
            },
            texture.size(),
        );
        renderer.queue().submit([encoder.finish()]);
        let (send, receive) = mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| send.send(result).unwrap());
        renderer.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        mapped[..4].try_into().unwrap()
    }
}
