//! Persistent sample validity. Only discontinuous view/source changes reset it.
#[derive(Clone, Copy)]
pub struct Accumulation {
    phase: u32,
    mask: u32,
    phases: u32,
}
impl Accumulation {
    pub const fn new(interleaved: bool) -> Self {
        Self { phase: 0, mask: 1, phases: if interleaved { 8 } else { 4 } }
    }

    pub const fn converged(self) -> bool {
        self.mask == (1 << self.phases) - 1
    }

    pub const fn advance(&mut self, invalidate: bool) -> (u32, u32) {
        if invalidate {
            self.phase = 0;
            self.mask = 0;
        } else {
            self.phase = (self.phase + 1) % self.phases;
        }
        self.mask |= 1 << self.phase;
        (self.phase, self.mask)
    }
}

/// Budget ray-traced pixels while the camera moves. History is recreated when
/// this extent changes; stationary views always return to native resolution.
#[derive(Clone, Copy)]
pub struct MotionResolution {
    scale: f32,
    fast_frames: u32,
}
impl Default for MotionResolution {
    fn default() -> Self {
        Self { scale: 0.5, fast_frames: 0 }
    }
}
impl MotionResolution {
    pub fn observe(&mut self, seconds: f32) {
        // Fall quickly, recover slowly. Increasing after every cheap frame
        // oscillated between two extents and repeatedly discarded AA history.
        if seconds > 0.030 {
            self.scale = (self.scale * 0.75).max(0.25);
            self.fast_frames = 0;
        } else if seconds < 0.014 {
            self.fast_frames += 1;
            if self.fast_frames >= 30 {
                self.scale = (self.scale / 0.75).min(1.0);
                self.fast_frames = 0;
            }
        } else {
            self.fast_frames = 0;
        }
    }

    pub fn extent(self, native: [u32; 2]) -> [u32; 2] {
        native.map(|value| ((value as f32 * self.scale).round() as u32).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_budget_is_bounded_and_resized_histories_restart_cleanly() {
        let mut quality = MotionResolution::default();
        for _ in 0..40 {
            quality.observe(0.20);
        }
        assert_eq!(quality.extent([2400, 1600]), [600, 400]);
        for _ in 0..240 {
            quality.observe(0.01);
        }
        assert_eq!(quality.extent([2400, 1600]), [2400, 1600]);
        assert_eq!(quality.extent([1, 1]), [1, 1]);
        for interleaved in [false, true] {
            let mut history = Accumulation::new(interleaved);
            for _ in 0..20 {
                history.advance(false);
            }
            assert!(history.converged());
            let (phase, mask) = history.advance(true);
            assert_eq!((phase, mask), (0, 1));
            for _ in 0..8 {
                history.advance(false);
            }
            assert!(history.converged());
        }
    }
}
