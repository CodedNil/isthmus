#![expect(clippy::wildcard_enum_match_arm, reason = "Wayland event enums are non-exhaustive and mostly ignored")]
use crate::{
    app::{CantusApp, View},
    config,
    interaction::{InputEvent, Interaction},
    render::{PANEL_START, Renderer, install_fonts, launcher::CORNER_RADIUS, lyrics, weathertime},
};
use calloop::{EventLoop, channel};
use clipboard::Clipboard;
use isthmus::{
    RenderError, SurfaceHandle,
    glam::{Vec2, vec2},
    wgpu::rwh::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, WaylandWindowHandle,
        WindowHandle,
    },
};
use smallvec::SmallVec;
use std::{
    fs::File,
    os::unix::fs::FileExt as _,
    ptr::NonNull,
    sync::Arc,
    time::{Duration, Instant},
};
use strum::VariantArray as _;
use tokio::runtime;
use tracing::warn;
use wayland_client::{
    Connection, Proxy as _, QueueHandle, WEnum,
    backend::Backend,
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::{self, WlCallback},
        wl_compositor::WlCompositor,
        wl_keyboard::{self, KeyState, KeymapFormat, WlKeyboard},
        wl_output::{self, WlOutput},
        wl_pointer::{self, WlPointer},
        wl_region::WlRegion,
        wl_registry::WlRegistry,
        wl_seat::{self, WlSeat},
        wl_surface::WlSurface,
    },
};
use wayland_protocols::{
    ext::background_effect::v1::client::{
        ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
        ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
    },
    wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};
use xkbcommon::xkb;

const PANEL_OVERFLOW: f32 = 12.0;

macro_rules! dispatch {
    ($app:ty, $proxy:ty, |$state:ident, $object:ident, $value:ident, $queue:ident| $body:block) => {
        dispatch!($app, $proxy, (), _data, |$state, $object, $value, $queue| $body);
    };
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

mod clipboard;

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

/// Runs the Wayland application event loop.
pub fn run() {
    let runtime = runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .thread_keep_alive(Duration::from_secs(10))
        .thread_name("cantus-async")
        .thread_stack_size(512 * 1024)
        .enable_all()
        .build()
        .expect("failed to start Cantus async runtime");
    let _runtime_context = runtime.enter();
    let connection = Connection::connect_to_env().expect("Failed to connect to Wayland display");
    let (globals, mut event_queue) =
        registry_queue_init::<LayerShellApp>(&connection).expect("Failed to read Wayland registry");
    let qhandle = event_queue.handle();
    let seat: WlSeat = globals.bind(&qhandle, 8..=9, ()).expect("Missing wl_seat v8");

    let (sender, updates) = channel::channel();
    let mut app = LayerShellApp {
        compositor: globals.bind(&qhandle, 6..=7, ()).expect("missing wl_compositor v6"),
        shell: globals.bind(&qhandle, 4..=4, ()).expect("missing zwlr_layer_shell_v1"),
        viewporter: globals.bind(&qhandle, 1..=1, ()).expect("missing wp_viewporter"),
        fractional: globals.bind(&qhandle, 1..=1, ()).expect("missing wp_fractional_scale_manager_v1"),
        backend: connection.backend(),
        clipboard: globals.bind(&qhandle, 1..=3, ()).ok().map(|manager| Clipboard::new(manager, &seat, &qhandle)),
        cantus: CantusApp::new(sender),
        background_manager: globals.bind(&qhandle, 1..=1, ()).ok(),
        ..
    };

    // Bind every output so names arrive; the configured monitor replaces the first.
    let registry = globals.registry();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            assert!(global.version >= 4, "Missing wl_output v4");
            let output = registry.bind::<WlOutput, (), LayerShellApp>(global.name, 4, &qhandle, ());
            app.output.get_or_insert(output);
        }
    }
    event_queue.roundtrip(&mut app).expect("Failed to fetch output details");

    app.surfaces[View::Bar as usize] = Some(app.create_surface(View::Bar, &qhandle));
    connection.flush().expect("Failed to flush initial commit");

    let mut event_loop = EventLoop::try_new().expect("failed to create event loop");
    calloop_wayland_source::WaylandSource::new(connection, event_queue)
        .insert(event_loop.handle())
        .expect("failed to register Wayland events");
    event_loop
        .handle()
        .insert_source(updates, |event, (), app: &mut LayerShellApp| {
            if let channel::Event::Msg(update) = event {
                update(&mut app.cantus);
            }
        })
        .expect("failed to register background updates");
    while !app.should_exit {
        let deadline = app.repeat.map_or(app.cantus.next_enrichment, |(_, next)| next.min(app.cantus.next_enrichment));
        if let Err(error) = event_loop.dispatch(deadline.saturating_duration_since(Instant::now()), &mut app) {
            warn!(%error, "Wayland connection closed");
            break;
        }
        if !app.cantus.launcher.open {
            app.repeat = None;
        }
        let now = Instant::now();
        if let Some((keycode, next)) = app.repeat
            && next <= now
        {
            app.repeat = Some((keycode, now + app.repeat_interval));
            handle_launcher_key(&mut app, keycode, &qhandle);
        }
        app.cantus.refresh();
        app.try_render_frame(&qhandle);
        app.copy(&qhandle);
    }
}

