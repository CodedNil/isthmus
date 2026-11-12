//! App startup, JavaScript controls and the paced GPU render loop.
use crate::{
    app::App,
    presence,
    render::{
        Renderer,
        light::{
            shadows::ShadowAtlas,
            sun::{SunSettings, utc_now},
        },
        pipeline::{targets::Targets, temporal::Accumulation},
    },
};
use gloo_render::request_animation_frame;
use isthmus::{
    glam::{vec2, vec3},
    wgpu::{DeviceType, Queue, SurfaceTarget},
};
use std::{
    cell::RefCell,
    error::Error,
    panic,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::oneshot;
use wasm_bindgen::JsCast;
use web_sys::console;
use web_time::Instant;

thread_local! {
    static APPLICATION: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_light_state(entity: &str, brightness: f32) -> bool {
    APPLICATION.with(|application| {
        application.borrow().as_ref().is_some_and(|app| app.borrow_mut().scene.set_light_state(entity, brightness))
    })
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_light_color(entity: &str, red: f32, green: f32, blue: f32) -> bool {
    APPLICATION.with(|application| {
        application
            .borrow()
            .as_ref()
            .is_some_and(|app| app.borrow_mut().scene.set_light_color(entity, vec3(red, green, blue)))
    })
}

/// Read-only diagnostics for matching GPU captures to reference renders.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn render_diagnostics() -> String {
    APPLICATION.with(|application| {
        let application = application.borrow();
        let Some(app) = application.as_ref() else {
            return String::new();
        };
        let app = app.borrow();
        let globals = app.globals(vec2(1280.0, 800.0));
        serde_json::json!({
            "eye": globals.eye.to_array(),
            "exposure": globals.exposure,
            "whiteBalance": globals.white_balance.to_array(),
            "sunDirection": globals.sun.direction.to_array(),
            "sunRadiance": globals.sun.radiance.to_array(),
            "sky": globals.sun.sky.to_array(),
            "probes": app.scene.probes.len(),
            "surfaceCharts": app.scene.objects.iter().filter(|item| item.surface_cache != u32::MAX).count(),
        })
        .to_string()
    })
}

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    fn laresProgress(stage: &str);

}

#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn start(visibility: Vec<u8>, lighting: Vec<u8>) -> Result<(), wasm_bindgen::JsValue> {
    panic::set_hook(Box::new(|panic| {
        console::error_1(&format!("Lares panic: {panic}").into());
    }));
    run(visibility, lighting).await.map_err(|error| error.to_string().into())
}

