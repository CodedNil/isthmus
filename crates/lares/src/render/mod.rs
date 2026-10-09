use crate::home::Object;
use acceleration::Node;
pub use acceleration::Scene;
use isthmus::prelude::*;
pub use scene::shade;

pub mod acceleration;
mod layout;
mod plants;
pub(crate) mod scene;

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
                // Four fixed subpixel rays cover thin geometry without temporal jitter.
                let mut color = Vec3::ZERO;
                let mut sample = 0;
                while sample < 4 {
                    let offset = vec2((sample & 1) as f32 - 0.5, (sample >> 1) as f32 - 0.5) * 0.5 * frame.pixel_size;
                    let ndc = (surface.pixel + offset) / frame.screen_size * 2.0 - 1.0;
                    let direction =
                        (frame.globals.forward + frame.globals.right * ndc.x - frame.globals.up * ndc.y).normalize();
                    color += shade(frame.globals, nodes, objects, grid, probes, direction, frame.time, sample)
                        .truncate()
                        .powf(2.2);
                    sample += 1;
                }
                (color * 0.25).powf(1.0 / 2.2).extend(1.0)
            })
    );
}
