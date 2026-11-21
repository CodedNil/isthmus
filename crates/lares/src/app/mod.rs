use crate::{
    app::camera::Camera,
    home,
    presence::Presence,
    render::{
        Globals,
        geometry::acceleration::Scene,
        light::sun::{SunSettings, utc_now},
    },
};
use isthmus::glam::{Vec2, Vec3};
pub use wayland::run;

pub mod camera;
mod runtime;
pub mod views;
mod wayland;

/// How far a moving object is between its authored pose (0) and its moved one (1).
#[derive(Clone, Copy, Default)]
struct Progress {
    amount: f32,
    dwell: f32,
}

pub struct App {
    pub presence: Presence,
    pub scene: Scene,
    pub camera: Camera,
    dragging: Option<(Vec2, Camera)>,
    pointer: Vec2,
    /// One per object in `scene.movers()`.
    motion: Vec<Progress>,
    /// Exposure and white balance, metered once so panning and light changes never shift the display.
    display: (f32, Vec3),
}

impl App {
    pub fn with_baked_lighting(settings: SunSettings, controls: &[u8]) -> Result<Self, &'static str> {
        Ok(Self::from_scene(Scene::from_baked_at(&home::rooms(), utc_now(), settings, controls)?))
    }

    fn from_scene(scene: Scene) -> Self {
        let camera =
            Camera { target: Vec2::ZERO, distance: (scene.radius * 2.0).clamp(6.0, 40.0), ..Camera::default() };
        let (eye, forward, right, up) = camera.rays(1.0);
        let display = scene.camera_settings(eye, forward, right, up);
        let motion = vec![Progress::default(); scene.movers().len()];
        Self { presence: Presence::default(), scene, camera, dragging: None, pointer: Vec2::ZERO, motion, display }
    }

    pub fn pointer_moved(&mut self, position: Vec2, viewport: Vec2) {
        self.pointer = position;
        if let Some((origin, mut camera)) = self.dragging {
            camera.pan(position - origin, viewport);
            self.camera.target = camera.target;
        }
    }

    /// Moves each moving object to its destination while its trigger holds, and back when it stops.
    /// Returns whether any is still moving.
    pub fn advance_motion(&mut self, dt: f32) -> bool {
        let dt = dt.clamp(0.0, 0.1);
        let mut moving = false;
        for (mover, progress) in self.scene.movers().iter().zip(&mut self.motion) {
            let wanted = mover.wanted(&self.presence);
            progress.dwell = if wanted { progress.dwell + dt } else { 0.0 };
            let target = if wanted && progress.dwell >= mover.delay() { 1.0 } else { 0.0 };
            let step = dt / mover.duration();
            progress.amount += (target - progress.amount).clamp(-step, step);
            moving |= (progress.amount - target).abs() > f32::EPSILON;
        }
        for (index, progress) in self.motion.iter().enumerate() {
            let entity_id = self.scene.movers()[index].entity_id;
            self.scene.set_motion(entity_id, progress.amount);
        }
        moving
    }

    pub const fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    pub const fn pointer_pressed(&mut self) {
        self.dragging = Some((self.pointer, self.camera));
    }

    pub const fn pointer_released(&mut self) {
        self.dragging = None;
    }

    pub fn scrolled(&mut self, amount: f32) {
        self.camera.zoom(amount);
        if self.is_dragging() {
            self.pointer_pressed();
        }
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
            normal_samples: 4,
            dynamic: self.scene.dynamic_objects(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::*;

    #[test]
    fn dragging_stays_anchored_across_duplicate_events_and_zoom() {
        let scene = Scene::new_at(&[], 1_782_043_200.0, SunSettings::default());
        let mut app = App::from_scene(scene);
        let viewport = Vec2::new(1600.0, 900.0);
        let origin = Vec2::new(800.0, 450.0);
        let initial = app.camera;
        app.pointer_moved(origin, viewport);
        app.pointer_pressed();
        for step in 0..1000 {
            app.pointer_moved(origin + Vec2::new(step as f32 * 0.13, 17.0), viewport);
        }
        let endpoint = origin + Vec2::new(129.87, 17.0);
        let mut expected = initial;
        expected.pan(endpoint - origin, viewport);
        assert!(app.camera.target.distance(expected.target) < 0.00001);
        let held = app.camera;
        for _ in 0..100 {
            app.pointer_moved(endpoint, viewport);
        }
        assert!(app.camera == held);
        app.scrolled(-1.0);
        let zoomed = app.camera;
        app.pointer_moved(endpoint, viewport);
        assert!(app.camera == zoomed);
        app.pointer_released();
        app.pointer_moved(origin, viewport);
        assert!(app.camera == zoomed);
    }

    #[test]
    fn camera_motion_and_source_controls_cannot_change_the_display_transform() {
        let scene = Scene::new_at(
            &[home::Room::new(Vec2::ZERO, Vec2::splat(3.0), home::Material::Paint).downlights("test", &[(0.0, 0.0)])],
            1_782_043_200.0,
            SunSettings::default(),
        );
        let mut app = App {
            presence: Presence::default(),
            scene,
            camera: Camera::default(),
            dragging: None,
            pointer: Vec2::ZERO,
            motion: Vec::new(),
            display: (1.3, Vec3::ONE),
        };
        for (pan, intensity) in [(8.0, 0.0), (-19.0, 0.75), (4.0, 1.0)] {
            app.camera.pan(Vec2::splat(pan), Vec2::splat(800.0));
            app.camera.zoom(0.1);
            app.scene.set_light_state("test", intensity);
            for size in [Vec2::splat(600.0), Vec2::new(1600.0, 900.0)] {
                let globals = app.globals(size);
                assert_eq!((globals.exposure, globals.white_balance), (1.3, Vec3::ONE));
            }
        }
    }
}
