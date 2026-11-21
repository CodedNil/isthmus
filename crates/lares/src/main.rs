use lares::bake::ensure_assets;
use std::{env, process::ExitCode};
use tracing::error;

fn main() -> ExitCode {
    lares::init_logging();
    // The bake command can run without a Wayland display.
    if env::args().any(|argument| argument == "--bake") {
        return match ensure_assets() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error!(%error, "could not bake lighting assets");
                ExitCode::FAILURE
            }
        };
    }
    match lares::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(%error, "Lares stopped");
            ExitCode::FAILURE
        }
    }
}
