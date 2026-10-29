use crate::{
    app::App,
    render::{
        self, Renderer, SunSettings,
        shadows::ShadowAtlas,
        sun::utc_now,
        temporal::{Accumulation, MotionResolution},
    },
};
use gloo_events::EventListener;
use gloo_render::request_animation_frame;
use isthmus::{
    glam::{Vec2, vec2, vec3},
    wgpu::{Queue, SurfaceTarget},
};
use std::{array::from_fn, cell::RefCell, error::Error, panic, rc::Rc};
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

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    fn laresProgress(stage: &str);

    // web-sys' stable MouseEvent bindings return i32 and truncate slow drags.
    #[wasm_bindgen::prelude::wasm_bindgen(extends = web_sys::PointerEvent, js_name = PointerEvent)]
    type PrecisePointer;
    #[wasm_bindgen::prelude::wasm_bindgen(method, getter, js_name = offsetX)]
    fn precise_x(this: &PrecisePointer) -> f64;
    #[wasm_bindgen::prelude::wasm_bindgen(method, getter, js_name = offsetY)]
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
    laresProgress("Preparing graphics…");
    next_frame().await;
    let physical = physical_size();
    let (mut gpu, surface) = Renderer::new(SurfaceTarget::Canvas(canvas.clone()), physical, ()).await?;
    laresProgress("Loading SDF light visibility…");
    let shadows = ShadowAtlas::decode(&app.borrow().scene, &visibility)?;
    drop(visibility);
    let (mut samples, mut images) = sample_images(&mut gpu, physical);
    laresProgress("Drawing the first view…");
    draw_sample(&mut gpu, &app.borrow(), surface, to_vec2(physical), &shadows, &samples, &images, 0, 1)?;
    gpu_finished(gpu.queue()).await?;
    next_frame().await;
    laresProgress("ready");

    let _pointer_listeners = ["pointerdown", "pointerup", "pointermove", "pointercancel"].map(|name| {
        let app = Rc::clone(&app);
        let capture = canvas.clone();
        EventListener::new(&canvas, name, move |event| {
            let event = event.unchecked_ref::<web_sys::PointerEvent>();
            let precise: &PrecisePointer = event.unchecked_ref();
            let position = vec2(precise.precise_x() as f32, precise.precise_y() as f32);
            let viewport = vec2(capture.client_width() as f32, capture.client_height() as f32);
            let mut app = app.borrow_mut();
            match name {
                "pointerdown" if event.button() == 0 => {
                    let _ = capture.set_pointer_capture(event.pointer_id());
                    app.pointer_moved(position, viewport);
                    app.pointer_pressed();
                }
                "pointerup" if event.button() == 0 => app.pointer_released(),
                "pointercancel" => app.pointer_released(),
                "pointermove" => app.pointer_moved(position, viewport),
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
    let mut motion_resolution = MotionResolution::default();
    let mut last_motion: Option<Instant> = None;
    let mut last_camera = app.borrow().camera;
    let mut last_lighting = app.borrow().scene.lighting_revision;
    let mut accumulation = Accumulation::new(interleaved(to_vec2(physical)));
    canvas.set_attribute("data-samples", "1").map_err(|error| format!("could not record sample count: {error:?}"))?;
    let mut sample_count = 1;
    let mut completed = Instant::now();
    let mut frame_seconds = 0.0f64;
    let mut next_presented = f64::NEG_INFINITY;
    loop {
        let timestamp = next_frame().await * 0.001;
        if window.document().is_some_and(|document| document.hidden()) {
            continue;
        }
        let physical = physical_size();
        app.borrow_mut().scene.update_time(utc_now());
        let camera_changed = app.borrow().camera != last_camera;
        if camera_changed {
            last_motion = Some(Instant::now());
        }
        let moving = last_motion.is_some_and(|time| time.elapsed().as_secs_f32() < 0.20);
        let render_size = if moving { motion_resolution.extent(physical) } else { physical };
        let changed = camera_changed
            || render_size != last_render_size
            || physical != last_size
            || app.borrow().scene.lighting_revision != last_lighting;
        // Full-resolution samples converge to 4× spatial AA over four frames.
        // Discard old views immediately on camera/lighting changes. Target
        // 30 FPS during interaction/convergence, with desktop GPU headroom;
        // keep refreshing slow animation at up to 8 Hz once converged.
        let active = changed || !accumulation.converged();
        let interval = if active { 1.0 / 30.0 } else { 1.0 / 8.0 };
        let gap = if active { (frame_seconds - interval).max(0.0) * 0.5 } else { frame_seconds * 4.0 };
        // Schedule on the browser's refresh clock, not completion callbacks:
        // callback jitter must not turn a 30 Hz target into alternating 20 Hz.
        if timestamp + 0.001 < next_presented || completed.elapsed().as_secs_f64() < gap {
            continue;
        }
        gpu.resize(surface, physical);
        if render_size != last_render_size {
            for sample in samples {
                gpu.remove_surface(sample);
            }
            (samples, images) = sample_images(&mut gpu, render_size);
            accumulation = Accumulation::new(interleaved(to_vec2(render_size)));
        }
        let (phase, mask) = accumulation.advance(changed);
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
            draw_sample(&mut gpu, &app, surface, to_vec2(physical), &shadows, &samples, &images, phase, mask)?;
            last_camera = app.camera;
            last_lighting = app.scene.lighting_revision;
        }
        last_size = physical;
        last_render_size = render_size;
        // Never queue another ray-traced frame before the previous one has
        // finished. Input events can still update the next camera meanwhile.
        gpu_finished(gpu.queue()).await?;
        frame_seconds = started.elapsed().as_secs_f64();
        if moving {
            motion_resolution.observe(frame_seconds as f32);
        }
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
    phase: u32,
    mask: u32,
) -> Result<(), isthmus::RenderError> {
    gpu.render(|render| {
        let render_size = Vec2::from_array(images[0].size().map(|value| value as f32));
        let checker = interleaved(render_size);
        let sample = if checker { phase / 2 } else { phase };
        app.draw(render, samples[sample as usize], render_size, shadows, phase, checker);
        render.surface(surface, size, app.globals(size), |mut frame| {
            render::resolve(&mut frame, images, mask, checker);
        });
    })
}

fn interleaved(size: Vec2) -> bool {
    size.x * size.y > 2_500_000.0
}

async fn gpu_finished(queue: &Queue) -> Result<(), oneshot::error::RecvError> {
    let (sender, ready) = oneshot::channel();
    queue.on_submitted_work_done(move || {
        let _ = sender.send(());
    });
    ready.await
}

async fn next_frame() -> f64 {
    let (sender, frame) = oneshot::channel();
    let _animation = request_animation_frame(move |timestamp| {
        let _ = sender.send(timestamp);
    });
    frame.await.unwrap_or_default()
}
