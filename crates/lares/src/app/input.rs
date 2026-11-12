//! Canvas pointer capture and wheel input for the app.
use super::App;
use gloo_events::EventListener;
use isthmus::glam::vec2;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    // web-sys' stable MouseEvent bindings return i32 and truncate slow drags.
    #[wasm_bindgen::prelude::wasm_bindgen(extends = web_sys::PointerEvent, js_name = PointerEvent)]
    type PrecisePointer;
    #[wasm_bindgen::prelude::wasm_bindgen(method, getter, js_name = clientX)]
    fn precise_x(this: &PrecisePointer) -> f64;
    #[wasm_bindgen::prelude::wasm_bindgen(method, getter, js_name = clientY)]
    fn precise_y(this: &PrecisePointer) -> f64;
}

/// Keep the returned listeners alive for the lifetime of the app.
pub(super) fn listen(app: Rc<RefCell<App>>, canvas: &HtmlCanvasElement) -> Vec<EventListener> {
    let active_pointer = Rc::new(Cell::new(None));
    let pointer_listeners =
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
                        drop(capture.set_pointer_capture(event.pointer_id()));
                        app.pointer_moved(position, viewport);
                        app.pointer_pressed();
                    }
                    "pointerup" if active_pointer.get() == Some(event.pointer_id()) => {
                        app.pointer_moved(position, viewport);
                        app.pointer_released();
                        active_pointer.set(None);
                        drop(capture.release_pointer_capture(event.pointer_id()));
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
    let wheel = EventListener::new(&canvas, "wheel", move |event| {
        let event = event.unchecked_ref::<web_sys::WheelEvent>();
        event.prevent_default();
        wheel_app.borrow_mut().scrolled(event.delta_y().signum() as f32);
    });

    let mut listeners = Vec::from(pointer_listeners);
    listeners.push(wheel);
    listeners
}
