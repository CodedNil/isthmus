use crate::{
    camera::Camera,
    home,
    render::{self, Frame, Globals, Program, Scene},
};
use core::cell::Cell;
use isthmus::{
    Render, SurfaceHandle,
    glam::{Vec2, Vec3},
};

#[derive(Clone, Copy)]
struct Meter {
    rays: [Vec3; 5],
    revision: u64,
    settings: (f32, Vec3),
}

pub struct App {
    pub scene: Scene,
    pub camera: Camera,
    dragging: bool,
    pointer: Vec2,
    meter: Cell<Option<Meter>>,
    exposure: Cell<f32>,
}

impl App {
    pub fn template() -> Self {
        Self::with_sun(render::SunSettings::default())
    }

    pub fn with_sun(settings: render::SunSettings) -> Self {
        let scene = Scene::new_at(&home::rooms(), render::sun::utc_now(), settings);
        let camera =
            Camera { target: Vec2::ZERO, distance: (scene.radius * 2.0).clamp(6.0, 40.0), ..Camera::default() };
        Self {
            scene,
            camera,
            dragging: false,
            pointer: Vec2::ZERO,
            meter: Cell::new(None),
            exposure: Cell::new(render::photometry::EXPOSURE),
        }
    }

    pub fn pointer_moved(&mut self, position: Vec2, viewport: Vec2) {
        let delta = position - self.pointer;
        self.pointer = position;
        if self.dragging {
            self.camera.pan(delta, viewport);
        }
    }

    pub const fn pointer_pressed(&mut self) {
        self.dragging = true;
    }

    pub const fn pointer_released(&mut self) {
        self.dragging = false;
    }

    pub fn scrolled(&mut self, amount: f32) {
        self.camera.zoom(amount);
    }

    pub fn globals(&self, size: Vec2) -> Globals {
        let (eye, forward, right, up) = self.camera.rays(size.x / size.y.max(1.0));
        let pixel_angle = self.camera.fov / size.y.max(1.0);
        let key = [eye, forward, right, up, self.scene.sun.direction];
        let revision = self.scene.lighting_revision;
        let target = if let Some(Meter { rays: previous, revision: old_revision, settings: value }) = self.meter.get() {
            if key == previous && revision == old_revision {
                value
            } else {
                let value = self.scene.camera_settings(eye, forward, right, up);
                self.meter.set(Some(Meter { rays: key, revision, settings: value }));
                value
            }
        } else {
            let value = self.scene.camera_settings(eye, forward, right, up);
            self.meter.set(Some(Meter { rays: key, revision, settings: value }));
            self.exposure.set(value.0);
            value
        };
        // Adapt gradually in stop space; regular metering never invalidates AA.
        let exposure = (self.exposure.get().log2() * 0.85 + target.0.log2() * 0.15).exp2();
        self.exposure.set(exposure);
        Globals {
            eye,
            exposure,
            white_balance: target.1,
            forward,
            right,
            up,
            pixel_angle,
            sun: self.scene.sun,
            sample_index: 0,
            interleaved: false,
            parity: 0,
        }
    }

    pub fn draw(
        &self,
        render: &mut Render<'_, Program>,
        surface: SurfaceHandle,
        size: Vec2,
        shadows: &render::shadows::ShadowAtlas,
        sample: u32,
        interleaved: bool,
    ) {
        let globals = Globals {
            sample_index: if interleaved { sample / 2 } else { sample },
            interleaved,
            parity: sample & 1,
            ..self.globals(size)
        };
        render.surface(surface, size, globals, |mut frame: Frame<'_>| {
            render::draw(&mut frame, &self.scene, shadows);
        });
    }
}
