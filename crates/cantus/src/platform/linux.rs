use crate::{
    app::{AppUpdater, Background, CantusApp, send_update, update},
    config,
    interaction::{InputEvent, Interaction},
    render::{
        PANEL_START, Renderer, TEXT_COLOR,
        launcher::{BACKGROUND_RADIUS, LauncherKey},
        lyrics,
        status::{AUDIO_SPECTRUM_BANDS, AudioMonitor, ProcessorSample, SystemSample},
        weathertime,
    },
};
use freedesktop_desktop_entry::{desktop_entries, get_languages_from_env};
use futures_util::StreamExt;
use isthmus::{
    SurfaceHandle,
    geometry::text::TextCache,
    glam::{FloatExt, Vec2, vec2},
};
use microfft::real::rfft_1024;
use nvml_wrapper::{Nvml, enum_wrappers::device::TemperatureSensor};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    ffi::c_void,
    fs::{self, File},
    future::Future,
    io::{self, Read, Write},
    os::{fd::AsFd, unix::net::UnixDatagram},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{net, runtime, sync::mpsc::UnboundedSender, task::spawn_blocking, time::sleep};
use tracing::warn;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::{self, WlCallback},
        wl_compositor::WlCompositor,
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::WlDataDeviceManager,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
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
use zbus::{
    proxy::{self, CacheProperties},
    zvariant::{self, OwnedObjectPath, OwnedValue},
};

const PANEL_OVERFLOW: f32 = 16.0;
const AUDIO_SAMPLE_RATE: u32 = 48_000;
const AUDIO_WINDOW_SIZE: usize = 1024;
const AUDIO_BAND_EDGES: [f32; AUDIO_SPECTRUM_BANDS + 1] =
    [60.0, 120.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 12_000.0];
const LAUNCHER_SOCKET_NAME: &str = "cantus-launcher.sock";
const TEXT_MIME: &str = "text/plain;charset=utf-8";

pub trait Task = Future + Send + 'static;

impl Background {
    pub(crate) fn spawn(&self, task: impl Task<Output = ()>) {
        self.runtime.spawn(task);
    }
}

fn spawn_thread(name: &'static str, job: impl FnOnce() + Send + 'static) {
    thread::Builder::new().name(name.into()).spawn(job).expect("failed to spawn background thread");
}

impl super::Platform {
    pub const STATUS_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

    pub fn start_status_monitor(updates: AppUpdater, audio: Arc<AudioMonitor>) {
        let volume = Arc::clone(&audio);
        spawn_thread("cantus-audio-playback", move || monitor_playback(&audio.spectrum));
        spawn_thread("cantus-audio-volume", move || monitor_volume(&volume.volume));
        spawn_thread("cantus-system-status", move || monitor_status(&updates));
    }

    pub fn start_location_monitor(background: &Background, updates: UnboundedSender<[f32; 2]>) {
        background.spawn(async move {
            if let Err(error) = stream_location(&updates).await {
                warn!(%error, "Location portal unavailable");
            }
        });
    }

    pub async fn sleep(duration: Duration) {
        sleep(duration).await;
    }

    pub fn set_volume(volume: f32) {
        let volume = format!("{volume:.3}");
        if let Err(error) = Command::new("wpctl").args(["set-volume", "@DEFAULT_AUDIO_SINK@", &volume]).spawn() {
            warn!(%error, "Failed to set PipeWire volume");
        }
    }

    /// Calls logind directly, which is what `systemctl poweroff` does under the hood.
    pub fn run_power_action(background: &Background, action: usize) {
        let method = ["PowerOff", "Reboot"][action];
        background.spawn(async move {
            let result: Result<(), zbus::Error> = async {
                zbus::Connection::system()
                    .await?
                    .call_method(
                        Some("org.freedesktop.login1"),
                        "/org/freedesktop/login1",
                        Some("org.freedesktop.login1.Manager"),
                        method,
                        &(false,),
                    )
                    .await?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                warn!(%error, method, "Failed to run held power action");
            }
        });
    }

    pub fn desktop_apps() -> Vec<super::DesktopApp> {
        let mut seen = HashSet::new();
        let locales = get_languages_from_env();
        desktop_entries(&locales)
            .into_iter()
            .filter(|entry| seen.insert(entry.id().to_owned()))
            .filter(|entry| !entry.no_display() && !entry.hidden() && !entry.terminal())
            .filter_map(|entry| {
                let action = entry.actions().and_then(|actions| {
                    let action = actions.into_iter().find(|action| !action.is_empty())?;
                    let name = entry.action_entry_localized(action, "Name", &locales)?;
                    Some((name.into_owned(), entry.parse_exec_action(action).ok()?))
                });
                Some(super::DesktopApp {
                    name: entry.name(&locales)?.into_owned(),
                    exec: entry.parse_exec().ok()?,
                    comment: entry.comment(&locales).unwrap_or_default().into_owned(),
                    action,
                    icon: entry
                        .icon()
                        .and_then(|icon| {
                            let path = Path::new(icon);
                            if path.is_absolute() {
                                Some(path.to_owned())
                            } else {
                                freedesktop_icons::lookup(icon).with_size(64).find()
                            }
                        })
                        .and_then(|path| {
                            let bytes = fs::read(path).ok()?;
                            Self::decode_icon(&bytes)
                        }),
                })
            })
            .collect()
    }

