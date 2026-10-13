//! `cargo run -p lares --release --example capture -- [directory [width height [view [utc-hour]]]]`
//! Uses the same four lighting/antialiasing samples as the live renderer.
use image::RgbImage;
use isthmus::glam::{Vec3, vec2, vec3};
use lares::{
    home,
    render::{Globals, Scene, SunSettings, shade, sun::utc_now},
};
use std::{env, error::Error, fs, thread, time::Instant};

const FOV: f32 = 1.30;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let destination = args.next().unwrap_or_else(|| format!("{}/assets/references", env!("CARGO_MANIFEST_DIR")));
    let width: usize = args.next().map_or(Ok(1200), |v| v.parse())?;
    let height: usize = args.next().map_or(Ok(900), |v| v.parse())?;
    let view = args.next();
    let now = utc_now();
    let utc_hour: Option<f64> = args.next().map(|hour| hour.parse()).transpose()?;
    if utc_hour.is_some_and(|hour| !hour.is_finite() || !(0.0..24.0).contains(&hour)) {
        return Err("UTC hour must be in [0, 24)".into());
    }
    let timestamp = utc_hour.map_or(now, |hour| (now / 86_400.0).floor() * 86_400.0 + hour * 3600.0);
    if width == 0 || height == 0 {
        return Err("image dimensions must be positive".into());
    }
    fs::create_dir_all(&destination)?;
    let started = Instant::now();
    let scene = Scene::new_at(&home::rooms(), timestamp, SunSettings::default());
    println!("Scene: {:?}", started.elapsed());
    for (name, eye, focus) in [
        ("entrance", vec3(-0.75, -0.65, 1.55), vec3(0.00, -1.75, 1.25)),
        ("bookshelf", vec3(-2.10, 1.15, 1.45), vec3(-2.15, -0.45, 1.05)),
        ("kitchen-fridge", vec3(-4.25, -0.30, 1.58), vec3(-2.72, -1.75, 1.34)),
        ("kitchen-window", vec3(-4.12, -0.17, 1.58), vec3(-4.12, -2.75, 1.26)),
        ("kitchen-cooker", vec3(-3.18, -0.43, 1.58), vec3(-5.10, -1.60, 1.24)),
        ("lounge", vec3(-2.55, 0.55, 1.50), vec3(-5.15, 1.55, 0.65)),
        ("desk", vec3(-1.65, 0.45, 1.55), vec3(-0.25, 1.90, 0.90)),
        ("bedroom", vec3(3.9, 0.0, 1.5), vec3(5.5, 2.5, 0.9)),
        ("kitchen-lights", vec3(-4.12, -0.17, 1.58), vec3(-4.12, -2.4, 2.1)),
        ("overhead", vec3(5.5, 6.0, 8.0), vec3(0.0, 0.0, 0.0)),
    ] {
        if view.as_ref().is_some_and(|v| v != name) {
            continue;
        }
        let started = Instant::now();
        let pixels = pixels(&scene, eye, focus, width, height);
        let image = RgbImage::from_raw(width as u32, height as u32, pixels).unwrap();
        image.save(format!("{destination}/{name}.webp"))?;
        println!("{name}: {:?}", started.elapsed());
    }
    Ok(())
}

fn pixels(scene: &Scene, eye: Vec3, focus: Vec3, width: usize, height: usize) -> Vec<u8> {
    let scale = (FOV * 0.5).tan();
    let forward = (focus - eye).normalize();
    let right = Vec3::Z.cross(forward).normalize();
    let up = forward.cross(right).normalize() * scale;
    let right = right * (width as f32 / height as f32 * scale);
    let globals = Globals { eye, forward, right, up, pixel_angle: FOV / height as f32, sun: scene.sun };
    let mut pixels = vec![0; width * height * 3];
    let workers = thread::available_parallelism().map_or(1, usize::from).min(16);
    let rows = height.div_ceil(workers);
    thread::scope(|scope| {
        for (chunk, pixels) in pixels.chunks_mut(rows * width * 3).enumerate() {
            scope.spawn(move || {
                for (index, pixel) in pixels.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                    let x = index % width;
                    let y = chunk * rows + index / width;
                    let mut color = Vec3::ZERO;
                    for sample in 0..4 {
                        let offset = vec2((sample & 1) as f32 - 0.5, (sample >> 1) as f32 - 0.5) * 0.5;
                        let ndc = (vec2(x as f32 + 0.5, y as f32 + 0.5) + offset) / vec2(width as f32, height as f32)
                            * 2.0
                            - 1.0;
                        let ray = (forward + right * ndc.x - up * ndc.y).normalize();
                        color += shade(
                            globals,
                            &scene.nodes,
                            &scene.objects,
                            &scene.lights,
                            scene.grid,
                            &scene.probes,
                            ray,
                            0.0,
                            sample,
                        )
                        .truncate()
                        .powf(2.2);
                    }
                    let color = (color * 0.25).powf(1.0 / 2.2);
                    pixel.copy_from_slice(&color.to_array().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
            });
        }
    });
    pixels
}
