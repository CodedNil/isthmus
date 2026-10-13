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
        "Initial transport + two bounces: {:?}; {} probes, {} fixtures",
        started.elapsed(),
        scene.probes.len(),
        scene.lights.len()
    );
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
    println!("Entity change + two bounce blend: {:?} average", dimming / 20);
}
