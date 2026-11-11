use super::{
    error::{RenderError, SetupError},
    gpu::Gpu,
    setup,
    surface::{SurfaceTarget, Target},
};
use crate::{Frame, Image, Program, Resources, ShaderData, ShaderFrame, SurfaceHandle, glam::Vec2};
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::collections::HashSet;
use web_time::{Duration, Instant};

/// CPU time to build and upload a frame beyond which it is logged. Presenting is excluded because it waits for vsync.
const SLOW_FRAME: Duration = Duration::from_millis(8);

/// GPU resources and presentation surfaces for one shader program.
pub struct Renderer<P: Program> {
    surfaces: SlotMap<SurfaceHandle, SurfaceTarget>,
    pub(super) gpu: Gpu,
    resources: P::Resources,
    started: Instant,
    last_frame: f64,
    order: SmallVec<[SurfaceHandle; 4]>,
}

/// Records surfaces that will be submitted together in one frame.
pub struct Render<'renderer, P: Program> {
    pub(super) renderer: &'renderer mut Renderer<P>,
    /// Seconds elapsed since renderer creation.
    pub time: f32,
    /// Seconds since the previous frame, capped at 0.1.
    pub delta_time: f32,
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
        let Some(target) = renderer.surfaces.get_mut(surface) else { return };
        assert!(!renderer.order.contains(&surface), "record each surface once per render batch");
        renderer.order.push(surface);
        let pixel_scale = screen_size / Vec2::from_array(target.size().map(|v| v as f32));
        let state = ShaderFrame::<P> { screen_size, time: self.time, pixel_scale, globals };
        target.frame = state.append(&mut renderer.gpu.words);
        draw(Frame {
            state,
            delta_time: self.delta_time,
            resources: &mut renderer.resources,
            gpu: &mut renderer.gpu,
            surface: target,
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
        let mut renderer = Self::with_gpu(gpu, resources);
        let handle = renderer.surfaces.insert(target);
        Ok((renderer, handle))
    }

    fn with_gpu(gpu: Gpu, resources: P::Resources) -> Self {
        Self {
            surfaces: SlotMap::with_key(),
            gpu,
            resources,
            started: Instant::now(),
            last_frame: 0.0,
            order: SmallVec::new(),
        }
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
            surface.clear_paints();
        }
        self.resources.begin_frame();
        let elapsed = self.started.elapsed().as_secs_f64();
        let delta = (elapsed - self.last_frame).min(0.1) as f32;
        self.last_frame = elapsed;
        let started = Instant::now();
        draw(&mut Render { renderer: self, time: elapsed as f32, delta_time: delta });
        let built = started.elapsed();
        self.gpu.check()?;
        let uploading = Instant::now();
        self.gpu.prepare(self.resources.data())?;
        self.gpu.check()?;
        let uploaded = uploading.elapsed();
        if built + uploaded > SLOW_FRAME {
            tracing::debug!(?built, ?uploaded, words = self.gpu.words.len(), "Slow frame");
        }
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
        Ok(Self::with_gpu(setup::device::<P>(instance, adapter, format).await?, resources))
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
    pub fn image_view(&self, image: &Image) -> Option<&wgpu::TextureView> {
        self.gpu.images.view(image)
    }

    /// Imports a filterable texture from this renderer's device for shader sampling.
    /// Import external textures again after recovery and replace their image handles.
    pub fn import_texture(&mut self, texture: &wgpu::Texture) -> Image {
        let image = Image { size: [texture.width(), texture.height()], pixels: [].into() };
        self.gpu.images.insert(&image, texture.create_view(&wgpu::TextureViewDescriptor::default()));
        image
    }

    /// Adds a persistent render target that can also be sampled by later draws.
    /// Its contents are preserved between render batches. Release its surface
    /// handle with `remove_surface` when no further updates are needed.
    pub fn add_image(&mut self, size: [u32; 2]) -> (SurfaceHandle, Image) {
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
    pub fn add_image_format(&mut self, size: [u32; 2], format: wgpu::TextureFormat) -> (SurfaceHandle, Image) {
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
