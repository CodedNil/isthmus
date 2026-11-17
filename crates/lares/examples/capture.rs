//! `cargo run -p lares --release --example capture -- [directory [width height [view [utc-hour]]]]`
//! Renders go to `assets/renders` by default; that directory is untracked, unlike the photographs in `assets/references`.
//! Uses the same four lighting/antialiasing samples as the live renderer.
//! `LARES_CAPTURE_MOTION=1` renders every moving object at its destination (0 is where it is authored).
use core::f32::consts::PI;
use image::{RgbImage, imageops::overlay};
use isthmus::glam::{Vec2, Vec3, vec2, vec3};
use lares::{
    camera::Camera,
    home::{self, Kind, Material},
    render::{Globals, Scene, SunSettings, pixel, pixel_with, shadows::ShadowAtlas, sun::utc_now},
    views,
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
use tracing::info;

fn main() -> Result<(), Box<dyn Error>> {
    lares::init_logging();
    let mut args = env::args().skip(1);
    let destination = args.next().unwrap_or_else(|| format!("{}/assets/renders", env!("CARGO_MANIFEST_DIR")));
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
    info!("Scene: {:?}", started.elapsed());
    if let Ok(progress) = env::var("LARES_CAPTURE_MOTION") {
        let progress: f32 = progress.parse()?;
        let names: Vec<_> = scene.movers().iter().map(|mover| mover.entity_id).collect();
        for name in names {
            scene.set_motion(name, progress);
        }
    }
    info!(
        "Captures: evening (21:00 UTC) above daytime; daytime fixtures {}; meter {}",
        if env::var("LARES_CAPTURE_LIGHTS").as_deref() == Ok("on") { "on" } else { "off" },
        if env::var("LARES_CAPTURE_METER").as_deref() == Ok("plan") { "full plan" } else { "per view" }
    );
    let luminance = vec3(0.2126, 0.7152, 0.0722);
    info!(
        "Evening daylight: {:.0} lx direct normal, {:.0} lx diffuse horizontal; clear sky, view-metered camera",
        scene.sun.radiance.dot(luminance) * 100.0,
        scene.sun.sky.dot(luminance) * 100.0 * PI * 0.85
    );
    let shadows = env::var("LARES_SHADOW_RESOLUTION").ok().map(|resolution| {
        let started = Instant::now();
        let atlas = ShadowAtlas::bake_cpu(&scene, resolution.parse().expect("shadow resolution must be an integer"));
        info!("Shadow atlas: {:?}", started.elapsed());
        atlas
    });
    let mut evening_images = BTreeMap::new();
    for lights_off in [false, true] {
        if lights_off {
            scene.set_time(timestamp);
            let entities: BTreeSet<_> =
                rooms.iter().flat_map(|room| room.lights.iter().map(|light| light.entity_id)).collect();
            let states: Vec<_> = entities.into_iter().map(|entity| (entity, 0.0)).collect();
            if env::var("LARES_CAPTURE_LIGHTS").as_deref() != Ok("on") {
                scene.set_light_states(&states);
            }
        }
        let mut all = views::all(scene.radius).to_vec();
        if let Ok(spec) = env::var("LARES_CAPTURE_VIEW") {
            for (index, part) in spec.split(';').enumerate() {
                all.push(custom_view(&rooms, part, index)?);
            }
        }
        for view in all {
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
            // Rejects cameras inside a wall before tracing.
            if scene.objects.iter().any(|object| {
                if !matches!(object.kind, Kind::Box | Kind::Ceiling) || object.material != Material::Paint {
                    return false;
                }
                let offset = view.eye - object.pos;
                let local = vec3(
                    offset.x * object.rotation.y + offset.y * object.rotation.x,
                    offset.y * object.rotation.y - offset.x * object.rotation.x,
                    offset.z,
                );
                local.abs().cmplt(object.size * 0.5).all()
            }) {
                return Err(format!("camera {name} is inside architectural geometry").into());
            }
            let started = Instant::now();
            let pixels = pixels(&scene, view, width, height, shadows.as_ref());
            let image = RgbImage::from_raw(width as u32, height as u32, pixels).unwrap();
            if lights_off {
                let mut stacked = RgbImage::new(width as u32, height as u32 * 2);
                overlay(&mut stacked, &evening_images.remove(name).unwrap(), 0, 0);
                overlay(&mut stacked, &image, 0, height as i64);
                stacked.save(format!("{destination}/{name}.webp"))?;
            } else {
                evening_images.insert(name, image);
            }
            info!("{name}: {:?}", started.elapsed());
        }
    }
    Ok(())
}

/// Parses `eye_x,eye_y,eye_z,focus_x,focus_y,focus_z[,fov]` in authored plan coordinates, as the plan is drawn;
/// several views are separated by `;` and saved as `custom0`, `custom1` and so on.
fn custom_view(rooms: &[home::Room], spec: &str, index: usize) -> Result<views::View, Box<dyn Error>> {
    let values: Vec<f32> = spec.split(',').map(str::parse).collect::<Result<_, _>>()?;
    if values.len() < 6 {
        return Err("LARES_CAPTURE_VIEW needs six numbers".into());
    }
    let vertices = rooms.iter().flat_map(|room| room.outline.iter().map(|point| room.position + *point));
    let min = vertices.clone().fold(Vec2::splat(f32::MAX), Vec2::min);
    let max = vertices.fold(Vec2::splat(f32::MIN), Vec2::max);
    let origin = (min + max) * 0.5;
    let world = |x: f32, y: f32, z: f32| vec3(x - origin.x, origin.y - y, z);
    let view = views::View::new(
        Box::leak(format!("custom{index}").into_boxed_str()),
        world(values[0], values[1], values[2]),
        world(values[3], values[4], values[5]),
    );
    Ok(values.get(6).map_or(view, |fov| view.with_fov(*fov)))
}

#[expect(clippy::option_if_let_else, reason = "Keep reference and cached shader calls readable side by side")]
fn pixels(scene: &Scene, view: views::View, width: usize, height: usize, shadows: Option<&ShadowAtlas>) -> Vec<u8> {
    let eye = view.eye;
    let (forward, right, up) = view.rays(width as f32 / height as f32);
    let (exposure, white_balance) = if env::var("LARES_CAPTURE_METER").as_deref() == Ok("plan") {
        let camera = Camera { distance: (scene.radius * 2.0).clamp(6.0, 40.0), ..Default::default() };
        let (eye, forward, right, up) = camera.rays(1.0);
        scene.camera_settings(eye, forward, right, up)
    } else {
        scene.camera_settings(eye, forward, right, up)
    };
    info!("Camera EV100: {:.2}", (100.0 / (1.2 * exposure)).log2());
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
        normal_samples: 4,
        dynamic: scene.dynamic_objects(),
    };
    // Exposure is kept when isolating direct light, so differences measure transport.
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
                        pixel_with::<true>(
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
