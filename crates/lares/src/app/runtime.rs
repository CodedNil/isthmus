//! The scene and its paced GPU render loop.
use super::{App, camera::Camera, wayland::NativeWindow};
use crate::{
    irradiance::bake::{baked_dir, ensure_assets},
    presence::{self, Snapshot},
    render::{
        Renderer,
        light::{
            shadows::ShadowAtlas,
            sun::{SunSettings, utc_now},
        },
        pipeline::{targets::Targets, temporal::Accumulation},
    },
};
use isthmus::{
    RenderError, SurfaceHandle,
    wgpu::{DeviceType, PollType, SurfaceTarget},
};
use std::{
    error::Error,
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;
use tracing::info;

pub(super) struct Runtime {
    pub app: App,
    pub next_frame: Instant,
    gpu: Renderer,
    surface: SurfaceHandle,
    targets: Targets,
    shadows: ShadowAtlas,
    presence: watch::Receiver<Snapshot>,
    accumulation: Accumulation,
    last_rendered: Option<(Camera, u64)>,
    last_frame: Instant,
    next_scene_refresh: Instant,
    first_frame: bool,
}

impl Runtime {
    pub(super) fn new(window: impl Into<SurfaceTarget<'static>>, size: [u32; 2]) -> Result<Self, Box<dyn Error>> {
        let (mut gpu, surface) = pollster::block_on(Renderer::new(window, size))?;
        let adapter = gpu.adapter_info();
        if adapter.device_type == DeviceType::Cpu {
            return Err("Lares needs a hardware Vulkan adapter".into());
        }
        info!(adapter = %adapter.name, "Using Vulkan");
        info!("Preparing lighting and scene");
        ensure_assets()?;
        let baked = baked_dir();
        let app = App::with_baked_lighting(SunSettings::default(), &fs::read(baked.join("lighting.bin"))?)?;
        let shadows = ShadowAtlas::decode(&app.scene, &fs::read(baked.join("visibility.bin"))?)?;
        let targets = Targets::new(&mut gpu, size);
        let now = Instant::now();
        Ok(Self {
            app,
            next_frame: now,
            gpu,
            surface,
            targets,
            shadows,
            presence: presence::subscribe(),
            accumulation: Accumulation::new(Targets::interleaved(size)),
            last_rendered: None,
            last_frame: now,
            next_scene_refresh: now,
            first_frame: true,
        })
    }

    /// Returns whether the frame finished without a GPU recovery.
    pub(super) fn draw(&mut self, window: &Arc<NativeWindow>, size: [u32; 2]) -> Result<bool, Box<dyn Error>> {
        let started = Instant::now();
        let elapsed = started.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = started;
        if self.presence.has_changed()? {
            self.app.presence.ingest(self.presence.borrow_and_update().clone());
        }
        let presence_moving = self.app.presence.advance();
        let moving = self.app.advance_motion(elapsed) || presence_moving;
        self.app.scene.update_time(utc_now());

        let stamp = (self.app.camera, self.app.scene.lighting_revision);
        let changed = if size == self.targets.size() {
            self.last_rendered != Some(stamp)
        } else {
            self.gpu.resize(self.surface, size);
            self.targets.resize(&mut self.gpu, size);
            self.accumulation = Accumulation::new(Targets::interleaved(size));
            true
        };
        let scene_active = changed || !self.accumulation.converged();
        let active = self.app.is_dragging() || scene_active || moving;
        let redraw = scene_active || started >= self.next_scene_refresh;
        let (phase, mask) =
            if redraw { self.accumulation.advance(changed) } else { (0, self.accumulation.valid_mask()) };
        let result = self.targets.record(&mut self.gpu, &self.app, &self.shadows, self.surface, phase, mask, redraw);
        match result {
            Ok(()) => {}
            Err(RenderError::DeviceLost(_) | RenderError::Gpu(_) | RenderError::SurfaceLost) => {
                if matches!(result, Err(RenderError::SurfaceLost)) {
                    self.gpu.remove_surface(self.surface);
                    self.surface = self.gpu.add_surface(Arc::clone(window), size)?;
                } else {
                    pollster::block_on(self.gpu.recover())?;
                }
                self.last_rendered = None;
                self.next_frame = started;
                self.next_scene_refresh = started;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        }
        // Keep one ray-traced frame in flight, including while a drag is active.
        self.gpu.device().poll(PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) })?;
        self.last_rendered = Some(stamp);
        if self.first_frame {
            info!("Lares ready");
            self.first_frame = false;
        }
        if redraw {
            self.next_scene_refresh = started + Duration::from_millis(125);
        }
        let completed = Instant::now();
        self.next_frame = if active {
            started + Duration::from_millis(16)
        } else {
            (started + Duration::from_millis(125)).max(completed + (completed - started) * 4)
        };
        Ok(true)
    }
}
