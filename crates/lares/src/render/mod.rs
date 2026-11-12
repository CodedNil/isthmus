//! The GPU path tracer's shader program: the shared globals, the world constants and the render modules.
pub use crate::{
    render::{
        geometry::acceleration::Scene,
        light::sun::{Sun, SunSettings},
        pipeline::targets::Targets,
    },
    scene::{pixel, pixel_with},
};
pub use geometry::acceleration;
use isthmus::prelude::*;
pub use light::{photometry, shadows, sun};
pub use pipeline::temporal;

pub mod geometry {
    pub mod acceleration;
    pub(crate) mod layout;
    pub(crate) mod light_culling;
    pub(crate) mod plants;
}

pub mod light {
    pub mod photometry;
    pub mod shadows;
    pub mod sun;
}

pub(crate) mod pipeline {
    mod ghosts;
    pub mod passes;
    mod resolve;
    pub mod targets;
    pub mod temporal;
}

pub const WALL_HEIGHT: f32 = 2.4;
pub const WALL_THICKNESS: f32 = 0.1;
pub const EXTERIOR_WALL_THICKNESS: f32 = 0.30;
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
    /// Photometric camera exposure, metered separately from light power.
    pub exposure: f32,
    /// Bounded photographic illuminant adaptation, applied before tone mapping.
    pub white_balance: Vec3,
    pub sun: Sun,
    pub sample_index: u32,
    pub interleaved: bool,
    pub parity: u32,
    /// Four tetrahedral SDF samples; uniform so drivers keep one distance evaluator.
    pub normal_samples: u32,
}

isthmus::program!(Globals, ());
