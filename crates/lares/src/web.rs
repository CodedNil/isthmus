use crate::{
    app::App,
    render::{self, Renderer, SunSettings, shadows::ShadowAtlas, sun::utc_now, temporal::Accumulation},
};
use gloo_events::EventListener;
use gloo_render::request_animation_frame;
use isthmus::{
    glam::{Vec2, vec2, vec3},
    wgpu::{DeviceType, Queue, SurfaceTarget, TextureFormat},
};
use std::{
    array::from_fn,
    cell::{Cell, RefCell},
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

mod presence_client;

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

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    fn laresProgress(stage: &str);

    // web-sys' stable MouseEvent bindings return i32 and truncate slow drags.
    #[wasm_bindgen::prelude::wasm_bindgen(extends = web_sys::PointerEvent, js_name = PointerEvent)]
    type PrecisePointer;
    #[wasm_bindgen::prelude::wasm_bindgen(method, getter, js_name = clientX)]
    fn precise_x(this: &PrecisePointer) -> f64;
    #[wasm_bindgen::prelude::wasm_bindgen(method, getter, js_name = clientY)]
    fn precise_y(this: &PrecisePointer) -> f64;
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
    wasm_bindgen_futures::spawn_local(presence_client::run(Rc::clone(&app)));
    laresProgress("Loading SDF light visibility…");
    let shadows = ShadowAtlas::decode(&app.borrow().scene, &visibility)?;
    drop(visibility);
    let (mut samples, mut images) = sample_images(&mut gpu, physical);
    let mut visibility = gpu.add_image_format(physical, TextureFormat::Rgba16Float);
    let mut materials = gpu.add_image_format(physical, TextureFormat::Rgba16Float);
    laresProgress("Drawing the first view…");
    draw_sample(
        &mut gpu,
        &app.borrow(),
        surface,
        to_vec2(physical),
        &shadows,
        &samples,
        &images,
        &visibility,
        &materials,
        0,
        1,
        true,
    )?;
    laresProgress("Waiting for the first frame…");
    gpu_finished(gpu.queue(), &device_lost).await?;
    next_frame().await;
    laresProgress("ready");

    let active_pointer = Rc::new(Cell::new(None));
    let _pointer_listeners =
        ["pointerdown", "pointerup", "pointermove", "pointercancel", "lostpointercapture"].map(|name| {
            let app = Rc::clone(&app);
            let capture = canvas.clone();
            let active_pointer = Rc::clone(&active_pointer);
            EventListener::new(&canvas, name, move |event| {
                let event = event.unchecked_ref::<web_sys::PointerEvent>();
                let precise: &PrecisePointer = event.unchecked_ref();
                let position = vec2(precise.precise_x() as f32, precise.precise_y() as f32);
                let viewport = vec2(capture.client_width() as f32, capture.client_height() as f32);
                let mut app = app.borrow_mut();
                match name {
                    "pointerdown" if event.button() == 0 && active_pointer.get().is_none() => {
                        active_pointer.set(Some(event.pointer_id()));
                        let _ = capture.set_pointer_capture(event.pointer_id());
                        app.pointer_moved(position, viewport);
                        app.pointer_pressed();
                    }
                    "pointerup" if active_pointer.get() == Some(event.pointer_id()) => {
                        app.pointer_moved(position, viewport);
                        app.pointer_released();
                        active_pointer.set(None);
                        let _ = capture.release_pointer_capture(event.pointer_id());
                    }
                    "pointercancel" | "lostpointercapture" if active_pointer.get() == Some(event.pointer_id()) => {
                        app.pointer_released();
                        active_pointer.set(None);
                    }
                    "pointermove" if active_pointer.get() == Some(event.pointer_id()) => {
                        app.pointer_moved(position, viewport);
                    }
                    _ => {}
                }
            })
        });

    let wheel_app = Rc::clone(&app);
    let _wheel = EventListener::new(&canvas, "wheel", move |event| {
        let event = event.unchecked_ref::<web_sys::WheelEvent>();
        event.prevent_default();
        wheel_app.borrow_mut().scrolled(event.delta_y().signum() as f32);
    });

    let mut last_size = physical;
    let mut last_render_size = physical;
    let mut last_camera = app.borrow().camera;
    let mut last_lighting = app.borrow().scene.lighting_revision;
    let mut accumulation = Accumulation::new(interleaved(to_vec2(physical)));
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
            let _ = canvas.set_attribute("data-presence-count", &presence_state.0.to_string());
            let _ = canvas.set_attribute("data-presence-connected", if presence_state.1 { "true" } else { "false" });
            last_presence = presence_state;
        }
        app.borrow_mut().scene.update_time(utc_now());
        let camera_changed = app.borrow().camera != last_camera;
        let moving = app.borrow().is_dragging();
        // Keep a fixed full-resolution grid throughout a gesture. Reallocating
        // history and switching interleaving mid-drag caused visible jumps.
        let render_size = physical;
        let changed = camera_changed
            || render_size != last_render_size
            || physical != last_size
            || app.borrow().scene.lighting_revision != last_lighting;
        // Full-resolution samples converge to 4× spatial AA over four frames.
        // Discard old views immediately on camera/lighting changes. Target
        // 60 FPS during interaction/convergence;
        // keep refreshing slow animation at up to 8 Hz once converged.
        let scene_active = changed || !accumulation.converged();
        let active = render::MOTION_PROBE || moving || scene_active || presence_moving;
        let redraw_scene = scene_active || timestamp >= next_scene_refresh;
        let interval = if active { 1.0 / 60.0 } else { 1.0 / 8.0 };
        let gap = if active { 0.0 } else { frame_seconds * 4.0 };
        // Schedule on the browser's refresh clock, not completion callbacks:
        // Keep input paced by presentation, without adding a delay after GPU completion.
        if !active && (timestamp + 0.001 < next_presented || completed.elapsed().as_secs_f64() < gap) {
            continue;
        }
        gpu.resize(surface, physical);
        if render_size != last_render_size {
            for sample in samples {
                gpu.remove_surface(sample);
            }
            (samples, images) = sample_images(&mut gpu, render_size);
            gpu.remove_surface(visibility.0);
            visibility = gpu.add_image_format(render_size, TextureFormat::Rgba16Float);
            gpu.remove_surface(materials.0);
            materials = gpu.add_image_format(render_size, TextureFormat::Rgba16Float);
            accumulation = Accumulation::new(interleaved(to_vec2(render_size)));
        }
        let (phase, mask) = if redraw_scene { accumulation.advance(changed) } else { (0, accumulation.valid_mask()) };
        let count = mask.count_ones();
        if count != sample_count {
            // Nonvisual diagnostic used by the running-clock browser checks.
            canvas
                .set_attribute("data-samples", &count.to_string())
                .map_err(|error| format!("could not record sample count: {error:?}"))?;
            sample_count = count;
        }
        let started = Instant::now();
        {
            let app = app.borrow();
            draw_sample(
                &mut gpu,
                &app,
                surface,
                to_vec2(physical),
                &shadows,
                &samples,
                &images,
                &visibility,
                &materials,
                phase,
                mask,
                redraw_scene,
            )?;
            last_camera = app.camera;
            last_lighting = app.scene.lighting_revision;
        }
        if redraw_scene {
            next_scene_refresh = timestamp + 1.0 / 8.0;
        }
        last_size = physical;
        last_render_size = render_size;
        // Never queue another ray-traced frame before the previous one has
        // finished. Input events can still update the next camera meanwhile.
        gpu_finished(gpu.queue(), &device_lost).await?;
        frame_seconds = started.elapsed().as_secs_f64();
        completed = Instant::now();
        next_presented = (next_presented + interval).max(timestamp + interval * 0.5);
    }
}

