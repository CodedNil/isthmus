#![expect(clippy::print_stderr, reason = "Timing measurements are printed for humans")]
use super::*;
use isthmus::{Resources, wgpu};
use std::{
    env, fs,
    time::{Duration, Instant},
};

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
