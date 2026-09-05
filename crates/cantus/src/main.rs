use cantus::{Platform, run};
use std::{env, io};

fn main() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if env::args().any(|arg| arg == "--generate-nix-options") {
        return cantus::generate_nix_options();
    }
    if env::args().any(|arg| arg == "--launcher") {
        Platform::trigger_launcher();
    }
    run();
    Ok(())
}
