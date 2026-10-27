//! `cargo run -p lares --release --example capture -- [directory [width height [view [utc-hour]]]]`
//! Uses the same four lighting/antialiasing samples as the live renderer.
use core::f32::consts::PI;
use image::RgbImage;
use isthmus::glam::{Vec3, vec2, vec3};
use lares::{
    home,
    render::{Globals, Scene, SunSettings, pixel, pixel_cached, shadows::ShadowAtlas, sun::utc_now},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs, thread,
    time::Instant,
};

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
    let timestamp = if let Ok(value) = env::var("LARES_TIMESTAMP") {
        value.parse()?
    } else {
        utc_hour.map_or(now, |hour| (now / 86_400.0).floor() * 86_400.0 + hour * 3600.0)
    };
    if width == 0 || height == 0 {
        return Err("image dimensions must be positive".into());
    }
    fs::create_dir_all(&destination)?;
    let started = Instant::now();
    let rooms = home::rooms();
    let mut scene = Scene::new_at(&rooms, timestamp, SunSettings::default());
    println!("Scene: {:?}", started.elapsed());
    let luminance = vec3(0.2126, 0.7152, 0.0722);
    println!(
        "Daylight: {:.0} lx direct normal, {:.0} lx diffuse horizontal; clear sky, view-metered camera",
        scene.sun.radiance.dot(luminance) * 100.0,
        scene.sun.sky.dot(luminance) * 100.0 * PI * 0.85
    );
    let shadows = env::var("LARES_SHADOW_RESOLUTION").ok().map(|resolution| {
        let started = Instant::now();
        let atlas = ShadowAtlas::bake_cpu(&scene, resolution.parse().expect("shadow resolution must be an integer"));
        println!("Shadow atlas: {:?}", started.elapsed());
        atlas
    });
    let paired = env::var("LARES_LIGHTS").is_ok_and(|value| value == "both");
    let off_only = env::var("LARES_LIGHTS").is_ok_and(|value| value == "off");
    let mut exposures = BTreeMap::new();
    for lights_off in if paired {
        &[false, true][..]
    } else if off_only {
        &[true][..]
    } else {
        &[false][..]
    } {
        if *lights_off {
            let entities: BTreeSet<_> =
                rooms.iter().flat_map(|room| room.lights.iter().map(|light| light.entity_id)).collect();
            for entity in entities {
                scene.set_light_state(entity, 0.0);
            }
        }
        for (name, eye, focus) in [
            ("plan", vec3(0.0, 0.0, (scene.radius * 2.0).clamp(6.0, 40.0)), Vec3::ZERO),
            ("entrance", vec3(-0.65, -0.10, 1.55), vec3(-0.20, -1.70, 1.25)),
            ("bookshelf", vec3(-2.10, 1.15, 1.45), vec3(-2.15, -0.45, 1.05)),
            ("kitchen-fridge", vec3(-4.25, -0.30, 1.58), vec3(-2.72, -1.75, 1.34)),
            ("kitchen-window", vec3(-4.12, -0.17, 1.58), vec3(-4.12, -2.75, 1.26)),
            ("kitchen-cooker", vec3(-3.15, -1.18, 1.58), vec3(-5.10, -1.60, 1.24)),
            ("lounge", vec3(-2.55, 0.55, 1.50), vec3(-5.15, 1.55, 0.65)),
            ("desk", vec3(-1.65, 0.45, 1.55), vec3(-0.25, 1.90, 0.90)),
            ("bedroom", vec3(3.9, 0.0, 1.5), vec3(5.5, 2.5, 0.9)),
            ("hall", vec3(2.9, -0.35, 1.55), vec3(-0.5, -0.35, 1.25)),
            ("bathroom", vec3(2.1, -0.70, 1.65), vec3(1.2, -2.0, 0.95)),
            ("ensuite", vec3(1.72, 1.90, 1.65), vec3(0.65, 2.50, 0.8)),
            ("boiler-room", vec3(1.5, -0.15, 1.75), vec3(1.5, 0.85, 1.0)),
            ("spare-room", vec3(3.2, -1.25, 1.55), vec3(4.6, -2.6, 1.2)),
            ("storage-top", vec3(-0.55, -2.35, 1.65), vec3(-1.75, -2.35, 1.05)),
            ("storage-bottom", vec3(-0.55, -1.25, 1.65), vec3(-1.75, -1.25, 1.05)),
            ("overhead", vec3(5.5, 6.0, 8.0), vec3(0.0, 0.0, 0.0)),
        ] {
            if view.as_ref().is_some_and(|v| v != name && v != "all") {
                continue;
            }
            if *lights_off && matches!(name, "plan" | "overhead") {
                continue;
            }
            let started = Instant::now();
            let (pixels, exposure) = pixels(
                &scene,
                eye,
                focus,
                width,
                height,
                shadows.as_ref(),
                exposures.get(name).copied(),
                if name == "plan" {
                    0.65
                } else if name == "boiler-room" {
                    1.65
                } else {
                    FOV
                },
            );
            let image = RgbImage::from_raw(width as u32, height as u32, pixels).unwrap();
            exposures.insert(name, exposure);
            let suffix = if *lights_off { "-lights-off" } else { "" };
            image.save(format!("{destination}/{name}{suffix}.webp"))?;
            println!("{name}: {:?}", started.elapsed());
        }
    }
    Ok(())
}

#[allow(clippy::option_if_let_else, reason = "Keep reference and cached shader calls readable side by side")]
fn pixels(
    scene: &Scene,
    eye: Vec3,
    focus: Vec3,
    width: usize,
    height: usize,
    shadows: Option<&ShadowAtlas>,
    exposure: Option<f32>,
    fov: f32,
) -> (Vec<u8>, f32) {
    let scale = (fov * 0.5).tan();
    let forward = (focus - eye).normalize();
    let right = if forward.z.abs() > 0.999 { Vec3::X } else { Vec3::Z.cross(forward).normalize() };
    let up = forward.cross(right).normalize() * scale;
    let right = right * (width as f32 / height as f32 * scale);
    let exposure = exposure.unwrap_or_else(|| scene.camera_exposure(eye, forward, right, up));
    println!("Camera EV100: {:.2}", (100.0 / (1.2 * exposure)).log2());
    let globals = Globals {
        eye,
        exposure,
        forward,
        right,
        up,
        pixel_angle: fov / height as f32,
        sun: scene.sun,
        sample_index: 0,
        interleaved: false,
        parity: 0,
    };
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
                            scene.grid,
                            &scene.probes,
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
                            scene.grid,
                            &scene.probes,
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
    (pixels, exposure)
}
