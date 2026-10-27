//! Compare reference and candidate captures, optionally saving a 4× error heatmap.
use core::array::from_fn;
use std::{env, error::Error};
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let reference = image::open(args.next().ok_or("reference path required")?)?.to_rgb8();
    let candidate = image::open(args.next().ok_or("candidate path required")?)?.to_rgb8();
    if reference.dimensions() != candidate.dimensions() {
        return Err("image dimensions differ".into());
    }
    let mut error = 0u64;
    let mut squared = 0u64;
    let mut changed = 0usize;
    let mut large = 0usize;
    let mut maximum = 0u8;
    let mut heat = candidate.clone();
    for ((a, b), out) in reference.pixels().zip(candidate.pixels()).zip(heat.pixels_mut()) {
        let diff = from_fn::<_, 3, _>(|i| a[i].abs_diff(b[i]));
        let max = *diff.iter().max().unwrap();
        changed += usize::from(max > 4);
        large += usize::from(max > 16);
        maximum = maximum.max(max);
        for &v in &diff {
            error += u64::from(v);
            squared += u64::from(v) * u64::from(v);
        }
        *out = image::Rgb(diff.map(|v| v.saturating_mul(4)));
    }
    let count = f64::from(reference.width()) * f64::from(reference.height());
    let mse = squared as f64 / (count * 3.0);
    println!(
        "MAE {:.3}/255; PSNR {:.2} dB; pixels >4: {:.2}%; >16: {:.2}%; max {}",
        error as f64 / (count * 3.0),
        10.0 * (255.0 * 255.0 / mse).log10(),
        changed as f64 / count * 100.0,
        large as f64 / count * 100.0,
        maximum
    );
    if let Some(path) = args.next() {
        heat.save(path)?;
    }
    Ok(())
}