    pub fn spawn(command: &[String]) {
        let Some((program, args)) = command.split_first() else {
            return;
        };
        if let Err(error) = Command::new("systemd-run")
            .args(["--user", "--collect", "--quiet", "--"])
            .arg(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            warn!(%error, program, "Failed to launch application");
        }
    }

    pub fn open_url(url: &str) {
        if let Err(error) = Command::new("xdg-open").arg(url).spawn() {
            warn!(%error, %url, "Failed to open URL");
        }
    }

    pub fn start_launcher_listener(background: &Background, updater: &AppUpdater) {
        let path = launcher_socket_path();
        if UnixDatagram::unbound().and_then(|socket| socket.send_to(&[0], &path)).is_ok() {
            warn!(?path, "Another Cantus instance owns the launcher socket");
            return;
        }
        let _ = fs::remove_file(&path);
        let updater = updater.clone();
        background.spawn(async move {
            let socket = match net::UnixDatagram::bind(&path) {
                Ok(socket) => socket,
                Err(error) => {
                    warn!(%error, ?path, "Failed to bind launcher toggle socket");
                    return;
                }
            };
            let mut buffer = [0u8; 1];
            while socket.recv(&mut buffer).await.is_ok() {
                if !send_update(&updater, |app| app.launcher.toggle()) {
                    warn!("Launcher toggle update was discarded");
                    break;
                }
            }
        });
    }

    pub fn trigger_launcher() -> ! {
        let path = launcher_socket_path();
        if let Err(error) = UnixDatagram::unbound().and_then(|socket| socket.send_to(&[0], &path)) {
            eprintln!("Failed to reach a running Cantus instance at {}: {error}", path.display());
            process::exit(1);
        }
        process::exit(0);
    }

    /// Runs the Wayland application event loop.
    ///
    /// # Panics
    ///
    /// Panics when required Wayland globals or rendering resources cannot be initialized.
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
        let compositor: WlCompositor = globals.bind(&qhandle, 6..=7, ()).expect("Missing wl_compositor v6");
        let layer_shell: ZwlrLayerShellV1 = globals.bind(&qhandle, 4..=4, ()).expect("Missing zwlr_layer_shell_v1");
        let seat: WlSeat = globals.bind(&qhandle, 8..=9, ()).expect("Missing wl_seat v8");

        let mut app = LayerShellApp {
            compositor,
            layer_shell,
            display_handle: NonNull::new(connection.backend().display_ptr().cast()).expect("Wayland display pointer"),
            clipboard: {
                let manager: WlDataDeviceManager =
                    globals.bind(&qhandle, 3..=3, ()).expect("Missing clipboard manager v3");
                let device = manager.get_data_device(&seat, &qhandle, ());
                (manager, device)
            },
            cantus: CantusApp::default(),
            scaling: (
                globals.bind(&qhandle, 1..=1, ()).expect("Missing wp_viewporter"),
                globals.bind(&qhandle, 1..=1, ()).expect("Missing wp_fractional_scale_manager_v1"),
            ),
            background_manager: globals.bind(&qhandle, 1..=1, ()).ok(),
            ..
        };

        // Every output is bound so its name and description arrive; the configured monitor replaces the first one.
        let registry = globals.registry();
        for global in globals.contents().clone_list() {
            if global.interface == "wl_output" {
                assert!(global.version >= 4, "Missing wl_output v4");
                let output = registry.bind::<WlOutput, (), LayerShellApp>(global.name, 4, &qhandle, ());
                app.output.get_or_insert(output);
            }
        }
        event_queue.roundtrip(&mut app).expect("Failed to fetch output details");

        app.surfaces[SurfaceKind::Bar as usize] = Some(app.create_surface(SurfaceKind::Bar, &qhandle));
        connection.flush().expect("Failed to flush initial commit");

        while !app.should_exit {
            if let Err(error) = event_queue.blocking_dispatch(&mut app) {
                warn!(%error, "Wayland connection closed");
                break;
            }
        }
    }
}

