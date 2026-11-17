use lares::{
    bake::ensure_assets,
    server::{self, DEFAULT_PORT},
};
use std::{env, process::ExitCode};
use tokio::runtime::Runtime;
use tracing::error;

fn main() -> ExitCode {
    lares::init_logging();
    // `lares --bake` prepares the lighting assets and exits, for builds that bake ahead of serving.
    if env::args().any(|argument| argument == "--bake") {
        return match ensure_assets() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error!(%error, "could not bake lighting assets");
                ExitCode::FAILURE
            }
        };
    }
    let runtime = Runtime::new().expect("failed to start the Lares runtime");
    let port = env::var("LARES_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_PORT);
    runtime.block_on(server::serve(port));
    ExitCode::SUCCESS
}