struct LayerShellApp {
    gpu: Option<Renderer> = None,
    gpu_retry: Option<Instant> = None,
    cantus: CantusApp,

    should_exit: bool = false,

    compositor: WlCompositor,
    shell: ZwlrLayerShellV1,
    viewporter: WpViewporter,
    fractional: WpFractionalScaleManagerV1,
    backend: Backend,
    pointer: Option<WlPointer> = None,
    keyboard: Option<WlKeyboard> = None,
    xkb_state: Option<xkb::State> = None,
    /// Repeat timing advertised by the compositor; zero disables repetition.
    repeat_delay: Duration = Duration::ZERO,
    repeat_interval: Duration = Duration::ZERO,
    /// The held key waiting to repeat and when it next fires.
    repeat: Option<(xkb::Keycode, Instant)> = None,
    clipboard: Option<Clipboard>,
    output: Option<WlOutput> = None,
    frame_callback: Option<WlCallback> = None,
    surfaces: [Option<WaylandSurface>; View::VARIANTS.len()] = [const { None }; View::VARIANTS.len()],
    background_manager: Option<ExtBackgroundEffectManagerV1>,
}

struct WaylandSurface {
    native: Arc<NativeSurface>,
    layer: ZwlrLayerSurfaceV1,
    fractional: WpFractionalScaleV1,
    viewport: WpViewport,
    size: Vec2,
    scale: f32,
    effect: Option<ExtBackgroundEffectSurfaceV1>,
    gpu: Option<SurfaceHandle>,
    blur_bounds: Option<(Vec2, Vec2)>,
    viewport_size: Option<[i32; 2]>,
    input_regions: Option<SmallVec<[[i32; 4]; 16]>>,
}

impl Drop for WaylandSurface {
    fn drop(&mut self) {
        if let Some(effect) = &self.effect {
            effect.destroy();
        }
        self.layer.destroy();
        self.fractional.destroy();
        self.viewport.destroy();
    }
}

impl LayerShellApp {
    fn bar_height(&self) -> f32 {
        let config = &self.cantus.config;
        let extension = (f32::from(config.weathertime_enabled) * weathertime::EXTENSION)
            .max(if config.lyrics_enabled { lyrics::height(&self.cantus.music) } else { 0.0 });
        config.height + PANEL_START + extension + PANEL_OVERFLOW
    }

