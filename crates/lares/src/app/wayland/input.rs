//! Pointer and touch events in the render buffer's coordinate space.
use super::Wayland;
use isthmus::glam::{Vec2, vec2};
use wayland_client::{
    Proxy as _, WEnum, delegate_noop,
    protocol::{
        wl_pointer::{self, WlPointer},
        wl_seat::{self, WlSeat},
        wl_touch::{self, WlTouch},
    },
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};

#[derive(Default)]
pub(super) struct Input {
    cursors: Option<WpCursorShapeManagerV1>,
    cursor: Option<WpCursorShapeDeviceV1>,
    pointer: Option<WlPointer>,
    touch: Option<WlTouch>,
    touch_id: Option<i32>,
    inside: bool,
}

impl Input {
    pub(super) fn new(cursors: Option<WpCursorShapeManagerV1>) -> Self {
        Self { cursors, ..Self::default() }
    }
}

impl Wayland {
    fn move_pointer(&mut self, x: f64, y: f64) {
        let size = Vec2::from_array(self.pixel_size().map(|pixels| pixels as f32));
        let logical = Vec2::from_array(self.logical.map(|points| points as f32));
        if let Some(runtime) = &mut self.runtime {
            runtime.app.pointer_moved(vec2(x as f32, y as f32) * (size / logical), size);
            if runtime.app.is_dragging() {
                self.request_redraw();
            }
        }
    }

    fn press_pointer(&mut self, pressed: bool) {
        if let Some(runtime) = &mut self.runtime {
            if pressed {
                runtime.app.pointer_pressed();
            } else {
                runtime.app.pointer_released();
            }
            self.request_redraw();
        }
    }

    pub(super) fn release_input(&mut self) {
        self.input.touch_id = None;
        self.press_pointer(false);
    }
}

dispatch!(WlSeat, |state, proxy, event, queue| {
    if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
        if capabilities.contains(wl_seat::Capability::Pointer) {
            if state.input.pointer.is_none() {
                let pointer = proxy.get_pointer(queue, ());
                state.input.cursor =
                    state.input.cursors.as_ref().map(|manager| manager.get_pointer(&pointer, queue, ()));
                state.input.pointer = Some(pointer);
            }
        } else if let Some(pointer) = state.input.pointer.take() {
            if let Some(cursor) = state.input.cursor.take() {
                cursor.destroy();
            }
            if pointer.version() >= 3 {
                pointer.release();
            }
            state.input.inside = false;
            if state.input.touch_id.is_none() {
                state.press_pointer(false);
            }
        }
        if capabilities.contains(wl_seat::Capability::Touch) {
            if state.input.touch.is_none() {
                state.input.touch = Some(proxy.get_touch(queue, ()));
            }
        } else if let Some(touch) = state.input.touch.take() {
            if touch.version() >= 3 {
                touch.release();
            }
            if state.input.touch_id.is_some() {
                state.release_input();
            }
        }
    }
});

dispatch!(WlPointer, |state, _proxy, event, _queue| {
    match event {
        wl_pointer::Event::Enter { serial, surface, surface_x, surface_y } if surface == state.window.wl => {
            state.input.inside = true;
            if let Some(cursor) = &state.input.cursor {
                cursor.set_shape(serial, Shape::Default);
            }
            if state.input.touch_id.is_none() {
                state.move_pointer(surface_x, surface_y);
            }
        }
        wl_pointer::Event::Leave { .. } => {
            state.input.inside = false;
            if state.input.touch_id.is_none() {
                state.press_pointer(false);
            }
        }
        event if state.input.inside && state.input.touch_id.is_none() => match event {
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => state.move_pointer(surface_x, surface_y),
            wl_pointer::Event::Button { button: 0x110, state: WEnum::Value(button), .. } => {
                state.press_pointer(button == wl_pointer::ButtonState::Pressed);
            }
            wl_pointer::Event::Axis { axis: WEnum::Value(wl_pointer::Axis::VerticalScroll), value, .. }
                if value != 0.0 =>
            {
                if let Some(runtime) = &mut state.runtime {
                    runtime.app.scrolled(value.signum() as f32);
                    state.request_redraw();
                }
            }
            _ => {}
        },
        _ => {}
    }
});

dispatch!(WlTouch, |state, _proxy, event, _queue| {
    match event {
        wl_touch::Event::Down { id, surface, x, y, .. }
            if state.input.touch_id.is_none()
                && surface == state.window.wl
                && state.runtime.as_ref().is_some_and(|runtime| !runtime.app.is_dragging()) =>
        {
            state.input.touch_id = Some(id);
            state.move_pointer(x, y);
            state.press_pointer(true);
        }
        wl_touch::Event::Motion { id, x, y, .. } if state.input.touch_id == Some(id) => state.move_pointer(x, y),
        wl_touch::Event::Up { id, .. } if state.input.touch_id == Some(id) => state.release_input(),
        wl_touch::Event::Cancel if state.input.touch_id.is_some() => state.release_input(),
        _ => {}
    }
});

delegate_noop!(Wayland: ignore WpCursorShapeDeviceV1);
