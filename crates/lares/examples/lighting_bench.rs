//! CPU cost of cached GI and the bounded daylight update; excludes GPU frame time.
//! Pass `--startup` to measure loading baked lighting instead.
use lares::{
    home,
    render::{Scene, SunSettings},
};
use std::{
    env, fs,
    hint::black_box,
    time::{Duration, Instant},
};
use tracing::info;

fn main() {
    lares::init_logging();
    if env::args().any(|arg| arg == "--startup") {
        startup();
        return;
    }
    let noon = 1_782_043_200.0; // 2026-06-21 12:00 UTC
    let started = Instant::now();
    let mut scene = Scene::new_at(&home::rooms(), noon, SunSettings::default());
    info!(
        "Initial transport + eight bounces: {:?}; {} active / {} allocated cache sites, {} fixtures",
        started.elapsed(),
        scene.probes.iter().filter(|probe| probe.positive.x >= 0.0).count(),
        scene.probes.len(),
        scene.lights.len()
    );
    info!("Transport paths, volume taps, bytes: {:?}", scene.transport_statistics());
    let lattice = (scene.grid.size.x * scene.grid.size.y * scene.grid.size.z) as usize;
    let volume = scene.probes[..lattice].iter().filter(|probe| probe.positive.x >= 0.0).count();
    let surfaces = scene.probes[lattice..].iter().filter(|probe| probe.normal.length_squared() > 0.5).count();
    info!(
        "GI: {volume}/{lattice} indoor volume sites (50 cm), {surfaces}/{} surface sites (20 cm); {:.2} MiB GPU cache",
        scene.probes.len() - lattice,
        size_of_val(&*scene.probes) as f64 / 1_048_576.0
    );
    let slots = scene.lights.len().min(32);
    let active = if slots == 32 { u32::MAX } else { (1u32 << slots) - 1 };
    let rejected: u32 = scene.light_masks.iter().map(|mask| ((!mask) & active).count_ones()).sum();
    info!("Conservative light culling: {rejected}/{} object/light pairs", scene.objects.len() * slots);
    let mut updates = Vec::new();
    for frame in 0..600 {
        let started = Instant::now();
        scene.update_time(noon + 60.0 + f64::from(frame) / 60.0);
        black_box(&scene);
        updates.push(started.elapsed());
    }
    updates.sort_unstable();
    info!("Daylight update CPU: median {:?}, p95 {:?}, max {:?}", updates[300], updates[570], updates[599]);
    let cold = Instant::now();
    scene.set_light_state("light.lounge_left_downlights", 0.0);
    info!("First entity response: {:?}", cold.elapsed());
    let mut dimming = Duration::ZERO;
    for state in 0..20 {
        let started = Instant::now();
        scene.set_light_state("light.lounge_left_downlights", f32::from(state % 2 == 0));
        black_box(&scene);
        dimming += started.elapsed();
    }
    info!("Cached entity field update: {:?} average", dimming / 20);
}

fn startup() {
    let start = Instant::now();
    let bytes = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/baked/lighting.bin")).expect("lighting asset");
    info!("Read {} bytes: {:?}", bytes.len(), start.elapsed());
    let rooms = home::rooms();
    for (label, time) in [("night", 1_782_000_000.0), ("noon", 1_782_043_200.0)] {
        let start = Instant::now();
        let scene = Scene::from_baked_at(&rooms, time, SunSettings::default(), &bytes).expect("current lighting asset");
        info!(
            "{label}: {:?}; {} probes; paths/taps/bytes {:?}",
            start.elapsed(),
            scene.probes.len(),
            scene.transport_statistics()
        );
    }
}
