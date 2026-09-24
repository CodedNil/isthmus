use crate::{
    camera::Camera,
    home::Home,
    render::{self, Frame, Globals, Program},
};
use isthmus::{Render, SurfaceHandle, glam::Vec2};

pub struct App {
    pub home: Home,
    pub camera: Camera,
    dragging: bool,
    pointer: Vec2,
}

impl App {
    pub fn template() -> Self {
        let home = Home::template();
        let camera = Camera {
            target: Vec2::ZERO.extend(0.0),
            distance: (home.radius * 1.9).clamp(6.0, 40.0),
            ..Camera::default()
        };
        Self { home, camera, dragging: false, pointer: Vec2::ZERO }
    }

    pub fn pointer_moved(&mut self, position: Vec2) {
        let delta = position - self.pointer;
        self.pointer = position;
        if self.dragging {
            self.camera.orbit(delta);
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
        let globals = Globals { eye, forward, right, up, pixel_angle: self.camera.fov / size.y.max(1.0) };
        render.surface(surface, size, globals, |mut frame: Frame<'_>| {
            render::draw(&mut frame, &self.home.rooms, &self.home.bounds);
        });
    }
}
