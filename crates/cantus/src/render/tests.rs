#![expect(clippy::print_stderr, reason = "Timing measurements are printed for humans")]
use super::*;
use isthmus::{Arena, wgpu};
use std::{
    env, fs,
    time::{Duration, Instant},
};
#[test]
fn text_is_shaped_with_kerning_and_marks() {
    install_fonts();
    let width = |text: &str| Line::new(text, 20.0, 400.0).text.width;
    assert!(width("AV") < width("A") + width("V"), "kerning pairs tighten");
    assert!((width("e\u{301}") - width("e")).abs() < 1e-3, "a combining mark adds no advance");
}

#[test]
fn animated_font_axes_reclaim_residency_without_invalidating_live_runs() {
    install_fonts();
    let mut arena = Arena::default();
    let held = Line::new("Held lyric", 14.0, 700.0).cached_distance();
    let sample = |arena: &mut Arena, line: &Line| {
        let placed = line.placed(arena, Vec2::ZERO);
        Text::on(placed, &arena.shared).distance_at(vec2(5.0, 0.0))
    };
    let reference = sample(&mut arena, &held);
    let started = Instant::now();
    let mut peak = 0;
    for index in 0..800 {
        arena.begin_frame();
        let line =
            Line::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", 14.0, 100.0 + index as f32);
        line.placed(&mut arena, Vec2::ZERO);
        peak = peak.max(arena.shared.len());
    }
    arena.begin_frame();
    assert_eq!(sample(&mut arena, &held), reference);
    assert_eq!(sample(&mut Arena::default(), &held), reference);
    // Evicting the source run must keep shared fields reusable by later tracks.
    let reused = Line::new("Held lyric", 14.0, 700.0).cached_distance();
    let before = arena.shared.len();
    held.placed(&mut arena, Vec2::ZERO);
    // A line placed twice in a frame is written once, and an equal one reuses the fields and writes only its records.
    assert_eq!(arena.shared.len(), before, "a line already placed this frame was written again");
    reused.placed(&mut arena, Vec2::ZERO);
    let records = reused.text.count as usize * 5 + 4;
    assert!(arena.shared.len() - before <= records, "live fields were rebuilt after source-run eviction");
    assert!(peak < 64 * 1024, "obsolete font axes accumulated {peak} words");
    eprintln!("800 animated font weights: {:?}, peak resident bytes: {}", started.elapsed(), peak * 4);
    let started = Instant::now();
    for _ in 0..1000 {
        arena.begin_frame();
        held.placed(&mut arena, Vec2::ZERO);
    }
    eprintln!("Steady glyph packing: {:?}/frame, resource bytes: {}", started.elapsed() / 1000, arena.shared.len() * 4);
}

#[test]
#[ignore = "manual release-mode rendering profile"]
fn profile_rendering() {
    install_fonts();
    let mut renderer = pollster::block_on(Renderer::headless(TextureFormat::Rgba8Unorm)).unwrap();
    let (target, _) = renderer.add_image([1920, 96]);
    let config = Config::default();
    let mut interaction = Interaction::default();
    interaction.enabled = false;
    let mut status = status::StatusPanel::default();
    let frames = env::var("CANTUS_PROFILE_FRAMES").map_or(1000, |value| value.parse::<u32>().unwrap());
    let mut recording = Duration::ZERO;
    let mut completion = Duration::ZERO;
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
                            let line = Line::new("Cantus • Music and lyrics ♪", 14.0, 700.0).cached_distance();
                            let text = line.at(vec2(column as f32 * 120.0, 32.0));
                            ui.paint_text(Rect::new(Vec2::ZERO, vec2(1440.0, 96.0)), 0.0, text, Vec4::ONE);
                        }
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
        "adapter={}, frames={frames}, recording={:?}/frame, completion={:?}/frame",
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
