//! Painting fills and outlines, and compositing the result.
use isthmus::{Fragment, glam::Vec4};

/// Composes fill and outline into a covered straight RGBA colour.
pub trait Paint {
    /// Includes antialiased fill and outer coverage in the returned colour.
    fn paint(self, fill: Vec4, outline: Vec4) -> Vec4;
}

/// Painting for fragments whose primitive sample knows how to [`Paint`] itself.
pub trait FragmentExt {
    /// Paints the main primitive, accounting for its deferred coverage exactly once.
    fn paint(&self, fill: Vec4, outline: Vec4) -> Vec4;
}

impl<T: Paint + Copy> FragmentExt for Fragment<T> {
    fn paint(&self, fill: Vec4, outline: Vec4) -> Vec4 {
        let color = self.sample.paint(fill, outline);
        color.with_w(color.w / self.coverage.max(f32::MIN_POSITIVE))
    }
}

/// Composites straight RGBA foreground over background.
pub fn source_over(foreground: Vec4, background: Vec4) -> Vec4 {
    let behind = background.w * (1.0 - foreground.w);
    let alpha = foreground.w + behind;
    ((foreground.truncate() * foreground.w + background.truncate() * behind) / alpha.max(f32::MIN_POSITIVE))
        .extend(alpha)
}
