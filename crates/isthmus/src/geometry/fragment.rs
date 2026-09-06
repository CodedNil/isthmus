use super::{Raster, text::TextResources};
use crate::{Program, glam::Vec2};
use core::ops::Deref;

/// Common fragment context with statically typed geometry-specific coordinates and queries.
#[derive(Clone, Copy)]
pub struct Fragment<'a, P: Program, G: FragmentGeometry<'a>> {
    /// Screen position in logical pixels.
    pub pixel: Vec2,
    /// Frame time in seconds.
    pub time: f32,
    /// Application data shared by every draw in this frame.
    pub globals: P::Globals,
    /// Geometry-specific coordinates and distance queries.
    pub geometry: G::Sample,
}

impl<'a, P: Program, G: FragmentGeometry<'a>> Deref for Fragment<'a, P, G> {
    type Target = G::Sample;

    fn deref(&self) -> &G::Sample {
        &self.geometry
    }
}

/// Defines a geometry's encoded payload, rasterization, and fragment queries together.
pub trait FragmentGeometry<'a>: Sized {
    /// Coordinates and queries available at the current fragment.
    type Sample: Copy;
    /// Data needed to reconstruct this geometry's fragment queries.
    type Payload: crate::ShaderData;
    /// Primitive used to cover the geometry on screen.
    type Raster: Raster;
    /// Constructs fragment queries from the raster primitive and its payload.
    fn sample(pixel: Vec2, raster: [Vec2; 3], payload: Self::Payload, text: TextResources<'a>) -> Self::Sample;
}

#[doc(hidden)]
pub trait ShaderInput<'a> {
    type Program: Program;
    type Geometry: FragmentGeometry<'a>;
}

impl<'a, P: Program, G: FragmentGeometry<'a>> ShaderInput<'a> for Fragment<'a, P, G> {
    type Geometry = G;
    type Program = P;
}

impl<'a, P: Program, G: FragmentGeometry<'a>> Fragment<'a, P, G> {
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
