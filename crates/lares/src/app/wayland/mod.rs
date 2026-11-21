//! Native xdg-shell window and compositor-paced presentation.
use super::runtime::Runtime;
use calloop::EventLoop;
use input::Input;
use isthmus::{
    WaylandWindow,
    rwh::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle},
};
use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::runtime::Builder;
use wayland_client::{
    Connection, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::WlCallback,
        wl_compositor::WlCompositor,
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
        wl_surface::{self, WlSurface},
    },
};
use wayland_protocols::{
    wp::{
        cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    xdg::{
        decoration::zv1::client::{
            zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
            zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1},
        },
        shell::client::{
            xdg_surface::{self, XdgSurface},
            xdg_toplevel::{self, XdgToplevel},
            xdg_wm_base::{self, XdgWmBase},
        },
    },
};

macro_rules! dispatch {
    ($proxy:ty, |$state:ident, $object:ident, $event:ident, $queue:ident| $body:block) => {
        impl wayland_client::Dispatch<$proxy, ()> for Wayland {
            fn event(
                $state: &mut Self,
                $object: &$proxy,
                $event: <$proxy as wayland_client::Proxy>::Event,
                (): &(),
                _connection: &wayland_client::Connection,
                $queue: &wayland_client::QueueHandle<Self>,
            ) $body
        }
    };
}

mod input;

pub fn run() -> Result<(), Box<dyn Error>> {
    let background = Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
    let _background = background.enter();
    let connection = Connection::connect_to_env()?;
    let (globals, events) = registry_queue_init::<Wayland>(&connection)?;
    let queue = events.handle();
    let compositor: WlCompositor = globals.bind(&queue, 6..=6, ())?;
    let shell: XdgWmBase = globals.bind(&queue, 1..=6, ())?;
    let _seat: Option<WlSeat> = globals.bind(&queue, 1..=8, ()).ok();
    let wl = compositor.create_surface(&queue, ());
    let surface = shell.get_xdg_surface(&wl, &queue, ());
    let toplevel = surface.get_toplevel(&queue, ());
    toplevel.set_title("Lares".to_owned());
    toplevel.set_app_id("lares".to_owned());
    let decoration = globals.bind::<ZxdgDecorationManagerV1, _, _>(&queue, 1..=1, ()).ok().map(|manager| {
        let decoration = manager.get_toplevel_decoration(&toplevel, &queue, ());
        decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        manager.destroy();
        decoration
    });
    let scaling = globals
        .bind::<WpViewporter, _, _>(&queue, 1..=1, ())
        .ok()
        .zip(globals.bind::<WpFractionalScaleManagerV1, _, _>(&queue, 1..=1, ()).ok())
        .map(|(viewporter, manager)| {
            let viewport = viewporter.get_viewport(&wl, &queue, ());
            let fractional = manager.get_fractional_scale(&wl, &queue, ());
            viewporter.destroy();
            manager.destroy();
            (viewport, fractional)
        });
    wl.commit();

    let mut app = Wayland {
        window: Arc::new(NativeWindow { native: WaylandWindow::new(wl), surface, toplevel, scaling, decoration }),
        runtime: None,
        input: Input::new(globals.bind(&queue, 1..=1, ()).ok()),
        logical: [1280, 800],
        pending: Configure::default(),
        integer_scale: 1,
        fractional_scale: None,
        frame_callback: None,
        status: WindowStatus::Configuring,
        activated: false,
    };

    let mut event_loop = EventLoop::try_new()?;
    calloop_wayland_source::WaylandSource::new(connection, events).insert(event_loop.handle())?;
    while app.status != WindowStatus::Closed {
        if let Some(wait) = app.wait()
            && wait.is_zero()
        {
            app.draw(&queue)?;
        }
        event_loop.dispatch(app.wait(), &mut app)?;
    }
    Ok(())
}

pub(super) struct NativeWindow {
    native: WaylandWindow,
    surface: XdgSurface,
    toplevel: XdgToplevel,
    scaling: Option<(WpViewport, WpFractionalScaleV1)>,
    decoration: Option<ZxdgToplevelDecorationV1>,
}

impl HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.native.display_handle()
    }
}

impl HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.native.window_handle()
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        if let Some(decoration) = &self.decoration {
            decoration.destroy();
        }
        if let Some((viewport, fractional)) = &self.scaling {
            fractional.destroy();
            viewport.destroy();
        }
        self.toplevel.destroy();
        self.surface.destroy();
    }
}

struct Wayland {
    runtime: Option<Runtime>,
    window: Arc<NativeWindow>,
    input: Input,
    logical: [i32; 2],
    pending: Configure,
    integer_scale: i32,
    fractional_scale: Option<u32>,
    frame_callback: Option<WlCallback>,
    status: WindowStatus,
    activated: bool,
}