async fn stream_location(sender: &UnboundedSender<[f32; 2]>) -> Result<(), Box<dyn Error + Send + Sync>> {
    const DESTINATION: &str = "org.freedesktop.portal.Desktop";
    let connection = zbus::Connection::session().await?;
    let location = proxy::Builder::<zbus::Proxy>::new(&connection)
        .destination(DESTINATION)?
        .path("/org/freedesktop/portal/desktop")?
        .interface("org.freedesktop.portal.Location")?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let session_token = format!("cantus_{:x}", fastrand::u64(..));
    let session: OwnedObjectPath = location
        .call(
            "CreateSession",
            &HashMap::from([
                ("session_handle_token", zvariant::Value::from(session_token)),
                ("accuracy", zvariant::Value::from(2u32)),
            ]),
        )
        .await?;
    let mut updates = location.receive_signal("LocationUpdated").await?;

    let request_token = format!("cantus_{:x}", fastrand::u64(..));
    let sender_name = connection.unique_name().unwrap().trim_start_matches(':').replace('.', "_");
    let request = proxy::Builder::<zbus::Proxy>::new(&connection)
        .destination(DESTINATION)?
        .path(format!("/org/freedesktop/portal/desktop/request/{sender_name}/{request_token}"))?
        .interface("org.freedesktop.portal.Request")?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut response = request.receive_signal("Response").await?;
    let _: OwnedObjectPath = location
        .call("Start", &(&session, "", HashMap::from([("handle_token", zvariant::Value::from(request_token))])))
        .await?;
    let (status, _): (u32, HashMap<String, OwnedValue>) =
        response.next().await.ok_or("Location portal returned no response")?.body().deserialize()?;
    if status != 0 {
        return Err(format!("Location request failed with status {status}").into());
    }

    while let Some(update) = updates.next().await {
        let (_, location): (OwnedObjectPath, HashMap<String, OwnedValue>) = update.body().deserialize()?;
        if sender
            .send([f64::try_from(&location["Latitude"])? as f32, f64::try_from(&location["Longitude"])? as f32])
            .is_err()
        {
            break;
        }
    }
    connection.call_method(Some(DESTINATION), session, Some("org.freedesktop.portal.Session"), "Close", &()).await?;
    Ok(())
}

fn launcher_socket_path() -> PathBuf {
    PathBuf::from(env::var_os("XDG_RUNTIME_DIR").expect("Wayland session requires XDG_RUNTIME_DIR"))
        .join(LAUNCHER_SOCKET_NAME)
}

fn monitor_status(updates: &AppUpdater) {
    let temperatures: Vec<_> = directory_paths("/sys/class/hwmon")
        .filter(|path| {
            fs::read_to_string(path.join("name"))
                .is_ok_and(|name| matches!(name.trim(), "coretemp" | "k10temp" | "zenpower" | "cpu_thermal"))
        })
        .flat_map(directory_paths)
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.as_encoded_bytes();
                name.starts_with(b"temp") && name.ends_with(b"_input")
            })
        })
        .collect();
    let amd: Vec<_> = directory_paths("/sys/class/drm")
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                name.as_encoded_bytes()
                    .strip_prefix(b"card")
                    .is_some_and(|index| !index.is_empty() && index.iter().all(u8::is_ascii_digit))
            })
        })
        .map(|path| path.join("device"))
        .filter(|path| path.join("mem_info_vram_total").exists())
        .map(|path| {
            let temperature = directory_paths(path.join("hwmon")).next().map(|path| path.join("temp1_input"));
            (path, temperature)
        })
        .collect();
    let nvml = Nvml::init().ok();
    let nvidia: Vec<_> = nvml
        .as_ref()
        .into_iter()
        .flat_map(|nvml| (0..nvml.device_count().unwrap_or(0)).filter_map(|index| nvml.device_by_index(index).ok()))
        .collect();
    let battery = directory_paths("/sys/class/power_supply")
        .find(|path| fs::read_to_string(path.join("type")).is_ok_and(|kind| kind.trim() == "Battery"));
    let counters = || fs::read_to_string("/proc/stat").ok().and_then(|stat| cpu_counters(&stat));
    let mut previous = counters();
    loop {
        let current = counters();
        let usage = previous.zip(current).map_or(0.0, |((busy, total), (next_busy, next_total))| {
            (next_busy.saturating_sub(busy) as f32 / next_total.saturating_sub(total).max(1) as f32).clamp(0.0, 1.0)
        });
        previous = current;
        let memory = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let memory_value = |name| {
            memory
                .lines()
                .find_map(|line| line.strip_prefix(name)?.split_ascii_whitespace().next()?.parse::<f32>().ok())
                .unwrap_or_default()
        };
        let total = memory_value("MemTotal:");
        let cpu = ProcessorSample {
            temperature: temperatures.iter().filter_map(read_number).fold(0.0, f32::max) / 1000.0,
            usage,
            memory: ((total - memory_value("MemAvailable:")) / total.max(1.0)).clamp(0.0, 1.0),
        };
        let gpu = nvidia
            .iter()
            .filter_map(|device| {
                let memory = device.memory_info().ok()?;
                Some((memory.total as f32, ProcessorSample {
                    temperature: device.temperature(TemperatureSensor::Gpu).unwrap_or_default() as f32,
                    usage: device.utilization_rates().map_or(0.0, |rates| rates.gpu as f32 / 100.0),
                    memory: memory.used as f32 / memory.total.max(1) as f32,
                }))
            })
            .chain(amd.iter().filter_map(|(path, temperature)| {
                let total = read_number(path.join("mem_info_vram_total"))?;
                Some((total, ProcessorSample {
                    temperature: temperature.as_ref().and_then(read_number).unwrap_or_default() / 1000.0,
                    usage: read_number(path.join("gpu_busy_percent")).unwrap_or_default() / 100.0,
                    memory: read_number(path.join("mem_info_vram_used")).unwrap_or_default() / total.max(1.0),
                }))
            }))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, sample)| sample);
        let battery_level = battery_sample(battery.as_deref());
        if !send_update(updates, move |app| {
            if let Some(status) = &mut app.bar.status {
                status.record(SystemSample { cpu, gpu, battery_level });
            }
        }) {
            break;
        }
        thread::sleep(super::Platform::STATUS_SAMPLE_INTERVAL);
    }
}

