//! Current renders beside their photographs; missing references remain blank.
use image::{
    Rgb, RgbImage,
    imageops::{FilterType, overlay, resize},
};
use std::{error::Error, path::Path};

#[path = "support/views.rs"]
#[allow(dead_code, reason = "The gallery reads camera names without rendering")]
mod views;

fn main() -> Result<(), Box<dyn Error>> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let views: Vec<_> = views::all(0.0)
        .into_iter()
        .filter(|view| !matches!(view.name, "plan" | "overhead"))
        .map(|view| view.name)
        .collect();
    let mut sheet = RgbImage::from_pixel(800, 600 * views.len() as u32, Rgb([30, 31, 33]));
    for (row, view) in views.iter().enumerate() {
        let reference = assets.join(format!("references/{view}-reference.webp"));
        if reference.exists() {
            let photo = image::open(reference)?.to_rgb8();
            overlay(&mut sheet, &resize(&photo, 400, 300, FilterType::Lanczos3), 0, row as i64 * 600 + 150);
        }
        let render = image::open(assets.join(format!("references/{view}.webp")))?.to_rgb8();
        overlay(&mut sheet, &resize(&render, 400, 600, FilterType::Lanczos3), 400, row as i64 * 600);
    }
    let output = assets.join("references/comparison.webp");
    sheet.save(&output)?;
    println!("{}: photograph left; evening lights on above daytime lights off at right", output.display());
    Ok(())
}
