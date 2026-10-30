//! Measures startup CPU work without conflating it with GPU driver compilation.
use lares::{
    home,
    render::{Scene, SunSettings},
};
use std::{fs, time::Instant};

fn main() {
    let start = Instant::now();
    let bytes = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/baked/lighting.bin")).expect("lighting asset");
    println!("Read {} bytes: {:?}", bytes.len(), start.elapsed());
    let rooms = home::rooms();
    for (label, time) in [("night", 1_782_000_000.0), ("noon", 1_782_043_200.0)] {
        let start = Instant::now();
        let scene = Scene::from_baked_at(&rooms, time, SunSettings::default(), &bytes).expect("current lighting asset");
        println!(
            "{label}: {:?}; {} probes; paths/taps/bytes {:?}",
            start.elapsed(),
            scene.probes.len(),
            scene.transport_statistics()
        );
    }
}
