use super::*;
use isthmus::{RenderError, Resources, SurfaceHandle, wgpu};
use std::{
    env, fs,
    sync::mpsc,
    time::{Duration, Instant},
};

fn pixels(renderer: &Renderer, image: &Image) -> Vec<u8> {
    let view = renderer.image_view(image).unwrap();
    let texture = view.texture();
    let [width, height] = image.size();
    let stride = (width * 4).next_multiple_of(256);
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("pixel regression"),
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device().create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None },
        },
        texture.size(),
    );
    renderer.queue().submit([encoder.finish()]);
    let (send, recv) = mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        send.send(result).unwrap();
    });
    renderer.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    buffer
        .slice(..)
        .get_mapped_range()
        .unwrap()
        .chunks(stride as usize)
        .flat_map(|row| &row[..width as usize * 4])
        .copied()
        .collect()
}

fn paint(
    renderer: &mut Renderer,
    targets: [SurfaceHandle; 2],
    size: Vec2,
    mut draw: impl FnMut(&mut UiContext, usize),
) -> Result<(), RenderError> {
    let config = Config::default();
    let mut interaction = Interaction::default();
    interaction.enabled = false;
    renderer.render(|render| {
        render.time = 10.0;
        render.delta_time = 0.0;
        for (index, target) in targets.into_iter().enumerate() {
            render.surface(target, size, Globals { bar_height: config.height, ..Globals::default() }, |frame| {
                draw(&mut UiContext { frame, config: &config, interaction: &mut interaction }, index);
            });
        }
    })
}

fn labels(renderer: &mut Renderer, targets: [SurfaceHandle; 2]) {
    paint(renderer, targets, vec2(64.0, 32.0), |ui, index| {
        let panel = Rect::new(Vec2::ZERO, vec2(64.0, 32.0));
        for (x, color) in if index == 0 { vec![(16.0, Vec3::X), (48.0, Vec3::Z)] } else { vec![(32.0, Vec3::Y)] } {
            let line = ui.frame.resources.line("A", 20.0, 700.0).centered(vec2(x, 16.0));
            ui.paint_text(panel, 0.0, line, color.extend(1.0));
        }
    })
    .unwrap();
}