fn directory_paths(path: impl AsRef<Path>) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(path).into_iter().flatten().flatten().map(|entry| entry.path())
}

fn read_number(path: impl AsRef<Path>) -> Option<f32> {
    fs::read_to_string(path).ok()?.trim().parse::<f32>().ok().filter(|value| value.is_finite())
}

fn cpu_counters(stat: &str) -> Option<(u64, u64)> {
    let mut fields = stat.lines().next()?.split_ascii_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let mut values = [0u64; 8];
    for value in &mut values {
        *value = fields.next()?.parse().ok()?;
    }
    // Guest time is already included in user/nice; idle and iowait are not busy time.
    let total = values.iter().sum::<u64>();
    Some((total - values[3] - values[4], total))
}

/// Charge level, negated while charging, or absent with no battery or idle at full.
fn battery_sample(path: Option<&Path>) -> Option<f32> {
    let path = path?;
    let read = |name: &str| fs::read_to_string(path.join(name));
    let level = read("capacity").ok()?.trim().parse::<f32>().ok()? / 100.0;
    if read("status").is_ok_and(|status| status.trim().eq_ignore_ascii_case("charging")) {
        Some(-level.max(f32::EPSILON))
    } else if level >= 0.995 {
        None
    } else {
        Some(level)
    }
}

fn monitor_playback(levels: &[AtomicU32; AUDIO_SPECTRUM_BANDS]) {
    loop {
        if let Err(error) = capture_playback(levels) {
            warn!(%error, "PipeWire playback meter stopped");
        }
        for level in levels {
            level.store(0.0f32.to_bits(), Ordering::Relaxed);
        }
        thread::sleep(Duration::from_secs(1));
    }
}

#[derive(Default)]
struct PipeWireState {
    default_sink: Option<String>,
    sinks: HashMap<String, f32>,
}

impl PipeWireState {
    fn update(&mut self, object: &Value) -> Option<f32> {
        if let Some(metadata) = object["metadata"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["key"] == "default.audio.sink"))
        {
            self.default_sink = metadata["value"]["name"].as_str().map(str::to_owned);
            return self.sinks.get(self.default_sink.as_ref()?).copied();
        }
        let info = &object["info"];
        if info["props"]["media.class"] == "Audio/Sink"
            && let Some(name) = info["props"]["node.name"].as_str()
            && let Some(props) = info["params"]["Props"]
                .as_array()
                .and_then(|items| items.iter().find(|props| props["channelVolumes"].is_array()))
        {
            let volumes = props["channelVolumes"].as_array().unwrap();
            let mut volume =
                (volumes.iter().filter_map(Value::as_f64).sum::<f64>() / volumes.len().max(1) as f64).cbrt() as f32;
            if props["mute"].as_bool().unwrap_or_default() {
                volume = -volume;
            }
            self.sinks.insert(name.to_owned(), volume);
            return (Some(name) == self.default_sink.as_deref()).then_some(volume);
        }
        None
    }
}

fn monitor_volume(volume: &AtomicU32) {
    loop {
        if let Err(error) = capture_volume(volume) {
            warn!(%error, "PipeWire volume monitor stopped");
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn with_output(command: &mut Command, read: impl FnOnce(process::ChildStdout) -> io::Result<()>) -> io::Result<()> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let result = read(child.stdout.take().expect("stdout was piped"));
    let _ = child.kill();
    child.wait()?;
    result
}

fn capture_volume(volume: &AtomicU32) -> io::Result<()> {
    with_output(Command::new("pw-dump").args(["--monitor", "--no-colors", "--indent", "0"]), |output| {
        let mut state = PipeWireState::default();
        for batch in serde_json::Deserializer::from_reader(output).into_iter::<Vec<Value>>() {
            for object in batch.map_err(io::Error::other)? {
                if let Some(level) = state.update(&object) {
                    volume.store(level.to_bits(), Ordering::Relaxed);
                }
            }
        }
        Ok(())
    })
}

fn capture_playback(levels: &[AtomicU32; AUDIO_SPECTRUM_BANDS]) -> io::Result<()> {
    with_output(
        Command::new("pw-record").args([
            "--properties",
            "stream.capture.sink=true",
            "--rate",
            &AUDIO_SAMPLE_RATE.to_string(),
            "--channels",
            "1",
            "--format",
            "f32",
            "--raw",
            "-",
        ]),
        |mut output| {
            let mut window = [0.0; AUDIO_WINDOW_SIZE];
            loop {
                match output.read_exact(bytemuck::cast_slice_mut(&mut window)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(error),
                }
                let spectrum = rfft_1024(&mut window);
                for (band, level) in levels.iter().enumerate() {
                    let bin = |frequency: f32| {
                        (frequency * AUDIO_WINDOW_SIZE as f32 / AUDIO_SAMPLE_RATE as f32).ceil() as usize
                    };
                    let bins = &spectrum[bin(AUDIO_BAND_EDGES[band])..bin(AUDIO_BAND_EDGES[band + 1])];
                    let rms = (bins.iter().map(microfft::Complex32::norm_sqr).sum::<f32>()
                        / bins.len() as f32
                        / AUDIO_WINDOW_SIZE as f32)
                        .sqrt();
                    let value = ((20.0 * rms.log10() + 30.0) / 30.0).saturate();
                    level.store(value.to_bits(), Ordering::Relaxed);
                }
            }
            Ok(())
        },
    )
}

#[derive(Clone, Copy)]
struct NativeSurface {
    display: NonNull<c_void>,
    window: NonNull<c_void>,
}

impl HasDisplayHandle for NativeSurface {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display));
        // SAFETY: LayerShellApp owns the Wayland connection that supplied this live display pointer.
        Ok(unsafe { DisplayHandle::borrow_raw(handle) })
    }
}