    fn create_surface(&self, kind: View, qhandle: &QueueHandle<Self>) -> WaylandSurface {
        let config = &self.cantus.config;
        let (output, layer, namespace, anchor, height, exclusive, keyboard) = match kind {
            View::Launcher => (
                None,
                Layer::Overlay,
                "cantus-launcher",
                Anchor::Top | Anchor::Bottom,
                0.0,
                0.0,
                KeyboardInteractivity::Exclusive,
            ),
            View::Bar => (
                self.output.as_ref(),
                match config.layer {
                    config::Layer::Background => Layer::Background,
                    config::Layer::Bottom => Layer::Bottom,
                    config::Layer::Top => Layer::Top,
                    config::Layer::Overlay => Layer::Overlay,
                },
                "cantus",
                match config.layer_anchor {
                    config::LayerAnchor::Top => Anchor::Top,
                    config::LayerAnchor::Bottom => Anchor::Bottom,
                },
                self.bar_height(),
                PANEL_START + config.height + f32::from(config.lyrics_enabled) * lyrics::EXTENSION,
                KeyboardInteractivity::None,
            ),
        };
        let wl = self.compositor.create_surface(qhandle, kind);
        let layer = self.shell.get_layer_surface(&wl, output, layer, namespace.into(), qhandle, kind);
        layer.set_anchor(Anchor::Left | Anchor::Right | anchor);
        layer.set_size(0, height as u32);
        layer.set_exclusive_zone(exclusive as i32);
        layer.set_keyboard_interactivity(keyboard);
        let surface = WaylandSurface {
            effect: self.background_manager.as_ref().map(|manager| manager.get_background_effect(&wl, qhandle, ())),
            viewport: self.viewporter.get_viewport(&wl, qhandle, ()),
            fractional: self.fractional.get_fractional_scale(&wl, qhandle, kind),
            native: Arc::new(NativeSurface { backend: self.backend.clone(), wl }),
            layer,
            size: vec2(0.0, height),
            scale: 1.0,
            gpu: None,
            blur_bounds: None,
            viewport_size: None,
            input_regions: None,
        };
        surface.native.wl.commit();
        surface
    }

    fn paste(&self) {
        let session = self.cantus.launcher.session;
        let Some(reader) = self.clipboard.as_ref().and_then(Clipboard::load) else { return };
        self.cantus.background.spawn_update(async move {
            let text = clipboard::read(reader).await?;
            Some(move |app: &mut CantusApp| app.launcher.paste(session, &text))
        });
    }

    fn active_surface(&self) -> &WaylandSurface {
        self.surfaces[View::Launcher as usize]
            .as_ref()
            .or(self.surfaces[View::Bar as usize].as_ref())
            .expect("bar surface exists")
    }

    fn sync_launcher_surface(&mut self, qhandle: &QueueHandle<Self>) {
        if self.cantus.launcher.open == self.surfaces[View::Launcher as usize].is_some() {
            return;
        }
        self.cantus.interaction = Interaction::default();
        self.repeat = None;
        self.frame_callback = None;
        if self.cantus.launcher.open {
            self.surfaces[View::Launcher as usize] = Some(self.create_surface(View::Launcher, qhandle));
        } else if let Some(surface) = self.surfaces[View::Launcher as usize].take()
            && let (Some(gpu), Some(handle)) = (&mut self.gpu, surface.gpu)
        {
            gpu.remove_surface(handle);
        }
    }

