#[cfg(target_os = "linux")]
pub use linux::run;

pub mod render;

#[cfg(target_os = "linux")]
mod linux;