impl HasWindowHandle for NativeSurface {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(self.window));
        // SAFETY: LayerShellApp owns the live wl_surface represented by this pointer.
        Ok(unsafe { WindowHandle::borrow_raw(handle) })
    }
}

struct LayerShellApp {
    // Drop GPU surfaces before destroying the Wayland proxies.
    gpu: Option<Renderer> = None,
    cantus: CantusApp,

    should_exit: bool = false,

    compositor: WlCompositor,
    layer_shell: ZwlrLayerShellV1,
    pointer: Option<WlPointer> = None,
    keyboard: Option<WlKeyboard> = None,
    xkb_state: Option<xkb::State> = None,
    /// Repeat timing advertised by the compositor; zero disables repetition.
    repeat_delay: Duration = Duration::ZERO,
    repeat_interval: Duration = Duration::ZERO,
    /// The held key waiting to repeat and when it next fires, pumped each frame.
    repeat: Option<(xkb::Keycode, Instant)> = None,
    /// Latest keyboard serial, which the compositor requires to claim the selection.
    key_serial: u32 = 0,
    clipboard: (WlDataDeviceManager, WlDataDevice),
    /// The selection offer to read on paste, kept only while it advertises text.
    selection: Option<WlDataOffer> = None,
    output: Option<WlOutput> = None,
    frame_callback: Option<WlCallback> = None,
    display_handle: NonNull<c_void>,
    surfaces: [Option<WaylandSurface>; 2] = [None, None],
    scaling: (WpViewporter, WpFractionalScaleManagerV1),
    background_manager: Option<ExtBackgroundEffectManagerV1>,
}

#[derive(Clone, Copy)]
enum SurfaceKind {
    Bar,
    Launcher,
}

struct WaylandSurface {
    wl: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    viewport: WpViewport,
    fractional: WpFractionalScaleV1,
    effect: Option<ExtBackgroundEffectSurfaceV1>,
    size: Vec2,
    scale: f32,
    configured: bool,
    gpu: Option<SurfaceHandle>,
    blur_bounds: Option<(Vec2, Vec2)>,
}

impl Drop for WaylandSurface {
    fn drop(&mut self) {
        self.layer.destroy();
        self.viewport.destroy();
        self.fractional.destroy();
        if let Some(effect) = &self.effect {
            effect.destroy();
        }
        self.wl.destroy();
    }
}

macro_rules! dispatch {
    ($proxy:ty, |$state:ident, $object:ident, $value:ident, $queue:ident| $body:block) => {
        dispatch!($proxy, (), _data, |$state, $object, $value, $queue| $body);
    };
    ($proxy:ty, $data_type:ty, $data:ident, |$state:ident, $object:ident, $value:ident, $queue:ident| $body:block) => {
        impl Dispatch<$proxy, $data_type> for LayerShellApp {
            fn event(
                $state: &mut Self,
                $object: &$proxy,
                $value: <$proxy as Proxy>::Event,
                $data: &$data_type,
                _conn: &Connection,
                $queue: &QueueHandle<Self>,
            ) $body
        }
    };
}

impl LayerShellApp {
    fn bar_surface_height(&self) -> f32 {
        let extension = if self.cantus.config.weathertime_enabled {
            weathertime::EXTENSION
        } else if self.cantus.config.lyrics_enabled {
            lyrics::EXTENSION
        } else {
            0.0
        } + PANEL_OVERFLOW;
        self.cantus.config.height + PANEL_START + extension
    }

