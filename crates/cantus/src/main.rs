use std::env;

use cantus::{Platform, run};

fn main() {
    if env::args().any(|arg| arg == "--launcher") {
        Platform::trigger_launcher();
    }
    run();
}
