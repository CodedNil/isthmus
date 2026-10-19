use crate::home::Object;
use acceleration::Node;
pub use acceleration::Scene;
use isthmus::prelude::*;
pub use scene::pixel;
pub use sun::{Sun, SunSettings};

pub mod acceleration;
mod layout;
mod plants;
pub(crate) mod scene;
pub mod sun;

pub const WALL_HEIGHT: f32 = 2.4;
pub const WALL_THICKNESS: f32 = 0.1;
pub const FLOOR_THICKNESS: f32 = 0.12;
pub const CEILING_THICKNESS: f32 = 0.04;
pub const DOOR_HEIGHT: f32 = 2.05;
pub const WINDOW_SILL: f32 = 0.9;
pub const WINDOW_HEAD: f32 = 2.05;

#[derive(Clone, Copy, ShaderData)]
pub struct Globals {
    /// Camera position in world coordinates.
    pub eye: Vec3,
    /// Ray through the center of the screen.
    pub forward: Vec3,
    /// Horizontal ray offset at the screen edge.
    pub right: Vec3,
    /// Vertical ray offset at the screen edge.
    pub up: Vec3,
    /// Vertical camera angle per pixel, in radians.
    pub pixel_angle: f32,
    pub sun: Sun,
}

isthmus::program!(Globals, ());

pub fn draw(frame: &mut Frame<'_>, scene: &Scene) {
    shader!(
        frame
            .blend(Blend::Replace)
            .upload({
                let nodes: &[Node] = &scene.nodes;
                let objects: &[Object] = &scene.objects;
                let lights: &[u32] = &scene.lights;
                let probes: &[scene::irradiance::Probe] = &scene.probes;
                let grid: scene::irradiance::Grid = scene.grid;
            })
            .primitive(|frame| Rect::new(Vec2::ZERO, frame.screen_size))
            .fragment(|frame, surface| {
                let (globals, size) = (frame.globals, frame.screen_size);
                pixel(globals, nodes, objects, lights, grid, probes, surface.pixel, size, frame.pixel_size, frame.time)
                    .extend(1.0)
            })
    );
}
