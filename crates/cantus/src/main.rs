use cantus::{platform, run};
use std::{env, io};

fn main() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if env::args().any(|arg| arg == "--generate-nix-options") {
        return cantus::generate_nix_options();
    }
    if env::args().any(|arg| arg == "--launcher") {
        return platform::trigger_launcher();
    }
    run();
    Ok(())
}
