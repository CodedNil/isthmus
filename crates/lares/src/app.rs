use crate::{
    camera::Camera,
    home,
    render::{self, Frame, Globals, Program, Scene},
};
use isthmus::{
    Render, SurfaceHandle,
    glam::{Vec2, Vec3},
};

pub struct App {
    pub scene: Scene,
    pub camera: Camera,
    dragging: bool,
    pointer: Vec2,
    /// Lock the display transform for the session. Panning and light controls
    /// must not fight an automatic camera meter or mix differently exposed AA samples.
    display: (f32, Vec3),
}

impl App {
    pub fn template() -> Self {
        Self::with_sun(render::SunSettings::default())
    }

    pub fn with_sun(settings: render::SunSettings) -> Self {
        Self::from_scene(Scene::new_at(&home::rooms(), render::sun::utc_now(), settings))
    }

    pub fn with_baked_lighting(settings: render::SunSettings, controls: &[u8]) -> Result<Self, &'static str> {
        Ok(Self::from_scene(Scene::from_baked_at(&home::rooms(), render::sun::utc_now(), settings, controls)?))
    }

    fn from_scene(scene: Scene) -> Self {
        let camera =
            Camera { target: Vec2::ZERO, distance: (scene.radius * 2.0).clamp(6.0, 40.0), ..Camera::default() };
        let (eye, forward, right, up) = camera.rays(1.0);
        let display = scene.camera_settings(eye, forward, right, up);
        Self { scene, camera, dragging: false, pointer: Vec2::ZERO, display }
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
        Globals {
            eye,
            exposure: self.display.0,
            white_balance: self.display.1,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_motion_and_source_controls_cannot_change_the_display_transform() {
        let scene = Scene::new_at(
            &[home::Room::new(Vec2::ZERO, Vec2::splat(3.0), home::Material::Paint).downlights("test", &[(0.0, 0.0)])],
            1_782_043_200.0,
            render::SunSettings::default(),
        );
        let mut app =
            App { scene, camera: Camera::default(), dragging: false, pointer: Vec2::ZERO, display: (1.3, Vec3::ONE) };
        for step in 0..20 {
            app.camera.pan(Vec2::splat(step as f32), Vec2::splat(800.0));
            app.camera.zoom(0.1);
            app.scene.set_light_state("test", step as f32 / 20.0);
            for size in [Vec2::splat(600.0), Vec2::new(1600.0, 900.0)] {
                let globals = app.globals(size);
                assert_eq!((globals.exposure, globals.white_balance), app.display);
            }
        }
    }
}
