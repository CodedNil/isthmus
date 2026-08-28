#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
use std::ops::Add;

#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Instant(f64);

#[cfg(target_arch = "wasm32")]
impl Instant {
    pub fn now() -> Self {
        Self(js_sys::Date::now())
    }

    pub fn elapsed(self) -> Duration {
        Duration::from_secs_f64(((js_sys::Date::now() - self.0) / 1_000.0).max(0.0))
    }
}

#[cfg(target_arch = "wasm32")]
impl Add<Duration> for Instant {
    type Output = Self;

    fn add(self, duration: Duration) -> Self::Output {
        Self(self.0 + duration.as_secs_f64() * 1_000.0)
    }
}
