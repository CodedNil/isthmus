use super::{
    error::{RenderError, SetupError},
    gpu::Gpu,
    images::TextureFormat,
    setup::{Device, Vulkan, Window},
    surface::{SurfaceTarget, Target, WindowTarget, surface_format},
};
use crate::{Frame, Image, Program, ShaderData, ShaderFrame, SurfaceHandle, glam::Vec2};
use ash::vk::PhysicalDeviceProperties;
use core::marker::PhantomData;
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

/// CPU time to build and upload a frame beyond which it is logged. Presenting is excluded because it waits for vsync.
const SLOW_FRAME: Duration = Duration::from_millis(8);

/// GPU resources and presentation surfaces for one shader program.
pub struct Renderer<P: Program> {
    surfaces: SlotMap<SurfaceHandle, SurfaceTarget>,
    pub(super) gpu: Gpu,
    program: PhantomData<P>,
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
    /// Records a named render pass targeting a surface, once per batch in logical pixels.
    pub fn surface(
        &mut self,
        name: &'static str,
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
        target.paints.clear();
        target.name = name;
        let pixel_scale = screen_size / Vec2::from_array(target.size().map(|v| v as f32));
        let state = ShaderFrame::<P> { screen_size, time: self.time, pixel_scale, globals };
        target.frame = state.append(&mut renderer.gpu.arena.words);
        draw(Frame { state, delta_time: self.delta_time, gpu: &mut renderer.gpu, surface: target });
    }
}
impl<P: Program> Renderer<P> {
    /// Creates a renderer that owns its presentation target.
    pub fn new(window: impl Window, size: [u32; 2]) -> Result<(Self, SurfaceHandle), SetupError> {
        let vulkan = Vulkan::new(Some(&window))?;
        let surface = vulkan.create_surface(window)?;
        let device = Device::new(Rc::clone(&surface.vulkan), Some(surface.raw), P::max_images())?;
        let (format, native_format) = surface_format(&device, &surface, None)?;
        let gpu = Gpu::new::<P>(device, format)?;
        let mut renderer = Self::with_gpu(gpu);
        let handle = renderer.surfaces.insert(SurfaceTarget::new(Target::Window(WindowTarget::new(
            surface,
            size,
            native_format,
        ))));
        Ok((renderer, handle))
    }

    fn with_gpu(gpu: Gpu) -> Self {
        Self {
            surfaces: SlotMap::with_key(),
            gpu,
            program: PhantomData,
            started: Instant::now(),
            last_frame: 0.0,
            order: SmallVec::new(),
        }
    }

    /// Records and presents one frame.
    pub fn render(&mut self, draw: impl FnOnce(&mut Render<'_, P>)) -> Result<(), RenderError> {
        if let Some(error) = &self.gpu.error {
            return Err(error.clone());
        }
        self.gpu.begin_frame();
        self.order.clear();
        let elapsed = self.started.elapsed().as_secs_f64();
        let delta = (elapsed - self.last_frame).min(0.1) as f32;
        self.last_frame = elapsed;
        let started = Instant::now();
        draw(&mut Render { renderer: self, time: elapsed as f32, delta_time: delta });
        let built = started.elapsed();
        if built > SLOW_FRAME {
            tracing::debug!(?built, words = self.gpu.arena.words.len(), "Slow frame recording");
        }
        let result = self.gpu.submit::<P>(&mut self.surfaces, &self.order);
        if let Err(error @ (RenderError::Gpu(_) | RenderError::DeviceLost(_))) = &result {
            self.gpu.device.report_fault(error);
            self.gpu.error = Some(error.clone());
        }
        result
    }

    /// Recreates the device and keeps offscreen image identities and surface handles.
    pub fn recover(&mut self) -> Result<(), SetupError> {
        let surface = self.surfaces.values().find_map(|target| match &target.target {
            Target::Window(window) => Some(window.surface.raw),
            Target::Image(_) => None,
        });
        let device = Device::new(Rc::clone(&self.gpu.device.vulkan), surface, P::max_images())?;
        let mut gpu = Gpu::new::<P>(device, self.gpu.format)?;
        let mut formats = SmallVec::<[_; 4]>::new();
        for (handle, target) in &self.surfaces {
            match &target.target {
                Target::Window(window) => {
                    let (_, format) = surface_format(&gpu.device, &window.surface, Some(gpu.format))?;
                    formats.push((handle, format));
                }
                Target::Image(image) => gpu.images.register(image, self.gpu.images.target_format(image)),
            }
        }
        for (handle, format) in formats {
            if let Target::Window(window) = &mut self.surfaces[handle].target {
                window.reset(format);
            }
        }
        self.gpu = gpu;
        Ok(())
    }

    /// Creates a renderer without a window for offscreen rendering and GPU tests.
    pub fn headless(format: TextureFormat) -> Result<Self, SetupError> {
        let device = Device::new(Vulkan::new(None)?, None, P::max_images())?;
        Ok(Self::with_gpu(Gpu::new::<P>(device, format)?))
    }

    /// Describes the selected Vulkan device.
    pub fn adapter_info(&self) -> &PhysicalDeviceProperties {
        &self.gpu.device.properties
    }

    pub fn device_name(&self) -> &str {
        self.gpu.device.properties.device_name_as_c_str().ok().and_then(|name| name.to_str().ok()).unwrap_or("Vulkan")
    }

    /// Waits for the most recently submitted frame.
    pub fn wait(&self, timeout: Duration) -> Result<(), RenderError> {
        self.gpu.wait(timeout)
    }

    /// Adds a persistent offscreen target, allocated on its first use.
    pub fn add_image(&mut self, size: [u32; 2]) -> (SurfaceHandle, Image) {
        self.add_image_format(size, self.gpu.format)
    }

    /// Adds another recording target sharing the same offscreen image.
    pub fn alias_image(&mut self, source: SurfaceHandle) -> Option<SurfaceHandle> {
        let Target::Image(image) = &self.surfaces.get(source)?.target else { return None };
        Some(self.surfaces.insert(SurfaceTarget::new(Target::Image(image.clone()))))
    }

    /// Adds a persistent offscreen target with an explicit filterable color format.
    pub fn add_image_format(&mut self, size: [u32; 2], format: TextureFormat) -> (SurfaceHandle, Image) {
        let image = Image { size, pixels: [].into() };
        self.gpu.images.register(&image, format);
        let handle = self.surfaces.insert(SurfaceTarget::new(Target::Image(image.clone())));
        (handle, image)
    }

    pub fn resize(&mut self, surface: SurfaceHandle, size: [u32; 2]) {
        if let Some(SurfaceTarget { target: Target::Window(window), .. }) = self.surfaces.get_mut(surface) {
            window.resize(size);
        }
    }

    pub fn add_surface(&mut self, window: impl Window, size: [u32; 2]) -> Result<SurfaceHandle, SetupError> {
        let surface = self.gpu.device.vulkan.create_surface(window)?;
        let (_, native_format) = surface_format(&self.gpu.device, &surface, Some(self.gpu.format))?;
        Ok(self.surfaces.insert(SurfaceTarget::new(Target::Window(WindowTarget::new(surface, size, native_format)))))
    }

    pub fn remove_surface(&mut self, surface: SurfaceHandle) {
        self.surfaces.remove(surface);
    }
}