    fn try_render_frame(&mut self, qhandle: &QueueHandle<Self>) {
        if let Some(at) = self.gpu_retry {
            if Instant::now() < at {
                return;
            }
            self.gpu = None;
            self.frame_callback = None;
            for surface in self.surfaces.iter_mut().flatten() {
                surface.gpu = None;
            }
            self.gpu_retry = None;
        }
        self.sync_launcher_surface(qhandle);
        let height = self.bar_height() as u32;
        if let Some(surface) = &self.surfaces[View::Bar as usize]
            && surface.size.y as u32 != height
        {
            surface.layer.set_size(0, height);
            surface.native.wl.commit();
        }
        if self.frame_callback.is_some()
            || self.surfaces[View::Bar as usize].is_none()
            || self.surfaces.iter().flatten().any(|surface| surface.size.min_element() <= 0.0)
        {
            return;
        }
        for surface in self.surfaces.iter_mut().flatten() {
            let destination = [surface.size.x as i32, surface.size.y as i32];
            if surface.viewport_size != Some(destination) {
                surface.viewport.set_destination(destination[0], destination[1]);
                surface.viewport_size = Some(destination);
            }
            let size = (surface.size * surface.scale).to_array().map(|size| size.round().max(1.0) as u32);
            if self.gpu.is_none() {
                install_fonts();
                let (gpu, handle) = match pollster::block_on(Renderer::new(Arc::clone(&surface.native), size)) {
                    Ok(renderer) => renderer,
                    Err(error) => {
                        tracing::error!(%error, "Could not initialize renderer; retrying");
                        self.gpu_retry = Some(Instant::now() + Duration::from_secs(1));
                        return;
                    }
                };
                tracing::info!("Using GPU device: {}", gpu.device_name());
                self.gpu = Some(gpu);
                surface.gpu = Some(handle);
            }
            let gpu = self.gpu.as_mut().unwrap();
            let handle = if let Some(handle) = surface.gpu {
                handle
            } else {
                let handle = match gpu.add_surface(Arc::clone(&surface.native), size) {
                    Ok(handle) => handle,
                    Err(error) => {
                        tracing::error!(%error, "Could not attach surface; retrying");
                        self.gpu_retry = Some(Instant::now() + Duration::from_secs(1));
                        return;
                    }
                };
                surface.gpu = Some(handle);
                handle
            };
            gpu.resize(handle, size);
        }
        if let Err(error) = self.gpu.as_mut().unwrap().render(|render| {
            for (&view, surface) in View::VARIANTS.iter().zip(&self.surfaces) {
                if let Some(surface) = surface {
                    self.cantus.draw(render, surface.gpu.unwrap(), surface.size, view);
                }
            }
        }) {
            tracing::error!(%error, "Could not render frame");
            if !matches!(error, RenderError::Validation | RenderError::ResourceLimit { .. } | RenderError::MissingImage)
                && (matches!(error, RenderError::SurfaceLost)
                    || pollster::block_on(self.gpu.as_mut().unwrap().recover())
                        .inspect_err(|error| tracing::error!(%error, "Could not recover GPU device"))
                        .is_err())
            {
                // A fresh instance is the fallback if the adapter itself is gone.
                self.gpu_retry = Some(Instant::now() + Duration::from_secs(1));
            }
            // Still request the frame callback so recovery gets a frame.
        }
        let input_regions: SmallVec<[[i32; 4]; 16]> = self
            .cantus
            .interaction
            .input_regions
            .drain(..)
            .map(|rect| {
                let min = rect.min.floor();
                let size = rect.max.ceil() - min;
                [min.x as i32, min.y as i32, size.x as i32, size.y as i32]
            })
            .collect();
        let surface = self.surfaces.iter_mut().rev().flatten().next().expect("bar surface exists");
        // Wayland keeps these across commits. Only resend when pixel bounds change.
        if surface.input_regions.as_ref() != Some(&input_regions) {
            let region = self.compositor.create_region(qhandle, ());
            for &[x, y, width, height] in &input_regions {
                region.add(x, y, width, height);
            }
            surface.native.wl.set_input_region(Some(&region));
            region.destroy();
            surface.input_regions = Some(input_regions);
        }
        self.update_blur_region(qhandle);
        self.frame_callback = Some(self.active_surface().native.wl.frame(qhandle, ()));
        self.active_surface().native.wl.commit();
    }

    fn copy(&mut self, queue: &QueueHandle<Self>) {
        if let Some(text) = self.cantus.launcher.pending_copy.take()
            && let Some(clipboard) = &self.clipboard
        {
            clipboard.store(text, queue);
        }
    }

    fn update_blur_region(&mut self, qhandle: &QueueHandle<Self>) {
        let Some(surface) = self.surfaces[View::Launcher as usize].as_mut() else { return };
        let Some(effect) = &surface.effect else { return };
        let (origin, size) = self.cantus.launcher.bounds(surface.size);
        if surface.blur_bounds == Some((origin, size)) {
            return;
        }
        let region = self.compositor.create_region(qhandle, ());
        // The blur region sits one pixel inside the antialiased panel edge.
        let x = origin.x.ceil() as i32 + 1;
        let y = origin.y.ceil() as i32 + 1;
        let width = (origin.x + size.x).floor() as i32 - 1 - x;
        let height = (origin.y + size.y).floor() as i32 - 1 - y;
        let radius = (CORNER_RADIUS as i32 - 1).min(width / 2).min(height / 2);
        region.add(x, y + radius, width, height - radius * 2);
        for row in 0..radius {
            let dy = radius as f32 - row as f32 - 0.5;
            let dx = ((radius * radius) as f32 - dy * dy).sqrt();
            let inset = radius - (dx + 0.5).round() as i32;
            region.add(x + inset, y + row, width - inset * 2, 1);
            region.add(x + inset, y + height - row - 1, width - inset * 2, 1);
        }
        effect.set_blur_region(Some(&region));
        region.destroy();
        surface.blur_bounds = Some((origin, size));
    }
}

