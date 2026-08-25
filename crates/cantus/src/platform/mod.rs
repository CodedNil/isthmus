#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub use {linux::DesktopApp, linux::Linux as Platform};
