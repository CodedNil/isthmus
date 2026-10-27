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