dispatch!(LayerShellApp, ZwlrLayerSurfaceV1, View, kind, |state, proxy, event, _qhandle| {
    let Some(surface) = state.surfaces[*kind as usize].as_mut().filter(|surface| surface.layer == *proxy) else {
        return;
    };
    match event {
        zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
            surface.layer.ack_configure(serial);
            // Zero leaves the corresponding dimension to the client.
            if width > 0 {
                surface.size.x = width as f32;
            }
            if height > 0 {
                surface.size.y = height as f32;
            }
        }
        zwlr_layer_surface_v1::Event::Closed => {
            if matches!(kind, View::Bar) {
                state.should_exit = true;
                return;
            }
            state.cantus.launcher.open = false;
        }
        _ => return,
    }
});

dispatch!(LayerShellApp, WlCallback, |state, proxy, event, _qhandle| {
    if matches!(event, wl_callback::Event::Done { .. })
        && state.frame_callback.as_ref().is_some_and(|callback| callback.id() == proxy.id())
    {
        state.frame_callback = None;
    }
});

dispatch!(LayerShellApp, WpFractionalScaleV1, View, kind, |state, proxy, event, _qhandle| {
    if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
        && let Some(surface) = state.surfaces[*kind as usize].as_mut()
        && surface.fractional == *proxy
    {
        surface.scale = scale as f32 / 120.0;
    }
});

dispatch!(LayerShellApp, WlOutput, |state, proxy, event, _qhandle| {
    let Some(monitor) = &state.cantus.config.monitor else { return };
    match event {
        wl_output::Event::Name { name } | wl_output::Event::Description { description: name }
            if name.to_ascii_lowercase().contains(&monitor.to_ascii_lowercase()) =>
        {
            state.output = Some(proxy.clone());
        }
        _ => {}
    }
});

dispatch!(LayerShellApp, WlSeat, |state, proxy, event, qhandle| {
    if let wl_seat::Event::Capabilities { capabilities } = event
        && let WEnum::Value(caps) = capabilities
    {
        if caps.contains(wl_seat::Capability::Pointer) {
            if state.pointer.is_none() {
                state.pointer = Some(proxy.get_pointer(qhandle, ()));
            }
        } else if let Some(pointer) = state.pointer.take() {
            pointer.release();
        }
        if caps.contains(wl_seat::Capability::Keyboard) {
            if state.keyboard.is_none() {
                state.keyboard = Some(proxy.get_keyboard(qhandle, ()));
            }
        } else if let Some(keyboard) = state.keyboard.take() {
            keyboard.release();
        }
    }
});

dispatch!(LayerShellApp, WlKeyboard, |state, _proxy, event, qhandle| {
    match event {
        wl_keyboard::Event::Keymap { format: WEnum::Value(KeymapFormat::XkbV1), fd, size } => {
            let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            let mut bytes = vec![0; size as usize];
            state.xkb_state = File::from(fd)
                .read_exact_at(&mut bytes, 0)
                .inspect_err(|error| warn!(%error, "Failed to read keyboard keymap"))
                .ok()
                .and_then(|()| String::from_utf8(bytes).ok())
                .and_then(|text| {
                    xkb::Keymap::new_from_string(
                        &context,
                        text,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                })
                .map(|keymap| xkb::State::new(&keymap));
            if state.xkb_state.is_none() {
                warn!("Keyboard keymap could not be loaded");
            }
        }
        wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
            if let Some(xkb_state) = &mut state.xkb_state {
                xkb_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
            }
        }
        wl_keyboard::Event::RepeatInfo { rate, delay } => {
            state.repeat_delay = Duration::from_millis(delay.max(0) as u64);
            state.repeat_interval =
                if rate > 0 { Duration::from_micros(1_000_000 / rate as u64) } else { Duration::ZERO };
            state.repeat = None;
        }
        wl_keyboard::Event::Leave { .. } => state.repeat = None,
        wl_keyboard::Event::Key { key, serial, state: WEnum::Value(key_state), .. } => {
            let keycode = xkb::Keycode::new(key + 8);
            if key_state == KeyState::Pressed {
                if let Some(clipboard) = &mut state.clipboard {
                    clipboard.serial = Some(serial);
                }
                // Modifiers are marked as non-repeating by the keymap, so they never latch here.
                let repeats =
                    state.xkb_state.as_ref().is_some_and(|xkb_state| xkb_state.get_keymap().key_repeats(keycode));
                state.repeat = (repeats && !state.repeat_interval.is_zero())
                    .then(|| (keycode, Instant::now() + state.repeat_delay));
                handle_launcher_key(state, keycode, qhandle);
            } else if state.repeat.is_some_and(|(held, _)| held == keycode) {
                state.repeat = None;
            }
            if let Some(xkb_state) = &mut state.xkb_state {
                let direction =
                    if key_state == KeyState::Released { xkb::KeyDirection::Up } else { xkb::KeyDirection::Down };
                xkb_state.update_key(keycode, direction);
            }
        }
        _ => {}
    }
});

