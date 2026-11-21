use super::LayerShellApp;
use std::{
    fs::File,
    io::{self, Write as _},
    os::fd::AsFd as _,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::unix::pipe::{Receiver, Sender},
    task::spawn_blocking,
    time::timeout,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop, event_created_child,
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::WlDataDeviceManager,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_seat::WlSeat,
    },
};

const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct Clipboard {
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    pub(super) serial: Option<u32>,
    pending: Option<Offer>,
    selection: Option<Offer>,
}

struct Offer {
    proxy: WlDataOffer,
    mime: Option<usize>,
}

impl Drop for Offer {
    fn drop(&mut self) {
        self.proxy.destroy();
    }
}

impl Clipboard {
    pub(super) fn new(manager: WlDataDeviceManager, seat: &WlSeat, queue: &QueueHandle<LayerShellApp>) -> Self {
        let device = manager.get_data_device(seat, queue, ());
        Self { manager, device, serial: None, pending: None, selection: None }
    }

    pub(super) fn load(&self) -> Option<Receiver> {
        let offer = self.selection.as_ref()?;
        let mime = TEXT_TYPES[offer.mime?].to_owned();
        let (reader, writer) = io::pipe().ok()?;
        let reader = Receiver::from_owned_fd(reader.into()).ok()?;
        offer.proxy.receive(mime, writer.as_fd());
        Some(reader)
    }

    pub(super) fn store(&self, text: String, queue: &QueueHandle<LayerShellApp>) {
        let Some(serial) = self.serial else { return };
        let source = self.manager.create_data_source(queue, Arc::<str>::from(text));
        for mime in TEXT_TYPES {
            source.offer(mime.to_owned());
        }
        self.device.set_selection(Some(&source), serial);
    }
}

pub(super) async fn read(mut reader: Receiver) -> Option<String> {
    let mut text = String::new();
    timeout(TRANSFER_TIMEOUT, reader.read_to_string(&mut text)).await.ok()?.ok()?;
    Some(text)
}

impl Dispatch<WlDataDevice, ()> for LayerShellApp {
    event_created_child!(LayerShellApp, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);

    fn event(
        state: &mut Self,
        _proxy: &WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let Some(clipboard) = &mut state.clipboard else { return };
        match event {
            wl_data_device::Event::DataOffer { id } => clipboard.pending = Some(Offer { proxy: id, mime: None }),
            wl_data_device::Event::Selection { id } => {
                clipboard.selection = clipboard.pending.take().filter(|offer| id.as_ref() == Some(&offer.proxy));
            }
            wl_data_device::Event::Enter { .. } => {
                // Discard drag-and-drop offers. Only clipboard text is handled here.
                clipboard.pending = None;
            }
            _ => {}
        }
    }
}

dispatch!(LayerShellApp, WlDataOffer, |state, proxy, event, _queue| {
    if let wl_data_offer::Event::Offer { mime_type } = event
        && let Some(offer) = state.clipboard.as_mut().and_then(|clipboard| clipboard.pending.as_mut())
        && offer.proxy == *proxy
        && let Some(rank) = TEXT_TYPES.iter().position(|mime| *mime == mime_type)
    {
        offer.mime = Some(offer.mime.map_or(rank, |best| best.min(rank)));
    }
});

dispatch!(LayerShellApp, WlDataSource, Arc<str>, text, |_state, proxy, event, _queue| {
    match event {
        wl_data_source::Event::Send { fd, .. } => {
            let text = Arc::clone(text);
            if let Ok(mut writer) = fd.try_clone().and_then(Sender::from_owned_fd) {
                tokio::spawn(async move {
                    let _transfer = timeout(TRANSFER_TIMEOUT, writer.write_all(text.as_bytes())).await;
                });
            } else {
                // The receiver can also supply a file descriptor other than a pipe.
                spawn_blocking(move || {
                    let _transfer = File::from(fd).write_all(text.as_bytes());
                });
            }
        }
        wl_data_source::Event::Cancelled => proxy.destroy(),
        _ => {}
    }
});

delegate_noop!(LayerShellApp: ignore WlDataDeviceManager);
