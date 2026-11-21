use super::spawn_thread;
use crate::{
    app::{AppUpdater, send_update},
    render::status::{AUDIO_SPECTRUM_BANDS, AudioMonitor, ProcessorSample, SystemSample},
};
use isthmus::glam::FloatExt as _;
use microfft::real::rfft_1024;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::watch;
use tracing::warn;

pub const STATUS_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const AUDIO_SAMPLE_RATE: u32 = 48_000;
const AUDIO_WINDOW_SIZE: usize = 1024;
/// Decibel range mapped to 0..1; wide, because the display rescales it.
const AUDIO_FLOOR_DB: f32 = -100.0;
const AUDIO_RANGE_DB: f32 = 94.0;
const AUDIO_BAND_EDGES: [f32; AUDIO_SPECTRUM_BANDS + 1] =
    [60.0, 120.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 12_000.0];

pub fn start_status_monitor(updates: AppUpdater, audio: Arc<AudioMonitor>) {
    spawn_thread("cantus-audio-playback", move || monitor_playback(&audio.spectrum));
    spawn_thread("cantus-system-status", move || monitor_status(&updates));
}

pub fn monitor_audio(audio: Option<Arc<AudioMonitor>>) -> watch::Receiver<bool> {
    let (playing, receiver) = watch::channel(false);
    spawn_thread("cantus-pipewire", move || monitor_pipewire(audio.as_deref(), &playing));
    receiver
}

pub fn set_volume(volume: f32) {
    let volume = format!("{volume:.3}");
    if let Err(error) = Command::new("wpctl").args(["set-volume", "@DEFAULT_AUDIO_SINK@", &volume]).spawn() {
        warn!(%error, "Failed to set PipeWire volume");
    }
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
    let nvidia = directory_paths("/sys/bus/pci/devices").any(|path| {
        fs::read_to_string(path.join("vendor")).is_ok_and(|vendor| vendor.trim() == "0x10de")
            && fs::read_to_string(path.join("class")).is_ok_and(|class| class.starts_with("0x03"))
    });
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
            .then(nvidia_samples)
            .into_iter()
            .flatten()
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
        thread::sleep(STATUS_SAMPLE_INTERVAL);
    }
}

fn nvidia_samples() -> Vec<(f32, ProcessorSample)> {
    let Ok(output) = Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total,memory.used,temperature.gpu,utilization.gpu", "--format=csv,noheader,nounits"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut values = line.split(',').map(|value| value.trim().parse::<f32>().ok());
            let total = values.next()??;
            let used = values.next()??;
            Some((total * 1_048_576.0, ProcessorSample {
                temperature: values.next().flatten().unwrap_or_default(),
                usage: values.next().flatten().unwrap_or_default() / 100.0,
                memory: used / total.max(1.0),
            }))
        })
        .collect()
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

fn monitor_playback(levels: &[AtomicU32; AUDIO_SPECTRUM_BANDS]) -> ! {
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

fn with_output(command: &mut Command, read: impl FnOnce(process::ChildStdout) -> io::Result<()>) -> io::Result<()> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let result = read(child.stdout.take().expect("stdout was piped"));
    let _kill = child.kill();
    child.wait()?;
    result
}

fn monitor_pipewire(audio: Option<&AudioMonitor>, playing: &watch::Sender<bool>) -> ! {
    loop {
        let result =
            with_output(Command::new("pw-dump").args(["--monitor", "--no-colors", "--indent", "0"]), |output| {
                let mut default_sink = None;
                let mut sinks = HashMap::new();
                let mut running_streams = HashSet::new();
                for batch in serde_json::Deserializer::from_reader(output).into_iter::<Vec<Value>>() {
                    for object in batch.map_err(io::Error::other)? {
                        if let Some(id) = object["id"].as_u64() {
                            let info = &object["info"];
                            if info.is_null() || info["state"].as_str().is_some_and(|state| state != "running") {
                                running_streams.remove(&id);
                            } else if info["state"] == "running"
                                && info["props"]["media.class"] == "Stream/Output/Audio"
                            {
                                running_streams.insert(id);
                            }
                        }
                        if let Some(metadata) = object["metadata"]
                            .as_array()
                            .and_then(|items| items.iter().find(|item| item["key"] == "default.audio.sink"))
                        {
                            default_sink = metadata["value"]["name"].as_str().map(str::to_owned);
                        } else {
                            let info = &object["info"];
                            if info["props"]["media.class"] != "Audio/Sink" {
                                continue;
                            }
                            let Some(name) = info["props"]["node.name"].as_str() else { continue };
                            let Some(props) = info["params"]["Props"]
                                .as_array()
                                .and_then(|items| items.iter().find(|props| props["channelVolumes"].is_array()))
                            else {
                                continue;
                            };
                            let volumes = props["channelVolumes"].as_array().unwrap();
                            let level = (volumes.iter().filter_map(Value::as_f64).sum::<f64>()
                                / volumes.len().max(1) as f64)
                                .cbrt() as f32;
                            sinks.insert(name.to_owned(), if props["mute"] == true { -level } else { level });
                        }
                        let local_playback = !running_streams.is_empty();
                        playing.send_if_modified(|previous| {
                            let changed = *previous != local_playback;
                            *previous = local_playback;
                            changed
                        });
                        if let Some(audio) = audio
                            && let Some(level) = default_sink.as_ref().and_then(|name| sinks.get(name))
                        {
                            audio.volume.store(level.to_bits(), Ordering::Relaxed);
                        }
                    }
                }
                Ok(())
            });
        if let Err(error) = result {
            warn!(%error, "PipeWire monitor stopped");
        }
        playing.send_replace(false);
        thread::sleep(Duration::from_secs(1));
    }
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
                        / (AUDIO_WINDOW_SIZE * AUDIO_WINDOW_SIZE) as f32)
                        .sqrt();
                    // Wide range; the display rescales per band.
                    let value = ((20.0 * rms.log10() - AUDIO_FLOOR_DB) / AUDIO_RANGE_DB).saturate();
                    level.store(value.to_bits(), Ordering::Relaxed);
                }
            }
            Ok(())
        },
    )
}
