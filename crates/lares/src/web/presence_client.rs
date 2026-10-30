use crate::{app::App, presence::Snapshot};
use gloo_events::EventListener;
use std::{cell::RefCell, rc::Rc, time::Duration};
use tokio::sync::oneshot;
use wasm_bindgen::JsCast;
use web_time::Instant;

pub(super) async fn run(app: Rc<RefCell<App>>) {
    let Some(window) = web_sys::window() else { return };
    let location = window.location();
    let Ok(host) = location.host() else { return };
    let scheme = if location.protocol().ok().as_deref() == Some("https:") { "wss" } else { "ws" };
    let url = format!("{scheme}://{host}/api/presence");
    loop {
        if let Ok(socket) = web_sys::WebSocket::new(&url) {
            let receiver = Rc::clone(&app);
            let _message = EventListener::new(&socket, "message", move |event| {
                let event = event.unchecked_ref::<web_sys::MessageEvent>();
                if let Some(text) = event.data().as_string()
                    && let Ok(snapshot) = serde_json::from_str::<Snapshot>(&text)
                {
                    receiver.borrow_mut().presence.ingest(snapshot);
                }
            });
            let (sender, closed) = oneshot::channel();
            let sender = Rc::new(RefCell::new(Some(sender)));
            let _closing = ["close", "error"].map(|name| {
                let sender = Rc::clone(&sender);
                EventListener::new(&socket, name, move |_| {
                    if let Some(sender) = sender.borrow_mut().take() {
                        let _ = sender.send(());
                    }
                })
            });
            let _ = closed.await;
            let _ = socket.close();
        }
        app.borrow_mut().presence.ingest(Snapshot::empty(false));
        let retry = Instant::now();
        while retry.elapsed() < Duration::from_secs(3) {
            super::next_frame().await;
        }
    }
}
