//! Shared rasterization bounds for effects on distance fields and vector text.

/// An effect that declares its distance-query and coordinate-displacement reach.
pub trait Effect: Copy {
    /// Maximum distance outside the original contour used by the effect.
    fn outset(self) -> f32 {
        0.0
    }

    /// Maximum coordinate displacement in logical pixels.
    fn displacement(self) -> f32 {
        0.0
    }
}

/// An exterior outline whose width also determines the required rasterization reach.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct Outline {
    /// Nonnegative outline width in logical pixels.
    pub width: f32,
}

impl Effect for Outline {
    fn outset(self) -> f32 {
        self.width.max(0.0)
    }
}

impl Outline {
    /// Antialiases and composites straight-alpha fill and outline colors using this outline's width.
    pub fn color(self, distance: f32, fill: glam::Vec4, outline: glam::Vec4) -> glam::Vec4 {
        let (coverage, outline_coverage) = super::sdf::fill_outline(distance, self.width);
        let fill_alpha = coverage * fill.w;
        let outline_alpha = outline_coverage * outline.w;
        let alpha = fill_alpha + outline_alpha;
        ((fill.truncate() * fill_alpha + outline.truncate() * outline_alpha) / alpha.max(0.0001)).extend(alpha)
    }
}
