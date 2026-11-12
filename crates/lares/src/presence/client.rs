//! Same-origin presence snapshots, with reconnects independent of the render loop.
use super::Snapshot;
use gloo_events::EventListener;
use std::{cell::RefCell, rc::Rc};
use tokio::sync::oneshot;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::Window;

pub(crate) async fn connect(ingest: impl FnMut(Snapshot) + 'static) {
    let ingest = Rc::new(RefCell::new(ingest));
    let Some(window) = web_sys::window() else { return };
    let location = window.location();
    let Ok(host) = location.host() else { return };
    let scheme = if location.protocol().ok().as_deref() == Some("https:") { "wss" } else { "ws" };
    let url = format!("{scheme}://{host}/api/presence");
    loop {
        if let Ok(socket) = web_sys::WebSocket::new(&url) {
            let receiver = Rc::clone(&ingest);
            let _message = EventListener::new(&socket, "message", move |event| {
                let event = event.unchecked_ref::<web_sys::MessageEvent>();
                if let Some(text) = event.data().as_string()
                    && let Ok(snapshot) = serde_json::from_str::<Snapshot>(&text)
                {
                    receiver.borrow_mut()(snapshot);
                }
            });
            let (sender, closed) = oneshot::channel();
            let sender = Rc::new(RefCell::new(Some(sender)));
            let _closing = ["close", "error"].map(|name| {
                let sender = Rc::clone(&sender);
                EventListener::new(&socket, name, move |_| {
                    if let Some(sender) = sender.borrow_mut().take() {
                        sender.send(()).unwrap_or_default();
                    }
                })
            });
            drop(closed.await);
            drop(socket.close());
        }
        ingest.borrow_mut()(Snapshot::empty(false));
        if retry_delay(&window).await.is_err() {
            return;
        }
    }
}

async fn retry_delay(window: &Window) -> Result<(), wasm_bindgen::JsValue> {
    let (sender, elapsed) = oneshot::channel();
    let callback = Closure::once(move || {
        sender.send(()).unwrap_or_default();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.as_ref().unchecked_ref(), 3_000)?;
    drop(elapsed.await);
    Ok(())
}
