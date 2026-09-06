use super::{QuadSample, Raster, text::TextResources};
use crate::{Program, glam::Vec2};
use core::ops::Deref;

/// Common fragment context with statically typed geometry-specific coordinates and queries.
#[derive(Clone, Copy)]
pub struct Fragment<P: Program, G = QuadSample> {
    /// Screen position in logical pixels.
    pub pixel: Vec2,
    /// Frame time in seconds.
    pub time: f32,
    /// Application data shared by every draw in this frame.
    pub globals: P::Globals,
    /// Geometry-specific coordinates and distance queries.
    pub geometry: G,
}

impl<P: Program, G> Deref for Fragment<P, G> {
    type Target = G;

    fn deref(&self) -> &G {
        &self.geometry
    }
}

/// Defines a geometry's encoded payload, rasterization, and fragment queries together.
pub trait GeometrySample<'a>: Sized {
    /// Data needed to reconstruct this geometry's fragment queries.
    type Payload: crate::ShaderData;
    /// Primitive used to cover the geometry on screen.
    type Raster: Raster;
    /// Constructs fragment queries from the raster primitive and its payload.
    fn sample(pixel: Vec2, raster: [Vec2; 3], payload: Self::Payload, text: TextResources<'a>) -> Self;
}

#[doc(hidden)]
pub trait ShaderInput<'a> {
    type Program: Program;
    type Sample: GeometrySample<'a>;
}

impl<'a, P: Program, G: GeometrySample<'a>> ShaderInput<'a> for Fragment<P, G> {
    type Program = P;
    type Sample = G;
}

impl<'a, P: Program, G: GeometrySample<'a>> Fragment<P, G> {
    #[doc(hidden)]
    pub fn new(
        pixel: Vec2,
        raster: [Vec2; 3],
        payload: G::Payload,
        time: f32,
        globals: P::Globals,
        text: TextResources<'a>,
    ) -> Self {
        Self { pixel, time, globals, geometry: G::sample(pixel, raster, payload, text) }
    }
}