#[derive(PartialEq, Eq)]
enum WindowStatus {
    Configuring,
    Visible,
    Suspended,
    Closed,
}

#[derive(Default)]
struct Configure {
    size: [i32; 2],
    suspended: bool,
    activated: bool,
}

impl Wayland {
    fn wait(&self) -> Option<Duration> {
        if self.status != WindowStatus::Visible || self.frame_callback.is_some() {
            return None;
        }
        Some(
            self.runtime
                .as_ref()
                .map_or(Duration::ZERO, |runtime| runtime.next_frame.saturating_duration_since(Instant::now())),
        )
    }

    fn request_redraw(&mut self) {
        if let Some(runtime) = &mut self.runtime {
            runtime.next_frame = Instant::now();
        }
    }

    fn pixel_size(&self) -> [u32; 2] {
        self.logical.map(|size| {
            self.fractional_scale.map_or_else(
                || size.saturating_mul(self.integer_scale) as u32,
                |scale| ((u64::from(size as u32) * u64::from(scale) + 60) / 120).max(1) as u32,
            )
        })
    }

    fn draw(&mut self, queue: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        let size = self.pixel_size();
        let window = &self.window;
        if self.fractional_scale.is_some()
            && let Some((viewport, _)) = &window.scaling
        {
            viewport.set_destination(self.logical[0], self.logical[1]);
            window.native.wl.set_buffer_scale(1);
        } else {
            window.native.wl.set_buffer_scale(self.integer_scale);
        }
        if self.runtime.is_none() {
            self.runtime = Some(Runtime::new(Arc::clone(window), size)?);
        }
        self.frame_callback = Some(window.native.wl.frame(queue, ()));
        if !self.runtime.as_mut().expect("runtime initialized").draw(window, size)? {
            // A recovered surface can be unmapped until the next frame is presented.
            self.frame_callback = None;
        }
        // Also submit the callback when Vulkan skipped a presentation.
        window.native.wl.commit();
        Ok(())
    }
}

dispatch!(XdgWmBase, |_state, proxy, event, _queue| {
    if let xdg_wm_base::Event::Ping { serial } = event {
        proxy.pong(serial);
    }
});

dispatch!(XdgToplevel, |state, _proxy, event, _queue| {
    match event {
        xdg_toplevel::Event::Configure { width, height, states } => {
            let has = |value| states.as_chunks::<4>().0.contains(&u32::to_ne_bytes(value));
            state.pending = Configure {
                size: [width, height],
                suspended: has(xdg_toplevel::State::Suspended as u32),
                activated: has(xdg_toplevel::State::Activated as u32),
            };
        }
        xdg_toplevel::Event::Close => state.status = WindowStatus::Closed,
        _ => {}
    }
});

dispatch!(XdgSurface, |state, proxy, event, _queue| {
    if let xdg_surface::Event::Configure { serial } = event {
        proxy.ack_configure(serial);
        if state.status == WindowStatus::Closed {
            return;
        }
        for (size, suggested) in state.logical.iter_mut().zip(state.pending.size) {
            if suggested > 0 {
                *size = suggested;
            }
        }
        proxy.set_window_geometry(0, 0, state.logical[0], state.logical[1]);
        if state.activated && !state.pending.activated {
            state.release_input();
        }
        state.activated = state.pending.activated;
        state.status = if state.pending.suspended { WindowStatus::Suspended } else { WindowStatus::Visible };
        state.request_redraw();
    }
});

dispatch!(WlCallback, |state, proxy, _event, _queue| {
    if state.frame_callback.as_ref() == Some(proxy) {
        state.frame_callback = None;
    }
});

dispatch!(WlSurface, |state, _proxy, event, _queue| {
    if let wl_surface::Event::PreferredBufferScale { factor } = event
        && factor > 0
        && factor != state.integer_scale
    {
        state.integer_scale = factor;
        if state.fractional_scale.is_none() {
            state.request_redraw();
        }
    }
});

dispatch!(WpFractionalScaleV1, |state, _proxy, event, _queue| {
    if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
        && scale > 0
        && state.fractional_scale != Some(scale)
    {
        state.fractional_scale = Some(scale);
        state.request_redraw();
    }
});

delegate_noop!(Wayland: ignore WlCompositor);
delegate_noop!(Wayland: ignore WpViewporter);
delegate_noop!(Wayland: ignore WpViewport);
delegate_noop!(Wayland: ignore WpFractionalScaleManagerV1);
delegate_noop!(Wayland: ignore WpCursorShapeManagerV1);
delegate_noop!(Wayland: ignore ZxdgDecorationManagerV1);
delegate_noop!(Wayland: ignore ZxdgToplevelDecorationV1);

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for Wayland {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
