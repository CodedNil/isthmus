use crate::render::{Renderer, bamboo::Bamboo};
use isthmus::{
    SurfaceHandle,
    glam::Vec2,
    wgpu::rwh::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, WaylandWindowHandle,
        WindowHandle,
    },
};
use std::{
    collections::BTreeMap,
    ptr::NonNull,
    sync::Arc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    backend::Backend,
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::{self, WlCallback},
        wl_compositor::WlCompositor,
        wl_output::WlOutput,
        wl_region::WlRegion,
        wl_registry::{self, WlRegistry},
        wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{self, WpFractionalScaleV1},
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
};

macro_rules! dispatch {
    ($app:ty, $proxy:ty, $data_type:ty, $data:ident,
        |$state:ident, $object:ident, $value:ident, $queue:ident| $body:block) => {
        impl wayland_client::Dispatch<$proxy, $data_type> for $app {
            fn event(
                $state: &mut Self,
                $object: &$proxy,
                $value: <$proxy as wayland_client::Proxy>::Event,
                $data: &$data_type,
                _conn: &wayland_client::Connection,
                $queue: &wayland_client::QueueHandle<Self>,
            ) $body
        }
    };
}

struct NativeSurface {
    backend: Backend,
    wl: WlSurface,
}

impl HasDisplayHandle for NativeSurface {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}

impl HasWindowHandle for NativeSurface {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let pointer = NonNull::new(self.wl.id().as_ptr().cast()).ok_or(HandleError::Unavailable)?;
        // SAFETY: The wl_surface and its display remain alive until this owner is dropped.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer))) })
    }
}

impl Drop for NativeSurface {
    fn drop(&mut self) {
        self.wl.destroy();
    }
}

type OutputId = u32;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

struct OutputSurface {
    output: WlOutput,
    native: Arc<NativeSurface>,
    layer: ZwlrLayerSurfaceV1,
    fractional: WpFractionalScaleV1,
    viewport: WpViewport,
    size: Vec2,
    scale: f32,
    frame_callback: Option<WlCallback>,
    render_surface: Option<SurfaceHandle>,
    last_render: Option<Instant>,
}

struct Wallpaper {
    compositor: WlCompositor,
    shell: ZwlrLayerShellV1,
    viewporter: WpViewporter,
    fractional: WpFractionalScaleManagerV1,
    backend: Backend,
    renderer: Option<Renderer>,
    bamboo: Bamboo,
    outputs: BTreeMap<OutputId, OutputSurface>,
}

impl Wallpaper {
    fn add_output(&mut self, id: OutputId, output: WlOutput, qhandle: &QueueHandle<Self>) {
        if self.outputs.contains_key(&id) {
            return;
        }
        let wl = self.compositor.create_surface(qhandle, id);
        let layer =
            self.shell.get_layer_surface(&wl, Some(&output), Layer::Background, format!("paries-{id}"), qhandle, id);
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        let input = self.compositor.create_region(qhandle, ());
        wl.set_input_region(Some(&input));
        input.destroy();
        let viewport = self.viewporter.get_viewport(&wl, qhandle, ());
        let fractional = self.fractional.get_fractional_scale(&wl, qhandle, id);
        wl.commit();
        self.outputs.insert(id, OutputSurface {
            output,
            viewport,
            fractional,
            native: Arc::new(NativeSurface { backend: self.backend.clone(), wl }),
            layer,
            size: Vec2::ZERO,
            scale: 1.0,
            frame_callback: None,
            render_surface: None,
            last_render: None,
        });
    }

    fn remove_output(&mut self, id: OutputId) {
        let Some(output) = self.outputs.remove(&id) else { return };
        if let Some(renderer) = &mut self.renderer
            && let Some(surface) = output.render_surface
        {
            renderer.remove_surface(surface);
        }
        output.output.release();
    }

    fn draw(&mut self, id: OutputId, qhandle: &QueueHandle<Self>) {
        let Some(output) = self.outputs.get_mut(&id) else { return };
        if output.size.min_element() <= 0.0 {
            return;
        }
        if output.last_render.is_some_and(|last| last.elapsed() < FRAME_INTERVAL) {
            output.request_frame(id, qhandle);
            return;
        }
        output.viewport.set_destination(output.size.x as i32, output.size.y as i32);
        let size = (output.size * output.scale).to_array().map(|size| size.round().max(1.0) as u32);
        if output.render_surface.is_none() {
            let render_surface = if let Some(renderer) = &mut self.renderer {
                renderer
                    .add_surface(Arc::clone(&output.native), size)
                    .expect("wallpaper surface is incompatible with the renderer")
            } else {
                let (renderer, surface) = pollster::block_on(Renderer::new(Arc::clone(&output.native), size, ()))
                    .expect("failed to initialize wallpaper renderer");
                eprintln!("Paries is rendering with {}", renderer.device_name());
                self.renderer = Some(renderer);
                surface
            };
            output.render_surface = Some(render_surface);
        }
        let render_surface = output.render_surface.unwrap();
        let renderer = self.renderer.as_mut().unwrap();
        renderer.resize(render_surface, size);
        if let Err(error) = renderer.render(|render| {
            render.surface(render_surface, output.size, (), |mut frame| {
                self.bamboo.show(&mut frame);
            });
        }) {
            eprintln!("Paries could not render output {id}: {error}");
        }
        output.last_render = Some(Instant::now());
        output.request_frame(id, qhandle);
    }
}

