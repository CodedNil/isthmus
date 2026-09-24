use lares::server::{self, DEFAULT_PORT};
use tokio::runtime::Runtime;
use tracing::Level;

fn main() {
    tracing_subscriber::fmt().with_target(false).with_max_level(Level::INFO).init();
    let runtime = Runtime::new().expect("failed to start the Lares runtime");
    runtime.block_on(server::serve(DEFAULT_PORT));
}
