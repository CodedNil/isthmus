use crate::{
    app::App,
    render::{Renderer, sun::utc_now},
};
use gloo_events::EventListener;
use gloo_render::request_animation_frame;
use isthmus::{glam::vec2, wgpu::SurfaceTarget};
use std::{cell::RefCell, error::Error, panic, rc::Rc};
use tokio::sync::oneshot;
use wasm_bindgen::JsCast;
use web_sys::console;

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    fn laresProgress(stage: &str);
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    panic::set_hook(Box::new(|panic| {
        console::error_1(&format!("Lares panic: {panic}").into());
    }));
    run().await.map_err(|error| error.to_string().into())
}

async fn run() -> Result<(), Box<dyn Error>> {
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
    let app = Rc::new(RefCell::new(App::template()));
    laresProgress("Preparing graphics…");
    next_frame().await;
    let physical = physical_size();
    let (mut gpu, surface) = Renderer::new(SurfaceTarget::Canvas(canvas.clone()), physical, ()).await?;
    laresProgress("Drawing the first view…");
    gpu.render(|render| app.borrow().draw(render, surface, to_vec2(physical)))?;
    let (sender, ready) = oneshot::channel();
    gpu.queue().on_submitted_work_done(move || {
        let _ = sender.send(());
    });
    ready.await?;
    next_frame().await;
    laresProgress("ready");

    let _pointer_listeners = ["pointerdown", "pointerup", "pointermove", "pointercancel"].map(|name| {
        let app = Rc::clone(&app);
        let capture = canvas.clone();
        EventListener::new(&canvas, name, move |event| {
            let event = event.unchecked_ref::<web_sys::PointerEvent>();
            let position = vec2(event.offset_x() as f32, event.offset_y() as f32);
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

    loop {
        next_frame().await;
        let physical = physical_size();
        gpu.resize(surface, physical);
        let mut app = app.borrow_mut();
        app.scene.update_time(utc_now());
        gpu.render(|render| app.draw(render, surface, to_vec2(physical)))?;
    }
}

async fn next_frame() {
    let (sender, frame) = oneshot::channel();
    let _animation = request_animation_frame(move |_| {
        let _ = sender.send(());
    });
    let _ = frame.await;
}