impl OutputSurface {
    fn request_frame(&mut self, id: OutputId, qhandle: &QueueHandle<Wallpaper>) {
        if self.frame_callback.is_none() {
            self.frame_callback = Some(self.native.wl.frame(qhandle, id));
        }
        self.native.wl.commit();
    }
}

/// Runs one layer-shell background surface for every Wayland output.
pub fn run() {
    let connection = Connection::connect_to_env().expect("failed to connect to Wayland");
    let (globals, mut events) = registry_queue_init::<Wallpaper>(&connection).expect("failed to read Wayland globals");
    let qhandle = events.handle();
    let mut app = Wallpaper {
        compositor: globals.bind(&qhandle, 6..=7, ()).expect("missing wl_compositor v6"),
        shell: globals.bind(&qhandle, 4..=4, ()).expect("missing zwlr_layer_shell_v1"),
        viewporter: globals.bind(&qhandle, 1..=1, ()).expect("missing wp_viewporter"),
        fractional: globals.bind(&qhandle, 1..=1, ()).expect("missing wp_fractional_scale_manager_v1"),
        backend: connection.backend(),
        renderer: None,
        bamboo: Bamboo::default(),
        outputs: BTreeMap::new(),
    };
    let registry = globals.registry();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            assert!(global.version >= 4, "missing wl_output v4");
            let output = registry.bind(global.name, 4, &qhandle, global.name);
            app.add_output(global.name, output, &qhandle);
        }
    }
    connection.flush().expect("failed to create wallpaper surfaces");
    loop {
        events.blocking_dispatch(&mut app).expect("Wayland dispatch failed");
    }
}

dispatch!(Wallpaper, ZwlrLayerSurfaceV1, OutputId, data, |state, _proxy, event, qhandle| {
    match event {
        zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
            if let Some(output) = state.outputs.get_mut(data) {
                let first = output.size.min_element() <= 0.0;
                output.layer.ack_configure(serial);
                // Zero leaves the corresponding dimension to the client.
                if width > 0 {
                    output.size.x = width as f32;
                }
                if height > 0 {
                    output.size.y = height as f32;
                }
                if first {
                    eprintln!(
                        "Configured output {} at {}x{} scale {}",
                        data, output.size.x, output.size.y, output.scale
                    );
                }
            }
            state.draw(*data, qhandle);
        }
        zwlr_layer_surface_v1::Event::Closed => state.remove_output(*data),
        _ => {}
    }
});

dispatch!(Wallpaper, WlCallback, OutputId, data, |state, proxy, event, qhandle| {
    if matches!(event, wl_callback::Event::Done { .. })
        && let Some(output) = state.outputs.get_mut(data)
        && output.frame_callback.as_ref().is_some_and(|callback| callback.id() == proxy.id())
    {
        output.frame_callback.take();
        state.draw(*data, qhandle);
    }
});

dispatch!(Wallpaper, WpFractionalScaleV1, OutputId, data, |state, proxy, event, queue| {
    if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
        && let Some(output) = state.outputs.get_mut(data)
        && output.fractional == *proxy
    {
        output.scale = scale as f32 / 120.0;
        state.draw(*data, queue);
    }
});
dispatch!(Wallpaper, WlOutput, OutputId, _data, |_state, _proxy, _event, _queue| {});

dispatch!(Wallpaper, WlRegistry, GlobalListContents, _data, |state, proxy, event, qhandle| {
    match event {
        wl_registry::Event::Global { name, interface, version } if interface == "wl_output" => {
            assert!(version >= 4, "missing wl_output v4");
            let output = proxy.bind(name, 4, qhandle, name);
            state.add_output(name, output, qhandle);
        }
        wl_registry::Event::GlobalRemove { name } => state.remove_output(name),
        _ => {}
    }
});

delegate_noop!(Wallpaper: ignore WlCompositor);
delegate_noop!(Wallpaper: ignore ZwlrLayerShellV1);
delegate_noop!(Wallpaper: ignore WpViewporter);
delegate_noop!(Wallpaper: ignore WpViewport);
delegate_noop!(Wallpaper: ignore WpFractionalScaleManagerV1);
dispatch!(Wallpaper, WlSurface, OutputId, _data, |_state, _proxy, _event, _queue| {});
delegate_noop!(Wallpaper: ignore WlRegion);

impl Drop for OutputSurface {
    fn drop(&mut self) {
        self.layer.destroy();
        self.fractional.destroy();
        self.viewport.destroy();
    }
}
