//! Browser services and permission-gated geolocation.

use super::{DesktopApp, Platform};
use crate::{
    app::{AppUpdater, Background, send_update},
    interaction::InputEvent,
    render::{
        Renderer, TEXT_COLOR,
        launcher::LauncherKey,
        status::{AudioMonitor, ProcessorSample, SystemSample},
    },
};
use gloo_events::{EventListener, EventListenerOptions};
use gloo_render::request_animation_frame;
use gloo_timers::future::TimeoutFuture;
use isthmus::glam::vec2;
use std::{
    cell::RefCell,
    future::Future,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use wasm_bindgen::{JsCast, closure::Closure};
use web_time::Instant;

pub trait Task = Future + 'static;

impl Background {
    pub(crate) fn spawn(&self, task: impl Task<Output = ()>) {
        wasm_bindgen_futures::spawn_local(task);
    }
}

/// Entry point used by the generated browser glue.
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|panic| {
        web_sys::console::error_1(&format!("Cantus panic: {panic}").into());
    }));
    wasm_bindgen_futures::spawn_local(async {
        if let Err(error) = run_web().await {
            web_sys::console::error_1(&format!("Cantus could not start: {error}").into());
        }
    });
}

impl Platform {
    fn sample_status(time: f32) -> SystemSample {
        let wave = |period: f32| (time / period).sin();
        SystemSample {
            cpu: ProcessorSample {
                temperature: 49.0 + wave(7.0) * 7.0,
                usage: 0.34 + wave(5.0) * 0.16,
                memory: 0.46 + wave(11.0) * 0.05,
            },
            gpu: Some(ProcessorSample {
                temperature: 55.0 + wave(9.8) * 6.0,
                usage: 0.27 + wave(6.5) * 0.14,
                memory: 0.38 + wave(13.0) * 0.04,
            }),
            battery_level: Some(0.78),
        }
    }

    /// Uses a demo spectrum because browsers require explicit speaker capture.
    fn sample_audio(time: f32, audio: &AudioMonitor) {
        let level = 0.52 + (time * 1.7).sin() * 0.18;
        audio.volume.store(level.to_bits(), Ordering::Relaxed);
        for (index, band) in audio.spectrum.iter().enumerate() {
            let phase = time * (1.2 + index as f32 * 0.15) + index as f32 * 0.8;
            band.store((0.18 + phase.sin().abs() * 0.62).to_bits(), Ordering::Relaxed);
        }
    }
}

impl Platform {
    pub const STATUS_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

    pub fn start_status_monitor(updates: AppUpdater, audio: Arc<AudioMonitor>) {
        let start = Instant::now();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let time = start.elapsed().as_secs_f32();
                if !send_update(&updates, move |app| {
                    if let Some(status) = &mut app.bar.status {
                        status.record(Self::sample_status(time));
                    }
                }) {
                    break;
                }
                Self::sample_audio(time, &audio);
                TimeoutFuture::new(Self::STATUS_SAMPLE_INTERVAL.as_millis() as u32).await;
            }
        });
    }

    pub fn start_location_monitor(_background: &Background, updates: UnboundedSender<[f32; 2]>) {
        let Some(geolocation) = web_sys::window().and_then(|window| window.navigator().geolocation().ok()) else {
            return;
        };
        let success = Closure::once(move |position: web_sys::Position| {
            let coordinates = position.coords();
            let _ = updates.send([coordinates.latitude() as f32, coordinates.longitude() as f32]);
        });
        let _ = geolocation.get_current_position(success.as_ref().unchecked_ref());
        success.forget();
    }

    pub async fn sleep(duration: Duration) {
        TimeoutFuture::new(duration.as_millis() as u32).await;
    }

    pub fn set_volume(_volume: f32) {}

    pub fn run_power_action(_background: &Background, _action: usize) {}

    pub fn desktop_apps() -> Vec<DesktopApp> {
        [
            ("Example Notes", "A small example application"),
            ("Example Calendar", "A browser calendar example"),
            ("Example Files", "A browser file manager example"),
            ("Example Mail", "A browser mail example"),
            ("Example Terminal", "A browser terminal example"),
        ]
        .into_iter()
        .map(|(name, comment)| DesktopApp {
            name: name.into(),
            exec: Vec::new(),
            comment: comment.into(),
            action: None,
            icon: None,
        })
        .collect()
    }

    pub fn spawn(_command: &[String]) {}

    pub fn open_url(url: &str) {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
    }

    pub fn start_launcher_listener(_background: &Background, _updater: &AppUpdater) {}

    pub fn trigger_launcher() -> ! {
        panic!("Cantus launcher triggering is unavailable in a browser")
    }

    pub fn run() {}
}

