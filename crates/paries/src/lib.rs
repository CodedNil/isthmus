#[path = "sdf.rs"]
pub mod render;

#[cfg(not(target_arch = "spirv"))]
mod host {
    use crate::render::{Bamboo, program};
    use isthmus::{
        Renderer, SurfaceHandle,
        glam::{Vec3, vec2},
    };
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
        WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
    };
    use std::{
        ffi::c_void,
        ptr::NonNull,
        time::{Duration, Instant},
    };
    use wayland_client::{
        Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
        globals::{GlobalListContents, registry_queue_init},
        protocol::{
            wl_callback::{self, WlCallback},
            wl_compositor::WlCompositor,
            wl_output::{self, WlOutput},
            wl_region::WlRegion,
            wl_registry::{self, WlRegistry},
            wl_surface::{self, WlSurface},
        },
    };
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
    };

    type OutputId = u32;
    const FRAME_INTERVAL: Duration = Duration::from_millis(33);

    #[derive(Clone, Copy)]
    struct NativeSurface {
        display: NonNull<c_void>,
        window: NonNull<c_void>,
    }

    impl HasDisplayHandle for NativeSurface {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(
                unsafe {
                    DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display)))
                },
            )
        }
    }

    impl HasWindowHandle for NativeSurface {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Wayland(WaylandWindowHandle::new(self.window))) })
        }
    }

    struct OutputSurface {
        id: OutputId,
        output: WlOutput,
        surface: WlSurface,
        layer_surface: ZwlrLayerSurfaceV1,
        frame_callback: Option<WlCallback>,
        render_surface: Option<SurfaceHandle>,
        logical_size: [u32; 2],
        output_scale: i32,
        configured: bool,
        last_render: Option<Instant>,
    }

    struct Wallpaper {
        compositor: WlCompositor,
        layer_shell: ZwlrLayerShellV1,
        display: NonNull<c_void>,
        renderer: Option<Renderer>,
        outputs: Vec<OutputSurface>,
    }

    impl Wallpaper {
        fn output_index(&self, id: OutputId) -> Option<usize> {
            self.outputs.iter().position(|output| output.id == id)
        }

        fn add_output(&mut self, id: OutputId, output: WlOutput, qhandle: &QueueHandle<Self>) {
            if self.output_index(id).is_some() {
                return;
            }
            let surface = self.compositor.create_surface(qhandle, id);
            let layer_surface = self.layer_shell.get_layer_surface(
                &surface,
                Some(&output),
                Layer::Background,
                format!("paries-{id}"),
                qhandle,
                id,
            );
            layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer_surface.set_exclusive_zone(0);
            layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
            let input = self.compositor.create_region(qhandle, ());
            surface.set_input_region(Some(&input));
            input.destroy();
            surface.commit();
            self.outputs.push(OutputSurface {
                id,
                output,
                surface,
                layer_surface,
                frame_callback: None,
                render_surface: None,
                logical_size: [0; 2],
                output_scale: 1,
                configured: false,
                last_render: None,
            });
        }

        fn remove_output(&mut self, id: OutputId) {
            let Some(index) = self.output_index(id) else { return };
            let output = self.outputs.swap_remove(index);
            if let Some(renderer) = &mut self.renderer
                && let Some(surface) = output.render_surface
            {
                renderer.remove_surface(surface);
            }
            drop(output.frame_callback);
            output.layer_surface.destroy();
            output.surface.destroy();
            if output.output.version() >= 3 {
                output.output.release();
            }
        }

        fn draw(&mut self, id: OutputId, qhandle: &QueueHandle<Self>) {
            let Some(index) = self.output_index(id) else { return };
            let output = &self.outputs[index];
            if !output.configured || output.logical_size.contains(&0) {
                return;
            }
            if output.last_render.is_some_and(|last| last.elapsed() < FRAME_INTERVAL) {
                self.request_frame(index, qhandle);
                return;
            }
            let size = output.logical_size;
            if output.render_surface.is_none() {
                let native = NativeSurface {
                    display: self.display,
                    window: NonNull::new(output.surface.id().as_ptr().cast()).expect("Wayland surface pointer exists"),
                };
                let render_surface = if let Some(renderer) = &mut self.renderer {
                    renderer
                        .add_surface(&native, size)
                        .expect("wallpaper surface is incompatible with the renderer")
                } else {
                    let (renderer, surface) = Renderer::new(
                        program(),
                        &native,
                        size,
                        include_bytes!("../../../assets/NotoSans-Variable.ttf"),
                        Vec3::ONE,
                    )
                    .expect("failed to initialize wallpaper renderer");
                    eprintln!("Paries is rendering with {}", renderer.device_name());
                    self.renderer = Some(renderer);
                    surface
                };
                self.outputs[index].render_surface = Some(render_surface);
            }
            let render_surface = self.outputs[index].render_surface.unwrap();
            let renderer = self.renderer.as_mut().unwrap();
            renderer.resize(render_surface, size);
            let mut bamboo = Bamboo;
            if let Err(error) = renderer.render(|render| {
                render.surface(render_surface, vec2(size[0] as f32, size[1] as f32), |mut frame| {
                    bamboo.show(&mut frame);
                });
            }) {
                eprintln!("Paries could not render output {id}: {error}");
            }
            self.outputs[index].last_render = Some(Instant::now());
            self.request_frame(index, qhandle);
        }

        fn request_frame(&mut self, index: usize, qhandle: &QueueHandle<Self>) {
            let output = &mut self.outputs[index];
            if output.frame_callback.is_none() {
                output.frame_callback = Some(output.surface.frame(qhandle, output.id));
            }
            output.surface.commit();
        }
    }

    /// Runs one layer-shell background surface for every Wayland output.
    ///
    /// # Panics
    /// Panics if Wayland, layer-shell, or the GPU renderer cannot be initialized.
    pub fn run() {
        let connection = Connection::connect_to_env().expect("failed to connect to Wayland");
        let (globals, mut events) =
            registry_queue_init::<Wallpaper>(&connection).expect("failed to read Wayland globals");
        let qhandle = events.handle();
        let compositor = globals.bind(&qhandle, 1..=7, ()).expect("missing wl_compositor");
        let layer_shell = globals.bind(&qhandle, 1..=4, ()).expect("missing zwlr_layer_shell_v1");
        let mut app = Wallpaper {
            compositor,
            layer_shell,
            display: NonNull::new(connection.backend().display_ptr().cast()).expect("Wayland display pointer exists"),
            renderer: None,
            outputs: Vec::new(),
        };
        let registry = globals.registry();
        for global in globals.contents().clone_list() {
            if global.interface == "wl_output" {
                let output = registry.bind(global.name, global.version.min(4), &qhandle, global.name);
                app.add_output(global.name, output, &qhandle);
            }
        }
        connection.flush().expect("failed to create wallpaper surfaces");
        loop {
            events.blocking_dispatch(&mut app).expect("Wayland dispatch failed");
        }
    }

    impl Dispatch<ZwlrLayerSurfaceV1, OutputId> for Wallpaper {
        fn event(
            state: &mut Self,
            proxy: &ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            data: &OutputId,
            _: &Connection,
            qhandle: &QueueHandle<Self>,
        ) {
            match event {
                zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                    proxy.ack_configure(serial);
                    if let Some(index) = state.output_index(*data) {
                        if width > 0 && height > 0 {
                            state.outputs[index].logical_size = [width, height];
                        }
                        let first = !state.outputs[index].configured;
                        state.outputs[index].configured = true;
                        if first {
                            let output = &state.outputs[index];
                            eprintln!(
                                "Configured output {} at {}x{} scale {}",
                                output.id, output.logical_size[0], output.logical_size[1], output.output_scale
                            );
                        }
                    }
                    state.draw(*data, qhandle);
                }
                zwlr_layer_surface_v1::Event::Closed => state.remove_output(*data),
                _ => {}
            }
        }
    }

    impl Dispatch<WlCallback, OutputId> for Wallpaper {
        fn event(
            state: &mut Self,
            proxy: &WlCallback,
            event: wl_callback::Event,
            data: &OutputId,
            _: &Connection,
            qhandle: &QueueHandle<Self>,
        ) {
            if matches!(event, wl_callback::Event::Done { .. })
                && let Some(index) = state.output_index(*data)
                && state.outputs[index]
                    .frame_callback
                    .as_ref()
                    .is_some_and(|callback| callback.id() == proxy.id())
            {
                state.outputs[index].frame_callback.take();
                state.draw(*data, qhandle);
            }
        }
    }

    impl Dispatch<WlOutput, OutputId> for Wallpaper {
        fn event(
            state: &mut Self,
            _: &WlOutput,
            event: wl_output::Event,
            data: &OutputId,
            _: &Connection,
            qhandle: &QueueHandle<Self>,
        ) {
            if let wl_output::Event::Scale { factor } = event
                && let Some(index) = state.output_index(*data)
            {
                state.outputs[index].output_scale = factor.max(1);
                state.draw(*data, qhandle);
            }
        }
    }

    impl Dispatch<WlRegistry, GlobalListContents> for Wallpaper {
        fn event(
            state: &mut Self,
            proxy: &WlRegistry,
            event: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            qhandle: &QueueHandle<Self>,
        ) {
            match event {
                wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } if interface == "wl_output" => {
                    let output = proxy.bind(name, version.min(4), qhandle, name);
                    state.add_output(name, output, qhandle);
                }
                wl_registry::Event::GlobalRemove { name } => state.remove_output(name),
                _ => {}
            }
        }
    }

    impl Dispatch<WlSurface, OutputId> for Wallpaper {
        fn event(
            _: &mut Self,
            _: &WlSurface,
            _: wl_surface::Event,
            _: &OutputId,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    delegate_noop!(Wallpaper: ignore WlCompositor);
    delegate_noop!(Wallpaper: ignore WlRegion);
    delegate_noop!(Wallpaper: ignore ZwlrLayerShellV1);
}

#[cfg(not(target_arch = "spirv"))]
pub use host::run;
