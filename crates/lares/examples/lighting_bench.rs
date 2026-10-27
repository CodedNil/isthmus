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
        "Initial transport + four bounces: {:?}; {} active / {} lattice probe sites, {} fixtures",
        started.elapsed(),
        scene.probes.iter().filter(|probe| probe.positive.x >= 0.0).count(),
        scene.probes.len(),
        scene.lights.len()
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
    let mut dimming = Duration::ZERO;
    for state in 0..20 {
        let started = Instant::now();
        scene.set_light_state("light.lounge_left_downlights", f32::from(state % 2 == 0));
        black_box(&scene);
        dimming += started.elapsed();
    }
    println!("Entity change + four bounce blend: {:?} average", dimming / 20);
}
