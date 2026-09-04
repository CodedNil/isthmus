//! Browser implementation of the shared Cantus platform services.
//!
//! The browser cannot inspect native CPU/GPU/battery state or speaker output,
//! so those services use plausible demo values. Weather location uses the
//! browser's permission-gated approximate geolocation API.

use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering, mpsc::Sender},
    time::Duration,
};

use gloo_timers::future::TimeoutFuture;
use js_sys::Date;
use reqwest::Client;
use tokio::sync::mpsc::UnboundedSender;
use wasm_bindgen::{JsCast, closure::Closure};

use super::{DesktopApp, Platform};
use crate::{
    app::{AppUpdater, Background},
    render::status::{AudioMonitor, ProcessorSample, SystemSample},
};

/// Entry point used by the generated browser glue.
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|panic| {
        web_sys::console::error_1(&format!("Cantus panic: {panic}").into());
    }));
    Platform::run();
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

    /// Browsers do not expose speaker output without an explicit capture
    /// stream, so keep the status visual useful with a small demo spectrum.
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

    pub fn start_status_monitor(updates: Sender<SystemSample>, audio: Arc<AudioMonitor>) {
        let start = Date::now();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let time = ((Date::now() - start) / 1_000.0) as f32;
                if updates.send(Self::sample_status(time)).is_err() {
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

    pub fn populate_app_icons(_apps: &mut [DesktopApp]) {}

    pub fn decode_icon(_bytes: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub async fn provider_icon(_http: Client, _url: String) -> Option<Vec<u8>> {
        None
    }

    pub fn start_exchange_rates(_background: &Background, _http: Client, update: fn(HashMap<String, f64>)) {
        update(HashMap::from([
            (String::from("EUR"), 0.92),
            (String::from("GBP"), 0.79),
            (String::from("JPY"), 149.0),
        ]));
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
            exec: String::new(),
            comment: comment.into(),
            icon_path: None,
            action: None,
            icon: None,
        })
        .collect()
    }

    pub fn spawn(_exec: &str) {}

    pub fn open_url(url: &str) {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
    }

    pub fn start_launcher_listener(_background: &Background, _updater: &AppUpdater) {}

    pub fn trigger_launcher() -> ! {
        panic!("Cantus launcher triggering is unavailable in a browser")
    }

    /// Starts the web data layer. Frame presentation is kept behind the
    /// isthmus canvas adapter, which currently only accepts native Vulkan/
    /// SPIR-V surfaces; the page shell still provides the eventual mount
    /// point without pretending to have a browser renderer today.
    pub fn run() {
        let _app = Box::leak(Box::new(crate::app::CantusApp::default()));
    }
}
