use lares::server::{self, DEFAULT_PORT};
use std::env;
use tokio::runtime::Runtime;
use tracing::Level;

fn main() {
    tracing_subscriber::fmt().with_target(false).with_max_level(Level::INFO).init();
    let runtime = Runtime::new().expect("failed to start the Lares runtime");
    let port = env::var("LARES_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_PORT);
    runtime.block_on(server::serve(port));
}
