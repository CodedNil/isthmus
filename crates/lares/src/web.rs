use crate::{app::App, render::Renderer};
use gloo_events::EventListener;
use gloo_render::request_animation_frame;
use isthmus::{glam::vec2, wgpu::SurfaceTarget};
use std::{cell::RefCell, rc::Rc};
use tokio::sync::oneshot;
use wasm_bindgen::JsCast;

#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    std::panic::set_hook(Box::new(|panic| {
        web_sys::console::error_1(&format!("Lares panic: {panic}").into());
    }));
    run().await.map_err(|error| error.to_string().into())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let window = web_sys::window().ok_or("browser window is unavailable")?;
    let canvas = window
        .document()
        .and_then(|document| document.get_element_by_id("lares"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or("#lares is not a canvas")?;
    let dimensions = || {
        let viewport = vec2(
            window.inner_width().ok().and_then(|value| value.as_f64()).unwrap_or(1.0) as f32,
            window.inner_height().ok().and_then(|value| value.as_f64()).unwrap_or(1.0) as f32,
        );
        let scale = window.device_pixel_ratio() as f32;
        let physical = [(viewport.x * scale).round().max(1.0) as u32, (viewport.y * scale).round().max(1.0) as u32];
        (vec2(physical[0] as f32, physical[1] as f32), physical)
    };
    let (_, physical) = dimensions();
    let (mut gpu, surface) = Renderer::new(SurfaceTarget::Canvas(canvas.clone()), physical, ()).await?;
    let app = Rc::new(RefCell::new(App::template()));

    let _pointer_listeners = ["pointerdown", "pointerup", "pointermove", "pointercancel"].map(|name| {
        let app = Rc::clone(&app);
        let capture = canvas.clone();
        EventListener::new(&canvas, name, move |event| {
            let event = event.unchecked_ref::<web_sys::PointerEvent>();
            let position = vec2(event.offset_x() as f32, event.offset_y() as f32);
            let mut app = app.borrow_mut();
            match name {
                "pointerdown" if event.button() == 0 => {
                    let _ = capture.set_pointer_capture(event.pointer_id());
                    app.pointer_moved(position);
                    app.pointer_pressed();
                }
                "pointerup" if event.button() == 0 => app.pointer_released(),
                "pointercancel" => app.pointer_released(),
                "pointermove" => app.pointer_moved(position),
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
        let (sender, frame) = oneshot::channel();
        let _animation = request_animation_frame(move |time| {
            let _ = sender.send(time);
        });
        let Ok(_) = frame.await else { break };
        let (size, physical) = dimensions();
        gpu.resize(surface, physical);
        let app = app.borrow();
        gpu.render(|render| app.draw(render, surface, size))?;
    }
    Ok(())
}