fn sample_images(gpu: &mut Renderer, size: [u32; 2]) -> ([isthmus::SurfaceHandle; 4], [isthmus::Image; 4]) {
    let pairs: [(isthmus::SurfaceHandle, isthmus::Image); 4] = from_fn(|_| gpu.add_image(size));
    (pairs.each_ref().map(|pair| pair.0), pairs.map(|pair| pair.1))
}

fn draw_sample(
    gpu: &mut Renderer,
    app: &App,
    surface: isthmus::SurfaceHandle,
    size: Vec2,
    shadows: &ShadowAtlas,
    samples: &[isthmus::SurfaceHandle; 4],
    images: &[isthmus::Image; 4],
    visibility: &(isthmus::SurfaceHandle, isthmus::Image),
    materials: &(isthmus::SurfaceHandle, isthmus::Image),
    phase: u32,
    mask: u32,
    redraw_scene: bool,
) -> Result<(), isthmus::RenderError> {
    gpu.render(|render| {
        let render_size = Vec2::from_array(images[0].size().map(|value| value as f32));
        let checker = interleaved(render_size);
        let sample = if checker { phase / 2 } else { phase };
        if redraw_scene {
            app.draw(render, samples[sample as usize], render_size, shadows, visibility, materials, phase, checker);
        }
        render.surface(surface, size, app.globals(size), |mut frame| {
            render::resolve(&mut frame, images, mask, checker, &app.presence.ghosts);
        });
    })
}

fn interleaved(size: Vec2) -> bool {
    size.x * size.y > 2_500_000.0
}

async fn gpu_finished(queue: &Queue, device_lost: &AtomicBool) -> Result<(), Box<dyn Error>> {
    let (sender, ready) = oneshot::channel();
    queue.on_submitted_work_done(move || {
        let _ = sender.send(());
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
        let _ = sender.send(timestamp);
    });
    (animation, frame)
}

async fn next_frame() -> f64 {
    let (_animation, frame) = animation_frame();
    frame.await.unwrap_or_default()
}
