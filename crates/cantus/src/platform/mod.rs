use image::imageops::FilterType;
use isthmus::Image;
use resvg::{
    render,
    tiny_skia::{Pixmap, Transform},
    usvg::{self, Tree},
};
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

#[cfg(target_arch = "wasm32")]
mod web;
pub use backend::{
    Task, desktop_apps, open_url, run, run_power_action, set_volume, sleep, spawn, start_launcher_listener,
    start_location_monitor, start_status_monitor, trigger_launcher,
};
#[cfg(target_arch = "wasm32")]
use web as backend;

pub const STATUS_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

/// One launchable application entry exposed to the launcher.
pub struct DesktopApp {
    pub name: String,
    pub exec: Vec<String>,
    pub comment: String,
    pub action: Option<(String, Vec<String>)>,
    pub icon: Option<Image>,
}

pub fn decode_icon(bytes: &[u8]) -> Option<Image> {
    const SIZE: u32 = 48;
    let pixels = if let Ok(image) = image::load_from_memory(bytes) {
        image.resize_to_fill(SIZE, SIZE, FilterType::Triangle).into_rgba8().into_raw()
    } else {
        let tree = Tree::from_data(bytes, &usvg::Options::default()).ok()?;
        let mut pixmap = Pixmap::new(SIZE, SIZE)?;
        let source = tree.size();
        render(
            &tree,
            Transform::from_scale(SIZE as f32 / source.width(), SIZE as f32 / source.height()),
            &mut pixmap.as_mut(),
        );
        pixmap.take_demultiplied()
    };
    Some(Image::rgba8([SIZE; 2], pixels))
}
