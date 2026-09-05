use image::imageops::FilterType;
use isthmus::Image;
use reqwest::Client;
use resvg::{
    render,
    tiny_skia::{Pixmap, Transform},
    usvg::{self, Tree},
};

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod backend;

#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod backend;

pub use backend::Task;

/// One launchable application entry exposed to the launcher.
pub struct DesktopApp {
    pub name: String,
    pub exec: Vec<String>,
    pub comment: String,
    pub action: Option<(String, Vec<String>)>,
    pub icon: Option<Image>,
}

/// Platform-specific services used by the shared Cantus UI.
pub struct Platform;

impl Platform {
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

    pub async fn provider_icon(http: Client, url: String) -> Option<Image> {
        let bytes = http.get(url).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
        Self::decode_icon(&bytes)
    }
}
