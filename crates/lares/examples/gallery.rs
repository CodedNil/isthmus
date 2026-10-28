//! Current renders beside their photographs; missing references remain blank.
use image::{
    Rgb, RgbImage,
    imageops::{FilterType, overlay, resize},
};
use std::{error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let views = [
        "entrance",
        "bookshelf",
        "kitchen-fridge",
        "kitchen-window",
        "kitchen-cooker",
        "lounge",
        "desk",
        "hall",
        "bedroom",
        "bathroom",
        "ensuite",
        "boiler-room",
        "spare-room",
        "storage-top",
        "storage-bottom",
    ];
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
    for old in ["comparison-lights-on.webp", "comparison-lights-off.webp"] {
        let path = assets.join("references").join(old);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    println!("{}: photograph left; evening lights on above daytime lights off at right", output.display());
    Ok(())
}