#[test]
fn pixels_survive_batching_multiple_surfaces_and_device_recovery() {
    let mut renderer = pollster::block_on(Renderer::headless(text_cache(), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
    let (first, image) = renderer.add_image([64, 32]);
    let (second, other) = renderer.add_image([128, 64]);
    labels(&mut renderer, [first, second]);
    let before = pixels(&renderer, &image);
    let green = pixels(&renderer, &other);
    assert!(before.as_chunks::<4>().0.iter().any(|p| p[0] > 0));
    assert!(before.as_chunks::<4>().0.iter().any(|p| p[2] > 0));
    assert!(before.as_chunks::<4>().0.iter().all(|p| p[1] == 0));
    assert!(green.as_chunks::<4>().0.iter().any(|p| p[1] > 0));
    assert!(green.as_chunks::<4>().0.iter().all(|p| p[0] == 0 && p[2] == 0));
    let alias = renderer.alias_image(first).unwrap();
    renderer.device().destroy();
    renderer.device().poll(wgpu::PollType::Poll).unwrap();
    assert!(matches!(renderer.render(|_| {}), Err(RenderError::DeviceLost(_))));
    pollster::block_on(renderer.recover()).unwrap();
    labels(&mut renderer, [alias, second]);
    assert_eq!(pixels(&renderer, &image), before);
    assert_eq!(pixels(&renderer, &other), green);
}

#[test]
fn animated_font_axes_reclaim_residency_without_invalidating_live_runs() {
    let mut cache = text_cache();
    let original = cache.shape("Held lyric", 14.0, 700.0);
    let held = original.cached_distance();
    let held = held.cached_distance();
    let line = cache.place(&held, Vec2::ZERO);
    let reference = line.resolve(cache.data()).distance_at(vec2(5.0, 0.0));
    let started = Instant::now();
    let mut peak = 0;
    for index in 0..800 {
        cache.begin_frame();
        let line =
            cache.shape("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", 14.0, 100.0 + index as f32);
        cache.place(&line, Vec2::ZERO);
        peak = peak.max(cache.data().len());
    }
    cache.begin_frame();
    let line = cache.place(&held, Vec2::ZERO);
    assert_eq!(line.resolve(cache.data()).distance_at(vec2(5.0, 0.0)), reference);
    let mut fresh = text_cache();
    let line = fresh.place(&held, Vec2::ZERO);
    assert_eq!(line.resolve(fresh.data()).distance_at(vec2(5.0, 0.0)), reference);
    // Evicting the source run must keep shared fields reusable by later tracks.
    let source = cache.shape("Held lyric", 14.0, 700.0);
    let reused = source.cached_distance();
    let before = cache.data().len();
    cache.place(&held, Vec2::ZERO);
    let placement_words = cache.data().len() - before;
    let before = cache.data().len();
    cache.place(&reused, Vec2::ZERO);
    assert_eq!(cache.data().len() - before, placement_words, "live fields were rebuilt after source-run eviction");
    assert!(peak < 64 * 1024, "obsolete font axes accumulated {peak} words");
    eprintln!("800 animated font weights: {:?}, peak resident bytes: {}", started.elapsed(), peak * 4);
    let started = Instant::now();
    for _ in 0..1000 {
        cache.begin_frame();
        cache.place(&held, Vec2::ZERO);
    }
    eprintln!("Steady glyph packing: {:?}/frame, resource bytes: {}", started.elapsed() / 1000, cache.data().len() * 4);
}

#[test]
fn graph_pixels_follow_captured_history_slices() {
    let mut renderer = pollster::block_on(Renderer::headless(text_cache(), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
    let (flat, flat_image) = renderer.add_image([1024, 80]);
    let (spike, spike_image) = renderer.add_image([1024, 80]);
    let mut panels = [status::StatusPanel::default(), status::StatusPanel::default()];
    for index in 0..32 {
        panels[1].record(status::SystemSample {
            cpu: status::ProcessorSample { temperature: 0.0, usage: f32::from(index == 16), memory: 0.0 },
            gpu: None,
            battery_level: None,
        });
    }
    paint(&mut renderer, [flat, spike], vec2(1024.0, 80.0), |context, index| {
        panels[index].show(context, weathertime::StatusSky::default());
    })
    .unwrap();
    assert!(
        pixels(&renderer, &flat_image) != pixels(&renderer, &spike_image),
        "history data never reached the graph shader"
    );
}

#[test]
#[ignore = "manual release-mode rendering profile"]
fn profile_rendering() {
    let mut renderer = pollster::block_on(Renderer::headless(text_cache(), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
    let (target, _) = renderer.add_image([1920, 96]);
    let config = Config::default();
    let mut interaction = Interaction::default();
    interaction.enabled = false;
    let mut status = status::StatusPanel::default();
    let frames = env::var("CANTUS_PROFILE_FRAMES").map_or(1000, |value| value.parse::<u32>().unwrap());
    let mut recording = Duration::ZERO;
    let mut completion = Duration::ZERO;
    let mut resources = 0;
    for index in 0..frames + 100 {
        let started = Instant::now();
        renderer
            .render(|render| {
                render.time = index as f32 / 60.0;
                render.delta_time = 1.0 / 60.0;
                render.surface(
                    target,
                    vec2(1920.0, 96.0),
                    Globals { bar_height: config.height, ..Globals::default() },
                    |frame| {
                        let mut ui = UiContext { frame, config: &config, interaction: &mut interaction };
                        status.show(&mut ui, weathertime::StatusSky::default());
                        for column in 0..12 {
                            let line =
                                ui.frame.resources.shape("Cantus • Music and lyrics ♪", 14.0, 700.0).cached_distance();
                            let line = ui.frame.resources.place(&line, vec2(column as f32 * 120.0, 32.0));
                            ui.paint_text(Rect::new(Vec2::ZERO, vec2(1440.0, 96.0)), 0.0, line, Vec4::ONE);
                        }
                        resources = ui.frame.resources.data().len() * 4;
                    },
                );
            })
            .unwrap();
        let submitted = Instant::now();
        renderer.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
        if index >= 100 {
            recording += submitted - started;
            completion += submitted.elapsed();
        }
    }
    eprintln!(
        "adapter={}, frames={frames}, recording={:?}/frame, completion={:?}/frame, resource_bytes={resources}",
        renderer.device_name(),
        recording / frames,
        completion / frames
    );
    eprintln!(
        "{}",
        fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
