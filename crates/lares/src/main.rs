use lares::app::server::{self, DEFAULT_PORT};
use std::env;
use tokio::runtime::Runtime;

fn main() {
    lares::init_logging();
    let runtime = Runtime::new().expect("failed to start the Lares runtime");
    let port = env::var("LARES_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_PORT);
    runtime.block_on(server::serve(port));
}
