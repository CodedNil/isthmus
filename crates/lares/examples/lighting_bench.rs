//! CPU cost of cached GI and the bounded daylight update; excludes GPU frame time.
use lares::{
    home,
    render::{Scene, SunSettings},
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    let noon = 1_782_043_200.0; // 2026-06-21 12:00 UTC
    let started = Instant::now();
    let mut scene = Scene::new_at(&home::rooms(), noon, SunSettings::default());
    println!(
        "Initial transport + eight bounces: {:?}; {} active / {} allocated cache sites, {} fixtures",
        started.elapsed(),
        scene.probes.iter().filter(|probe| probe.positive.x >= 0.0).count(),
        scene.probes.len(),
        scene.lights.len()
    );
    println!("Transport paths, volume taps, bytes: {:?}", scene.transport_statistics());
    let lattice = (scene.grid.size.x * scene.grid.size.y * scene.grid.size.z) as usize;
    let volume = scene.probes[..lattice].iter().filter(|probe| probe.positive.x >= 0.0).count();
    let surfaces = scene.probes[lattice..].iter().filter(|probe| probe.normal.length_squared() > 0.5).count();
    println!(
        "GI: {volume}/{lattice} indoor volume sites (50 cm), {surfaces}/{} surface sites (20 cm); {:.2} MiB GPU cache",
        scene.probes.len() - lattice,
        size_of_val(&*scene.probes) as f64 / 1_048_576.0
    );
    let slots = scene.lights.len().min(32);
    let active = if slots == 32 { u32::MAX } else { (1u32 << slots) - 1 };
    let rejected: u32 = scene.light_masks.iter().map(|mask| ((!mask) & active).count_ones()).sum();
    println!("Conservative light culling: {rejected}/{} object/light pairs", scene.objects.len() * slots);
    let mut updates = Vec::new();
    for frame in 0..600 {
        let started = Instant::now();
        scene.update_time(noon + 60.0 + f64::from(frame) / 60.0);
        black_box(&scene);
        updates.push(started.elapsed());
    }
    updates.sort_unstable();
    println!("Daylight update CPU: median {:?}, p95 {:?}, max {:?}", updates[300], updates[570], updates[599]);
    let cold = Instant::now();
    scene.set_light_state("light.lounge_left_downlights", 0.0);
    println!("First entity response: {:?}", cold.elapsed());
    let mut dimming = Duration::ZERO;
    for state in 0..20 {
        let started = Instant::now();
        scene.set_light_state("light.lounge_left_downlights", f32::from(state % 2 == 0));
        black_box(&scene);
        dimming += started.elapsed();
    }
    println!("Cached entity field update: {:?} average", dimming / 20);
}
