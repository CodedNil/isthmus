use isthmus::MinMax;

/// Constant-speed approach to a goal, equivalent to repeatedly calling `move_towards`.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Ramp {
    from: f32,
    to: f32,
    start: f32,
    /// Units per second; zero holds `from`.
    rate: f32,
}

impl Ramp {
    /// A value at rest.
    pub const fn new(value: f32) -> Self {
        Self { from: value, to: value, start: 0.0, rate: 0.0 }
    }

    /// The value at `time` seconds; times before the start hold the starting value.
    pub fn at(self, time: f32) -> f32 {
        let travelled = (time - self.start).max(0.0) * self.rate;
        self.from + (self.to - self.from).clamp_num(-travelled, travelled)
    }

    /// Jumps to `value` without animating.
    pub const fn set(&mut self, value: f32) {
        *self = Self::new(value);
    }

    /// Heads for `goal` at `rate` units per second, continuing from the current value.
    /// Repeating the same request is free, so this can be called every frame. Returns the value now.
    pub fn approach(&mut self, time: f32, goal: f32, rate: f32) -> f32 {
        // Bitwise: only a changed request restarts the ramp.
        if goal.to_bits() != self.to.to_bits() || rate.to_bits() != self.rate.to_bits() {
            *self = Self { from: self.at(time), to: goal, start: time, rate };
        }
        self.at(time)
    }

    /// Like `approach`, taking the seconds a full 0-1 sweep should last.
    pub fn approach_over(&mut self, time: f32, goal: f32, seconds: f32) -> f32 {
        self.approach(time, goal, seconds.recip())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_continuous_when_repeating_or_changing_goals() {
        let mut ramp = Ramp::new(0.0);
        ramp.approach_over(0.0, 1.0, 0.5);
        assert_eq!(ramp.approach_over(0.125, 1.0, 0.5), 0.25);
        assert_eq!(ramp.at(0.25), 0.5);
        assert_eq!(ramp.at(1.0), 1.0);
        assert_eq!(ramp.approach(0.25, 0.0, 2.0), 0.5);
        assert_eq!(ramp.at(0.375), 0.25);
        assert_eq!(ramp.at(1.0), 0.0);
    }
}
