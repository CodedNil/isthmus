//! `cargo run -p lares --release --example capture -- [directory [width height [view [utc-hour]]]]`
//! Uses the same four lighting/antialiasing samples as the live renderer.
use core::f32::consts::PI;
use image::{RgbImage, imageops::overlay};
use isthmus::glam::{Vec3, vec2, vec3};
use lares::{
    home,
    render::{Globals, Scene, SunSettings, pixel, pixel_cached, shadows::ShadowAtlas, sun::utc_now},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    path::Path,
    thread,
    time::Instant,
};

#[path = "support/views.rs"]
mod views;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let destination = args.next().unwrap_or_else(|| format!("{}/assets/references", env!("CARGO_MANIFEST_DIR")));
    let width: usize = args.next().map_or(Ok(1200), |v| v.parse())?;
    let height: usize = args.next().map_or(Ok(900), |v| v.parse())?;
    let requested = args.next();
    let now = utc_now();
    let utc_hour: Option<f64> = args.next().map(|hour| hour.parse()).transpose()?;
    if utc_hour.is_some_and(|hour| !hour.is_finite() || !(0.0..24.0).contains(&hour)) {
        return Err("UTC hour must be in [0, 24)".into());
    }
    let timestamp: f64 = if let Ok(value) = env::var("LARES_TIMESTAMP") {
        value.parse()?
    } else {
        (now / 86_400.0).floor() * 86_400.0 + utc_hour.unwrap_or(12.0) * 3600.0
    };
    if width == 0 || height == 0 {
        return Err("image dimensions must be positive".into());
    }
    fs::create_dir_all(&destination)?;
    let started = Instant::now();
    let rooms = home::rooms();
    // Evening fixtures and midday daylight are separate photographic setups.
    let evening = (timestamp / 86_400.0).floor() * 86_400.0 + 21.0 * 3600.0;
    let mut scene = Scene::new_at(&rooms, evening, SunSettings::default());
    println!("Scene: {:?}", started.elapsed());
    println!("Captures: evening lights on (21:00 UTC) above daytime lights off; independently metered");
    let luminance = vec3(0.2126, 0.7152, 0.0722);
    println!(
        "Evening daylight: {:.0} lx direct normal, {:.0} lx diffuse horizontal; clear sky, view-metered camera",
        scene.sun.radiance.dot(luminance) * 100.0,
        scene.sun.sky.dot(luminance) * 100.0 * PI * 0.85
    );
    let shadows = env::var("LARES_SHADOW_RESOLUTION").ok().map(|resolution| {
        let started = Instant::now();
        let atlas = ShadowAtlas::bake_cpu(&scene, resolution.parse().expect("shadow resolution must be an integer"));
        println!("Shadow atlas: {:?}", started.elapsed());
        atlas
    });
    let mut evening_images = BTreeMap::new();
    for lights_off in [false, true] {
        if lights_off {
            scene.set_time(timestamp);
            let entities: BTreeSet<_> =
                rooms.iter().flat_map(|room| room.lights.iter().map(|light| light.entity_id)).collect();
            let states: Vec<_> = entities.into_iter().map(|entity| (entity, 0.0)).collect();
            scene.set_light_states(&states);
        }
        for view in views::all(scene.radius) {
            let name = view.name;
            if requested.as_ref().is_some_and(|selection| {
                if selection == "matched" {
                    !Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join(format!("assets/references/{name}-reference.webp"))
                        .exists()
                } else {
                    selection != "all" && !selection.split(',').any(|selected| selected == name)
                }
            }) {
                continue;
            }
            let started = Instant::now();
            let pixels = pixels(&scene, view, width, height, shadows.as_ref());
            let image = RgbImage::from_raw(width as u32, height as u32, pixels).unwrap();
            if lights_off {
                let mut stacked = RgbImage::new(width as u32, height as u32 * 2);
                overlay(&mut stacked, &evening_images.remove(name).unwrap(), 0, 0);
                overlay(&mut stacked, &image, 0, height as i64);
                stacked.save(format!("{destination}/{name}.webp"))?;
                let old = Path::new(&destination).join(format!("{name}-lights-off.webp"));
                if old.exists() {
                    fs::remove_file(old)?;
                }
            } else {
                evening_images.insert(name, image);
            }
            println!("{name}: {:?}", started.elapsed());
        }
    }
    Ok(())
}

#[allow(clippy::option_if_let_else, reason = "Keep reference and cached shader calls readable side by side")]
fn pixels(scene: &Scene, view: views::View, width: usize, height: usize, shadows: Option<&ShadowAtlas>) -> Vec<u8> {
    let eye = view.eye;
    let (forward, right, up) = view.rays(width as f32 / height as f32);
    let (exposure, white_balance) = scene.camera_settings(eye, forward, right, up);
    println!("Camera EV100: {:.2}", (100.0 / (1.2 * exposure)).log2());
    let globals = Globals {
        eye,
        exposure,
        white_balance,
        forward,
        right,
        up,
        pixel_angle: view.fov / height as f32,
        sun: scene.sun,
        sample_index: 0,
        interleaved: false,
        parity: 0,
    };
    // Keep the original exposure when isolating direct lighting, so a
    // diagnostic difference measures transport rather than camera adaptation.
    let direct_only = env::var("LARES_CAPTURE_GI").is_ok_and(|value| value == "off").then(|| {
        scene
            .probes
            .iter()
            .copied()
            .map(|mut probe| {
                probe.light.constant = Vec3::ZERO;
                probe.light.x = Vec3::ZERO;
                probe.light.y = Vec3::ZERO;
                probe.light.z = Vec3::ZERO;
                probe
            })
            .collect::<Vec<_>>()
    });
    let probes = direct_only.as_deref().unwrap_or(&scene.probes);
    let mut pixels = vec![0; width * height * 3];
    let workers = thread::available_parallelism().map_or(1, usize::from).div_ceil(2).min(8);
    let rows = height.div_ceil(workers);
    thread::scope(|scope| {
        for (chunk, pixels) in pixels.chunks_mut(rows * width * 3).enumerate() {
            scope.spawn(move || {
                for (index, rgb) in pixels.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                    let x = index % width;
                    let y = chunk * rows + index / width;
                    let color = if let Some(shadows) = shadows {
                        pixel_cached(
                            globals,
                            &scene.nodes,
                            &scene.objects,
                            &scene.lights,
                            &scene.light_masks,
                            &scene.contacts,
                            scene.grid,
                            probes,
                            vec2(x as f32 + 0.5, y as f32 + 0.5),
                            vec2(width as f32, height as f32),
                            1.0,
                            0.0,
                            &shadows.image,
                            shadows.info,
                        )
                    } else {
                        pixel(
                            globals,
                            &scene.nodes,
                            &scene.objects,
                            &scene.lights,
                            &scene.light_masks,
                            &scene.contacts,
                            scene.grid,
                            probes,
                            vec2(x as f32 + 0.5, y as f32 + 0.5),
                            vec2(width as f32, height as f32),
                            1.0,
                            0.0,
                        )
                    };
                    rgb.copy_from_slice(&color.to_array().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
            });
        }
    });
    pixels
}
