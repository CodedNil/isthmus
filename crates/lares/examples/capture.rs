use image::{
    RgbImage,
    imageops::{FilterType, resize},
};
use isthmus::glam::{Vec3, vec2, vec3};
use lares::{
    home,
    render::{Globals, Scene, shade},
};
use std::{error::Error, fs, thread, time::Instant};

const FOV: f32 = 1.30;

fn main() -> Result<(), Box<dyn Error>> {
    let destination = format!("{}/assets/references", env!("CARGO_MANIFEST_DIR"));
    fs::create_dir_all(&destination)?;
    let started = Instant::now();
    let scene = Scene::new(&home::rooms());
    println!("Scene: {:?}", started.elapsed());
    for (name, eye, focus) in [
        ("entrance", vec3(-0.75, -0.65, 1.55), vec3(0.00, -1.75, 1.25)),
        ("bookshelf", vec3(-2.10, 1.15, 1.45), vec3(-2.15, -0.45, 1.05)),
        ("kitchen-fridge", vec3(-4.25, -0.30, 1.58), vec3(-2.72, -1.75, 1.34)),
        ("kitchen-window", vec3(-4.12, -0.17, 1.58), vec3(-4.12, -2.75, 1.26)),
        ("kitchen-cooker", vec3(-3.18, -0.43, 1.58), vec3(-5.10, -1.60, 1.24)),
        ("lounge", vec3(-2.55, 0.55, 1.50), vec3(-5.15, 1.55, 0.65)),
        ("desk", vec3(-1.65, 0.45, 1.55), vec3(-0.25, 1.90, 0.90)),
    ] {
        let started = Instant::now();
        let pixels = pixels(&scene, eye, focus, 2400, 1800);
        let image = RgbImage::from_raw(2400, 1800, pixels).unwrap();
        resize(&image, 1200, 900, FilterType::Lanczos3).save(format!("{destination}/{name}.webp"))?;
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
    let globals = Globals { eye, forward, right, up, pixel_angle: FOV / height as f32 };
    let mut pixels = vec![0; width * height * 3];
    let workers = thread::available_parallelism().map_or(1, usize::from).min(16);
    let rows = height.div_ceil(workers);
    thread::scope(|scope| {
        for (chunk, pixels) in pixels.chunks_mut(rows * width * 3).enumerate() {
            scope.spawn(move || {
                for (index, pixel) in pixels.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                    let x = index % width;
                    let y = chunk * rows + index / width;
                    let ndc = vec2((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32) * 2.0 - 1.0;
                    let ray = (forward + right * ndc.x - up * ndc.y).normalize();
                    let color = shade(
                        globals,
                        &scene.nodes,
                        &scene.objects,
                        scene.grid,
                        &scene.probes,
                        ray,
                        0.0,
                        ((x & 1) | ((y & 1) << 1)) as u32,
                    );
                    pixel.copy_from_slice(
                        &color.truncate().to_array().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                    );
                }
            });
        }
    });
    pixels
}
