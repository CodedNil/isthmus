use crate::{
    camera::Camera,
    home,
    render::{self, Frame, Globals, Program, Scene},
};
use isthmus::{Render, SurfaceHandle, glam::Vec2};

pub struct App {
    pub scene: Scene,
    pub camera: Camera,
    dragging: bool,
    pointer: Vec2,
}

impl App {
    pub fn template() -> Self {
        let scene = Scene::new(&home::rooms());
        let camera =
            Camera { target: Vec2::ZERO, distance: (scene.radius * 2.0).clamp(6.0, 40.0), ..Camera::default() };
        Self { scene, camera, dragging: false, pointer: Vec2::ZERO }
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

    pub fn draw(&self, render: &mut Render<'_, Program>, surface: SurfaceHandle, size: Vec2) {
        let (eye, forward, right, up) = self.camera.rays(size.x / size.y.max(1.0));
        let pixel_angle = self.camera.fov / size.y.max(1.0);
        let globals = Globals { eye, forward, right, up, pixel_angle };
        render.surface(surface, size, globals, |mut frame: Frame<'_>| {
            render::draw(&mut frame, &self.scene);
        });
    }
}