async fn run(visibility: Vec<u8>, lighting: Vec<u8>) -> Result<(), Box<dyn Error>> {
    let window = web_sys::window().ok_or("browser window is unavailable")?;
    let canvas = window
        .document()
        .and_then(|document| document.get_element_by_id("lares"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or("#lares is not a canvas")?;
    let physical_size = || {
        let scale = window.device_pixel_ratio();
        [window.inner_width(), window.inner_height()]
            .map(|length| (length.ok().and_then(|value| value.as_f64()).unwrap_or(1.0) * scale).round().max(1.0) as u32)
    };
    let to_vec2 = |[width, height]: [u32; 2]| vec2(width as f32, height as f32);
    laresProgress("Preparing graphics…");
    next_frame().await;
    let physical = physical_size();
    let (mut gpu, surface) = Renderer::new(SurfaceTarget::Canvas(canvas.clone()), physical, ()).await?;
    let device_lost = Arc::new(AtomicBool::new(false));
    let lost = Arc::clone(&device_lost);
    gpu.device().set_device_lost_callback(move |reason, message| {
        lost.store(true, Ordering::Relaxed);
        console::error_1(&format!("Lares graphics device lost ({reason:?}): {message}").into());
    });
    let adapter = gpu.adapter_info();
    console::info_1(&format!("Lares graphics adapter: {adapter:?}").into());
    if adapter.device_type == DeviceType::Cpu {
        return Err("Only CPU software graphics is available. Lares needs hardware GPU acceleration. Enable browser graphics acceleration and restart the browser. If your GPU driver was recently updated, restart the computer before reloading Lares.".into());
    }
    laresProgress("Preparing the scene…");
    next_frame().await;
    let defaults = SunSettings::default();
    let setting = |name, fallback| {
        canvas
            .get_attribute(name)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(fallback)
    };
    let settings = SunSettings {
        latitude: setting("data-latitude", defaults.latitude),
        longitude: setting("data-longitude", defaults.longitude),
        north_degrees: setting("data-north-degrees", defaults.north_degrees),
    };
    let app = Rc::new(RefCell::new(App::with_baked_lighting(settings, &lighting)?));
    drop(lighting);
    APPLICATION.with(|application| *application.borrow_mut() = Some(Rc::clone(&app)));
    let receiver = Rc::clone(&app);
    wasm_bindgen_futures::spawn_local(presence::connect(move |snapshot| {
        receiver.borrow_mut().presence.ingest(snapshot);
    }));
    laresProgress("Loading SDF light visibility…");
    let shadows = ShadowAtlas::decode(&app.borrow().scene, &visibility)?;
    drop(visibility);
    let mut targets = Targets::new(&mut gpu, physical);
    laresProgress("Drawing the first view…");
    targets.record(&mut gpu, &app.borrow(), &shadows, (surface, to_vec2(physical)), 0, 1, true)?;
    laresProgress("Waiting for the first frame…");
    gpu_finished(gpu.queue(), &device_lost).await?;
    next_frame().await;
    laresProgress("ready");

    let _input = super::input::listen(Rc::clone(&app), &canvas);

    let mut last_size = physical;
    let mut last_render_size = physical;
    let mut last_camera = app.borrow().camera;
    let mut last_lighting = app.borrow().scene.lighting_revision;
    let mut accumulation = Accumulation::new(Targets::interleaved(physical));
    canvas.set_attribute("data-samples", "1").map_err(|error| format!("could not record sample count: {error:?}"))?;
    let mut sample_count = 1;
    let mut completed = Instant::now();
    let mut frame_seconds = 0.0f64;
    let mut next_presented = f64::NEG_INFINITY;
    let mut next_scene_refresh = f64::NEG_INFINITY;
    let mut last_presence = (usize::MAX, false);
    let mut scheduled = animation_frame();
    loop {
        let (_animation, ready) = scheduled;
        let timestamp = ready.await.unwrap_or_default() * 0.001;
        // Request the next refresh before waiting for this frame’s GPU work.
        scheduled = animation_frame();
        if window.document().is_some_and(|document| document.hidden()) {
            continue;
        }
        let physical = physical_size();
        let presence_moving = app.borrow_mut().presence.advance();
        let presence_state = {
            let app = app.borrow();
            (app.presence.ghosts.len(), app.presence.connected)
        };
        if presence_state != last_presence {
            drop(canvas.set_attribute("data-presence-count", &presence_state.0.to_string()));
            drop(canvas.set_attribute("data-presence-connected", if presence_state.1 { "true" } else { "false" }));
            last_presence = presence_state;
        }
        app.borrow_mut().scene.update_time(utc_now());
        let camera_changed = app.borrow().camera != last_camera;
        let moving = app.borrow().is_dragging();
        // The grid stays fixed through a gesture, since reallocating mid-drag jumps visibly.
        let render_size = physical;
        let changed = camera_changed
            || render_size != last_render_size
            || physical != last_size
            || app.borrow().scene.lighting_revision != last_lighting;
        // Four samples converge to 4× AA and reset on camera or lighting changes; 60 FPS while active, 8 Hz once converged.
        let scene_active = changed || !accumulation.converged();
        let active = moving || scene_active || presence_moving;
        let redraw_scene = scene_active || timestamp >= next_scene_refresh;
        let interval = if active { 1.0 / 60.0 } else { 1.0 / 8.0 };
        let gap = if active { 0.0 } else { frame_seconds * 4.0 };
        // Paced on the browser's refresh clock, not on GPU completion.
        if !active && (timestamp + 0.001 < next_presented || completed.elapsed().as_secs_f64() < gap) {
            continue;
        }
        gpu.resize(surface, physical);
        if render_size != last_render_size {
            targets = targets.resized(&mut gpu, render_size);
            accumulation = Accumulation::new(Targets::interleaved(render_size));
        }
        let (phase, mask) = if redraw_scene { accumulation.advance(changed) } else { (0, accumulation.valid_mask()) };
        let count = mask.count_ones();
        if count != sample_count {
            // Read by the browser checks.
            canvas
                .set_attribute("data-samples", &count.to_string())
                .map_err(|error| format!("could not record sample count: {error:?}"))?;
            sample_count = count;
        }
        let started = Instant::now();
        {
            let app = app.borrow();
            targets.record(&mut gpu, &app, &shadows, (surface, to_vec2(physical)), phase, mask, redraw_scene)?;
            last_camera = app.camera;
            last_lighting = app.scene.lighting_revision;
        }
        if redraw_scene {
            next_scene_refresh = timestamp + 1.0 / 8.0;
        }
        last_size = physical;
        last_render_size = render_size;
        // One ray-traced frame in flight at a time.
        gpu_finished(gpu.queue(), &device_lost).await?;
        frame_seconds = started.elapsed().as_secs_f64();
        completed = Instant::now();
        next_presented = (next_presented + interval).max(timestamp + interval * 0.5);
    }
}

async fn gpu_finished(queue: &Queue, device_lost: &AtomicBool) -> Result<(), Box<dyn Error>> {
    let (sender, ready) = oneshot::channel();
    queue.on_submitted_work_done(move || {
        sender.send(()).unwrap_or_default();
    });
    ready.await?;
    if device_lost.load(Ordering::Relaxed) {
        return Err(
            "The graphics device was lost. Restart the browser and check GPU acceleration before reloading Lares."
                .into(),
        );
    }
    Ok(())
}

fn animation_frame() -> (gloo_render::AnimationFrame, oneshot::Receiver<f64>) {
    let (sender, frame) = oneshot::channel();
    let animation = request_animation_frame(move |timestamp| {
        sender.send(timestamp).unwrap_or_default();
    });
    (animation, frame)
}

async fn next_frame() -> f64 {
    let (_animation, frame) = animation_frame();
    frame.await.unwrap_or_default()
}
