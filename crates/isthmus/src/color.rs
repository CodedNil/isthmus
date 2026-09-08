use crate::glam::Vec4;

/// Composition of straight-alpha shader colors.
pub trait ColorExt {
    #[must_use]
    /// Scales opacity while preserving the represented color.
    fn opacity(self, opacity: f32) -> Self;
    /// Composites this foreground over a background, returning straight-alpha color.
    #[must_use]
    fn over(self, background: Self) -> Self;
}

impl ColorExt for Vec4 {
    fn opacity(mut self, opacity: f32) -> Self {
        self.w *= opacity;
        self
    }

    fn over(self, background: Self) -> Self {
        let behind = background.w * (1.0 - self.w);
        let alpha = self.w + behind;
        ((self.truncate() * self.w + background.truncate() * behind) / alpha.max(f32::MIN_POSITIVE)).extend(alpha)
    }
}