    fn create_surface(&self, kind: SurfaceKind, qhandle: &QueueHandle<Self>) -> WaylandSurface {
        let launcher = matches!(kind, SurfaceKind::Launcher);
        let config = &self.cantus.config;
        let wl = self.compositor.create_surface(qhandle, kind);
        let layer = self.layer_shell.get_layer_surface(
            &wl,
            if launcher { None } else { self.output.as_ref() },
            if launcher {
                Layer::Overlay
            } else {
                match config.layer {
                    config::Layer::Background => Layer::Background,
                    config::Layer::Bottom => Layer::Bottom,
                    config::Layer::Top => Layer::Top,
                    config::Layer::Overlay => Layer::Overlay,
                }
            },
            if launcher { "cantus-launcher" } else { "cantus" }.into(),
            qhandle,
            kind,
        );
        layer.set_anchor(
            Anchor::Left
                | Anchor::Right
                | if launcher {
                    Anchor::Top | Anchor::Bottom
                } else {
                    match config.layer_anchor {
                        config::LayerAnchor::Top => Anchor::Top,
                        config::LayerAnchor::Bottom => Anchor::Bottom,
                    }
                },
        );
        let height = if launcher { 0.0 } else { self.bar_surface_height() };
        layer.set_size(0, height as u32);
        layer.set_exclusive_zone(if launcher {
            0
        } else {
            (PANEL_START + config.height + f32::from(config.lyrics_enabled) * lyrics::EXTENSION) as i32
        });
        layer.set_keyboard_interactivity(if launcher {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        let surface = WaylandSurface {
            viewport: self.scaling.0.get_viewport(&wl, qhandle, ()),
            fractional: self.scaling.1.get_fractional_scale(&wl, qhandle, kind),
            effect: self.background_manager.as_ref().map(|manager| manager.get_background_effect(&wl, qhandle, ())),
            wl,
            layer,
            size: vec2(0.0, height),
            scale: 1.0,
            configured: false,
            gpu: None,
            blur_bounds: None,
        };
        surface.wl.commit();
        surface
    }

    /// Re-fires the held key for as long as it stays down, once the initial delay has passed.
    fn pump_key_repeat(&mut self) {
        let now = Instant::now();
        while let Some((keycode, next)) = self.repeat
            && next <= now
            && self.cantus.launcher.open
        {
            self.repeat = Some((keycode, next + self.repeat_interval));
            handle_launcher_key(self, keycode);
        }
    }

    /// Reads clipboard data off the event loop so this client can also serve its own selection.
    fn paste(&self) -> Option<()> {
        let offer = self
            .selection
            .as_ref()
            .filter(|offer| offer.data::<AtomicBool>().is_some_and(|text| text.load(Ordering::Relaxed)))?;
        let session = self.cantus.launcher.session;
        let (mut reader, writer) = io::pipe().ok()?;
        offer.receive(TEXT_MIME.to_owned(), writer.as_fd());
        drop(writer); // Close the local write end so the reader can reach EOF.
        Connection::from_backend(offer.backend().upgrade()?).flush().ok()?;
        self.cantus.enrichment.background.spawn_update(async move {
            let text = spawn_blocking(move || {
                let mut text = String::new();
                reader.read_to_string(&mut text).map(|_| text.replace(['\n', '\r'], " "))
            })
            .await
            .ok()?
            .ok()?;
            Some(update(move |app| {
                if app.launcher.open && app.launcher.session == session {
                    app.launcher.edit(|field| field.insert(&text));
                }
            }))
        });
        Some(())
    }

    fn active_surface(&self) -> &WaylandSurface {
        self.surfaces[1].as_ref().or(self.surfaces[0].as_ref()).expect("bar surface exists")
    }

    fn sync_launcher_surface(&mut self, qhandle: &QueueHandle<Self>) {
        let open = self.cantus.launcher.open;
        if open == self.surfaces[1].is_some() {
            return;
        }
        self.cantus.interaction = Interaction::default();
        self.repeat = None;
        self.frame_callback = None;
        if open {
            self.surfaces[1] = Some(self.create_surface(SurfaceKind::Launcher, qhandle));
        } else if let Some(surface) = self.surfaces[1].take()
            && let (Some(gpu), Some(handle)) = (&mut self.gpu, surface.gpu)
        {
            gpu.remove_surface(handle);
        }
    }

    fn try_render_frame(&mut self, qhandle: &QueueHandle<Self>) {
        self.pump_key_repeat();
        self.cantus.apply_pending_updates();
        self.sync_launcher_surface(qhandle);
        if self.frame_callback.is_some()
            || self.surfaces[0].is_none()
            || self.surfaces.iter().flatten().any(|surface| !surface.configured)
        {
            return;
        }
        for surface in self.surfaces.iter_mut().flatten() {
            let native = NativeSurface {
                display: self.display_handle,
                window: NonNull::new(surface.wl.id().as_ptr().cast()).expect("Wayland surface pointer"),
            };
            let size = (surface.size * surface.scale).round().to_array().map(|size| size as u32);
            surface.viewport.set_destination(surface.size.x as i32, surface.size.y as i32);
            if self.gpu.is_none() {
                // SAFETY: The renderer is dropped before the owned Wayland surfaces.
                let (gpu, handle) = unsafe {
                    Renderer::new(
                        &native,
                        size,
                        TextCache::new(include_bytes!("../../../../assets/NotoSans-Variable.ttf"), TEXT_COLOR),
                    )
                }
                .expect("failed to initialize renderer");
                tracing::info!("Using GPU device: {}", gpu.device_name());
                self.gpu = Some(gpu);
                surface.gpu = Some(handle);
            }
            let gpu = self.gpu.as_mut().unwrap();
            let handle = *surface.gpu.get_or_insert_with(|| {
                // SAFETY: The GPU surface is removed before the Wayland surface is destroyed.
                unsafe { gpu.add_surface(&native, size) }.expect("incompatible surface")
            });
            gpu.resize(handle, size);
        }
        if let Err(error) = self.gpu.as_mut().unwrap().render(|render| {
            for (index, surface) in self.surfaces.iter().enumerate() {
                if let Some(surface) = surface {
                    render.surface(surface.gpu.unwrap(), surface.size, |frame| {
                        self.cantus.draw(frame, index == 0, index == 1);
                    });
                }
            }
        }) {
            tracing::error!(%error, "Could not render frame");
        }
        self.update_input_region(qhandle);
        self.update_blur_region(qhandle);
        self.frame_callback = Some(self.active_surface().wl.frame(qhandle, ()));
        self.active_surface().wl.commit();
        if let Some(text) = self.cantus.launcher.pending_copy.take() {
            let source = self.clipboard.0.create_data_source(qhandle, Arc::<str>::from(text));
            source.offer(TEXT_MIME.to_owned());
            self.clipboard.1.set_selection(Some(&source), self.key_serial);
        }
    }

    fn update_input_region(&mut self, qhandle: &QueueHandle<Self>) {
        let wl_surface = self.active_surface().wl.clone();
        let region = self.compositor.create_region(qhandle, ());
        for quad in self.cantus.interaction.input_regions.drain(..) {
            let (min, max) = quad.extents();
            let min = min.floor();
            let size = max.ceil() - min;
            region.add(min.x as i32, min.y as i32, size.x as i32, size.y as i32);
        }
        wl_surface.set_input_region(Some(&region));
        region.destroy();
    }

    fn update_blur_region(&mut self, qhandle: &QueueHandle<Self>) {
        let Some(surface) = self.surfaces[1].as_mut() else { return };
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
        let radius = (BACKGROUND_RADIUS - 1).min(width / 2).min(height / 2);
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

dispatch!(ZwlrLayerSurfaceV1, SurfaceKind, kind, |state, proxy, event, qhandle| {
    let Some(surface) = state.surfaces[*kind as usize].as_mut().filter(|surface| surface.layer == *proxy) else {
        return;
    };
    match event {
        zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
            proxy.ack_configure(serial);
            if width > 0 {
                surface.size.x = width as f32;
            }
            if height > 0 {
                surface.size.y = height as f32;
            }
            surface.configured = surface.size.min_element() > 0.0;
        }
        zwlr_layer_surface_v1::Event::Closed => {
            if matches!(kind, SurfaceKind::Bar) {
                state.should_exit = true;
                return;
            }
            state.cantus.launcher.open = false;
        }
        _ => return,
    }
    state.try_render_frame(qhandle);
});

dispatch!(WlCallback, |state, proxy, event, qhandle| {
    if matches!(event, wl_callback::Event::Done { .. })
        && state.frame_callback.as_ref().is_some_and(|callback| callback.id() == proxy.id())
    {
        state.frame_callback = None;
        state.try_render_frame(qhandle);
    }
});

dispatch!(WpFractionalScaleV1, SurfaceKind, kind, |state, proxy, event, qhandle| {
    if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
        && let Some(surface) = state.surfaces[*kind as usize].as_mut()
        && surface.fractional == *proxy
    {
        surface.scale = scale as f32 / 120.0;
        state.try_render_frame(qhandle);
    }
});

dispatch!(WlOutput, |state, proxy, event, _qhandle| {
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

dispatch!(WlSeat, |state, proxy, event, qhandle| {
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

impl Dispatch<WlDataDevice, ()> for LayerShellApp {
    event_created_child!(Self, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, AtomicBool::new(false)),
    ]);

    fn event(
        state: &mut Self,
        _proxy: &WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let wl_data_device::Event::Selection { id } = event {
            if let Some(stale) = state.selection.take() {
                stale.destroy();
            }
            state.selection = id;
        }
    }
}

dispatch!(WlDataOffer, AtomicBool, text, |_state, _proxy, event, _qhandle| {
    if let wl_data_offer::Event::Offer { mime_type } = event
        && mime_type == TEXT_MIME
    {
        text.store(true, Ordering::Relaxed);
    }
});

dispatch!(WlDataSource, Arc<str>, text, |_state, proxy, event, _qhandle| {
    match event {
        wl_data_source::Event::Send { fd, .. } => {
            let text = Arc::clone(text);
            spawn_thread("cantus-clipboard", move || {
                if let Err(error) = File::from(fd).write_all(text.as_bytes()) {
                    warn!(%error, "Failed to serve the clipboard selection");
                }
            });
        }
        wl_data_source::Event::Cancelled => proxy.destroy(),
        _ => {}
    }
});

dispatch!(WlKeyboard, |state, _proxy, event, _qhandle| {
    match event {
        wl_keyboard::Event::Keymap { format: WEnum::Value(KeymapFormat::XkbV1), fd, size } => {
            let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            // SAFETY: Wayland supplied fd and size for an XKB keymap in the declared format.
            let keymap = unsafe {
                xkb::Keymap::new_from_fd(
                    &context,
                    fd,
                    size as usize,
                    xkb::KEYMAP_FORMAT_TEXT_V1,
                    xkb::KEYMAP_COMPILE_NO_FLAGS,
                )
            };
            state.xkb_state = keymap.ok().flatten().map(|keymap| xkb::State::new(&keymap));
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
        wl_keyboard::Event::Key { serial, key, state: WEnum::Value(key_state), .. } => {
            let keycode = xkb::Keycode::new(key + 8);
            state.key_serial = serial;
            if key_state == KeyState::Pressed {
                // Modifiers are marked as non-repeating by the keymap, so they never latch here.
                let repeats =
                    state.xkb_state.as_ref().is_some_and(|xkb_state| xkb_state.get_keymap().key_repeats(keycode));
                state.repeat = (repeats && !state.repeat_interval.is_zero())
                    .then(|| (keycode, Instant::now() + state.repeat_delay));
                handle_launcher_key(state, keycode);
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
fn handle_launcher_key(state: &mut LayerShellApp, keycode: xkb::Keycode) {
    if !state.cantus.launcher.open {
        return;
    }
    let Some(xkb_state) = &state.xkb_state else {
        return;
    };
    let sym = xkb_state.key_get_one_sym(keycode);
    if !state.surfaces[1].as_ref().is_some_and(|surface| surface.configured)
        && matches!(sym.raw(), xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter)
    {
        return;
    }
    let shift = xkb_state.mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE);
    let control = xkb_state.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE);
    let character = sym.key_char();
    // Held control turns `key_char` into a control code, so the shortcuts read the keysym instead.
    let letter = char::from_u32(sym.raw()).filter(char::is_ascii_alphabetic).map(|letter| letter.to_ascii_lowercase());

    if control && letter == Some('v') {
        state.paste();
        return;
    }
    let key = match sym.raw() {
        xkb::keysyms::KEY_Escape => Some(LauncherKey::Escape),
        xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter => Some(LauncherKey::Activate),
        xkb::keysyms::KEY_Up => Some(LauncherKey::Up),
        xkb::keysyms::KEY_Down => Some(LauncherKey::Down),
        xkb::keysyms::KEY_BackSpace => Some(LauncherKey::Backspace),
        xkb::keysyms::KEY_Delete => Some(LauncherKey::Delete),
        xkb::keysyms::KEY_Left => Some(LauncherKey::Left),
        xkb::keysyms::KEY_Right => Some(LauncherKey::Right),
        xkb::keysyms::KEY_Home => Some(LauncherKey::Home),
        xkb::keysyms::KEY_End => Some(LauncherKey::End),
        _ if control && letter == Some('a') => Some(LauncherKey::SelectAll),
        _ if control && letter == Some('c') => Some(LauncherKey::Copy),
        _ if control && letter == Some('x') => Some(LauncherKey::Cut),
        _ => None,
    };
    if let Some(key) = key {
        state.cantus.launcher.key(key, shift);
    } else if let Some(typed) = character.filter(|typed| !typed.is_control() && !control) {
        state.cantus.launcher.edit(|field| field.insert(typed.encode_utf8(&mut [0u8; 4])));
    }
}

dispatch!(WlPointer, |state, _proxy, event, _qhandle| {
    if state.gpu.is_none() {
        return;
    }
    let surface_id = state.active_surface().wl.id();
    let interaction = &mut state.cantus.interaction;
    match event {
        wl_pointer::Event::Enter { surface: wl_surface, surface_x, surface_y, .. } if surface_id == wl_surface.id() => {
            interaction.apply(InputEvent::Enter(vec2(surface_x as f32, surface_y as f32)));
        }
        wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
            let position = vec2(surface_x as f32, surface_y as f32);
            interaction.apply(InputEvent::Motion(position));
        }
        wl_pointer::Event::Leave { .. } => {
            interaction.apply(InputEvent::Leave);
        }
        wl_pointer::Event::Button { button, state: button_state, .. } => match (button, button_state) {
            (0x110, WEnum::Value(wl_pointer::ButtonState::Pressed)) => {
                interaction.apply(InputEvent::Press);
            }
            (0x110, WEnum::Value(wl_pointer::ButtonState::Released)) => {
                interaction.apply(InputEvent::Release);
            }
            (0x111, WEnum::Value(wl_pointer::ButtonState::Pressed)) if interaction.dragging() => {
                interaction.apply(InputEvent::Cancel);
            }
            _ => {}
        },
        wl_pointer::Event::AxisValue120 {
            axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
            value120: discrete,
            ..
        } if discrete != 0 => {
            interaction.apply(InputEvent::Scroll(discrete.signum()));
        }
        _ => {}
    }
});

dispatch!(WlRegistry, GlobalListContents, _globals, |_state, _proxy, _event, _qhandle| {});

delegate_noop!(LayerShellApp: ignore ZwlrLayerShellV1);
delegate_noop!(LayerShellApp: ignore WpFractionalScaleManagerV1);
delegate_noop!(LayerShellApp: ignore WpViewporter);
delegate_noop!(LayerShellApp: ignore WpViewport);
delegate_noop!(LayerShellApp: ignore WlCompositor);
delegate_noop!(LayerShellApp: ignore WlRegion);
delegate_noop!(LayerShellApp: ignore WlDataDeviceManager);
delegate_noop!(LayerShellApp: ignore ExtBackgroundEffectManagerV1);
delegate_noop!(LayerShellApp: ignore ExtBackgroundEffectSurfaceV1);

dispatch!(WlSurface, SurfaceKind, _kind, |_state, _proxy, _event, _qhandle| {});
