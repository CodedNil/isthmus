use crate::home::Object;
use acceleration::Node;
pub use acceleration::Scene;
use isthmus::prelude::*;

pub mod acceleration;
mod layout;
mod plants;
mod scene;

pub const WALL_HEIGHT: f32 = 2.4;
pub const WALL_THICKNESS: f32 = 0.1;
pub const FLOOR_THICKNESS: f32 = 0.12;
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
}

isthmus::program!(Globals, ());

pub fn draw(frame: &mut Frame<'_>, scene: &Scene) {
    shader!(
        frame
            .blend(Blend::Replace)
            .upload({
                let nodes: &[Node] = &scene.nodes;
                let objects: &[Object] = &scene.objects;
                let probes: &[scene::irradiance::Probe] = &scene.probes;
                let grid: scene::irradiance::Grid = scene.grid;
            })
            .primitive(|frame| { Rect::new(Vec2::ZERO, frame.screen_size) })
            .fragment(|frame, surface| {
                let ndc = surface.pixel / frame.screen_size * 2.0 - 1.0;
                let direction =
                    (frame.globals.forward + frame.globals.right * ndc.x - frame.globals.up * ndc.y).normalize();
                scene::shade(frame.globals, nodes, objects, grid, probes, direction, frame.time)
            })
    );
}