/// Applies one key press to the launcher's search field and selection while it is open.
fn handle_launcher_key(state: &mut LayerShellApp, keycode: xkb::Keycode, qhandle: &QueueHandle<LayerShellApp>) {
    if !state.cantus.launcher.open {
        return;
    }
    let Some(xkb_state) = &state.xkb_state else {
        return;
    };
    let sym = xkb_state.key_get_one_sym(keycode);
    if !state.surfaces[View::Launcher as usize].as_ref().is_some_and(|surface| surface.size.min_element() > 0.0)
        && matches!(sym.raw(), xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter)
    {
        return;
    }
    let shift = xkb_state.mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE);
    let control = xkb_state.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE);
    let mut text = [0; 4];
    let key = match sym.raw() {
        xkb::keysyms::KEY_Escape => "Escape",
        xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter => "Enter",
        xkb::keysyms::KEY_Up => "ArrowUp",
        xkb::keysyms::KEY_Down => "ArrowDown",
        xkb::keysyms::KEY_BackSpace => "Backspace",
        xkb::keysyms::KEY_Delete => "Delete",
        xkb::keysyms::KEY_Left => "ArrowLeft",
        xkb::keysyms::KEY_Right => "ArrowRight",
        xkb::keysyms::KEY_Home => "Home",
        xkb::keysyms::KEY_End => "End",
        _ => {
            let character = if control { char::from_u32(sym.raw()).filter(char::is_ascii) } else { sym.key_char() };
            let Some(character) = character else { return };
            character.encode_utf8(&mut text)
        }
    };
    if control && key.eq_ignore_ascii_case("v") {
        state.paste();
    } else {
        state.cantus.launcher.input(key, shift, control);
        state.copy(qhandle);
    }
}

dispatch!(LayerShellApp, WlPointer, |state, _proxy, event, _qhandle| {
    if state.gpu.is_none() {
        return;
    }
    let surface = Arc::clone(&state.active_surface().native);
    let interaction = &mut state.cantus.interaction;
    let input =
        match event {
            wl_pointer::Event::Enter { surface: wl_surface, surface_x, surface_y, .. } if surface.wl == wl_surface => {
                InputEvent::Enter(vec2(surface_x as f32, surface_y as f32))
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                InputEvent::Motion(vec2(surface_x as f32, surface_y as f32))
            }
            wl_pointer::Event::Leave { .. } => InputEvent::Leave,
            wl_pointer::Event::Button { button, serial, state: WEnum::Value(button_state), .. } => {
                if let Some(clipboard) = &mut state.clipboard {
                    clipboard.serial = Some(serial);
                }
                match (button, button_state) {
                    (0x110, wl_pointer::ButtonState::Pressed) => InputEvent::Press,
                    (0x110, wl_pointer::ButtonState::Released) => InputEvent::Release,
                    (0x111, wl_pointer::ButtonState::Pressed) if interaction.dragging() => InputEvent::Cancel,
                    _ => return,
                }
            }

            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll), value120, ..
            } if value120 != 0 => InputEvent::Scroll(value120.signum()),
            _ => return,
        };
    interaction.apply(input);
});

dispatch!(LayerShellApp, WlRegistry, GlobalListContents, _globals, |_state, _proxy, _event, _qhandle| {});

delegate_noop!(LayerShellApp: ignore WlRegion);
delegate_noop!(LayerShellApp: ignore ExtBackgroundEffectManagerV1);
delegate_noop!(LayerShellApp: ignore ExtBackgroundEffectSurfaceV1);

delegate_noop!(LayerShellApp: ignore WlCompositor);
delegate_noop!(LayerShellApp: ignore ZwlrLayerShellV1);
delegate_noop!(LayerShellApp: ignore WpViewporter);
delegate_noop!(LayerShellApp: ignore WpViewport);
delegate_noop!(LayerShellApp: ignore WpFractionalScaleManagerV1);
dispatch!(LayerShellApp, WlSurface, View, _data, |_state, _proxy, _event, _queue| {});
