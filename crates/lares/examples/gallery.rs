//! Current renders beside their photographs; missing references remain blank.
use image::{
    Rgb, RgbImage,
    imageops::{FilterType, overlay, resize},
};
use std::{error::Error, path::Path};

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
    for lights_off in [false, true] {
        let mut sheet = RgbImage::from_pixel(800, 300 * views.len() as u32, Rgb([30, 31, 33]));
        for (row, view) in views.iter().enumerate() {
            let reference = assets.join(format!("references/{view}-reference.webp"));
            for (column, path) in [
                (0, reference),
                (1, assets.join(format!("references/{view}{}.webp", if lights_off { "-lights-off" } else { "" }))),
            ] {
                if !path.exists() && column == 0 {
                    continue;
                }
                let image = image::open(path)?.to_rgb8();
                overlay(&mut sheet, &resize(&image, 400, 300, FilterType::Lanczos3), column * 400, row as i64 * 300);
            }
        }
        let output =
            assets.join(format!("references/comparison-lights-{}.webp", if lights_off { "off" } else { "on" }));
        sheet.save(&output)?;
        println!(
            "{}: photograph left, current render right; unphotographed rooms have a blank left column",
            output.display()
        );
    }
    Ok(())
}