async fn run_web() -> Result<(), String> {
    let window = web_sys::window().ok_or("browser window is unavailable")?;
    let canvas = window
        .document()
        .and_then(|document| document.get_element_by_id("cantus"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or("#cantus is not a canvas")?;
    let app = Rc::new(RefCell::new(crate::app::CantusApp::default()));
    app.borrow_mut().launcher.open = true;
    let logical_size = || {
        [
            window.inner_width().ok().and_then(|value| value.as_f64()).unwrap_or(1.0) as f32,
            window.inner_height().ok().and_then(|value| value.as_f64()).unwrap_or(1.0) as f32,
        ]
    };
    let [width, height] = logical_size();
    let scale = window.device_pixel_ratio() as f32;
    let (mut gpu, surface) = Renderer::new(
        canvas.clone(),
        [(width * scale).round() as u32, (height * scale).round() as u32],
        include_bytes!("../../../../assets/NotoSans-Variable.ttf"),
        TEXT_COLOR,
    )
    .await
    .map_err(|error| error.to_string())?;

    let _pointer =
        ["pointerenter", "pointermove", "pointerdown", "pointerup", "pointerleave", "pointercancel"].map(|name| {
            let app = Rc::clone(&app);
            EventListener::new(&canvas, name, move |event| {
                let event = event.unchecked_ref::<web_sys::PointerEvent>();
                let position = vec2(event.client_x() as f32, event.client_y() as f32);
                let mut app = app.borrow_mut();
                let input = match name {
                    "pointerdown" if event.button() == 0 => {
                        app.interaction.apply(InputEvent::Enter(position));
                        InputEvent::Press
                    }
                    "pointerup" if event.button() == 0 => {
                        app.interaction.apply(InputEvent::Motion(position));
                        InputEvent::Release
                    }
                    "pointerenter" => InputEvent::Enter(position),
                    "pointerleave" => InputEvent::Leave,
                    "pointercancel" => InputEvent::CancelDrag,
                    "pointermove" => InputEvent::Motion(position),
                    _ => return,
                };
                app.interaction.apply(input);
            })
        });

    let wheel_app = Rc::clone(&app);
    let _wheel = EventListener::new_with_options(
        &canvas,
        "wheel",
        EventListenerOptions::enable_prevent_default(),
        move |event| {
            let event = event.unchecked_ref::<web_sys::WheelEvent>();
            event.prevent_default();
            wheel_app.borrow_mut().interaction.apply(InputEvent::Scroll(event.delta_y().signum() as i32));
        },
    );

    let key_app = Rc::clone(&app);
    let _key = EventListener::new_with_options(
        &window,
        "keydown",
        EventListenerOptions::enable_prevent_default(),
        move |event| {
            let event = event.unchecked_ref::<web_sys::KeyboardEvent>();
            let mut app = key_app.borrow_mut();
            if event.ctrl_key() && event.key() == "k" {
                app.launcher.toggle();
                app.interaction = Default::default();
                event.prevent_default();
                return;
            }
            if !app.launcher.open {
                return;
            }
            let command = match event.key().as_str() {
                "Escape" => Some(LauncherKey::Escape),
                "Enter" => Some(LauncherKey::Activate),
                "ArrowUp" => Some(LauncherKey::Up),
                "ArrowDown" => Some(LauncherKey::Down),
                "Backspace" => Some(LauncherKey::Backspace),
                "Delete" => Some(LauncherKey::Delete),
                "ArrowLeft" => Some(LauncherKey::Left),
                "ArrowRight" => Some(LauncherKey::Right),
                "Home" => Some(LauncherKey::Home),
                "End" => Some(LauncherKey::End),
                "a" if event.ctrl_key() => Some(LauncherKey::SelectAll),
                "c" if event.ctrl_key() => Some(LauncherKey::Copy),
                "x" if event.ctrl_key() => Some(LauncherKey::Cut),
                _ => None,
            };
            if let Some(command) = command {
                app.launcher.key(command, event.shift_key());
                if !app.launcher.open {
                    app.interaction = Default::default();
                }
                event.prevent_default();
            } else if !event.ctrl_key() && event.key().chars().count() == 1 {
                let value = event.key();
                app.launcher.edit(|field| field.insert(&value));
            }
        },
    );
    let paste_app = Rc::clone(&app);
    let _paste = EventListener::new_with_options(
        &window,
        "paste",
        EventListenerOptions::enable_prevent_default(),
        move |event| {
            let event = event.unchecked_ref::<web_sys::ClipboardEvent>();
            let mut app = paste_app.borrow_mut();
            if !app.launcher.open {
                return;
            }
            if let Some(data) = event.clipboard_data()
                && let Ok(text) = data.get_data("text/plain")
            {
                event.prevent_default();
                app.launcher.edit(|field| field.insert(&text.replace(['\n', '\r'], " ")));
            }
        },
    );

    loop {
        let [width, height] = logical_size();
        let scale = window.device_pixel_ratio() as f32;
        let physical = [(width * scale).round() as u32, (height * scale).round() as u32];
        if canvas.width() != physical[0] || canvas.height() != physical[1] {
            canvas.set_width(physical[0]);
            canvas.set_height(physical[1]);
            gpu.resize(surface, physical);
        }
        {
            let mut app = app.borrow_mut();
            app.apply_pending_updates();
            gpu.render(|render| {
                render.surface(surface, vec2(width, height), |frame| app.draw(frame, true, true));
            })
            .map_err(|error| error.to_string())?;
            if let Some(text) = app.launcher.pending_copy.take() {
                let _ = window.navigator().clipboard().write_text(&text);
            }
        }
        let (sender, frame) = oneshot::channel();
        let _animation = request_animation_frame(move |_| {
            let _ = sender.send(());
        });
        let _ = frame.await;
    }
}
