use crate::bake::ensure_assets;
use axum::Router;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::{net::TcpListener, task::spawn_blocking};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

const FRONTEND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
pub const DEFAULT_PORT: u16 = 8128;

pub async fn serve(port: u16) {
    match spawn_blocking(ensure_assets).await {
        Ok(Ok(())) => {}
        result => {
            warn!(?result, "could not prepare lighting assets");
            return;
        }
    }
    let app = Router::new().fallback_service(
        ServeDir::new(FRONTEND)
            .precompressed_gzip()
            .not_found_service(ServeFile::new(format!("{FRONTEND}/index.html"))),
    );
    let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    match TcpListener::bind(address).await {
        Ok(listener) => {
            info!("Lares is serving http://localhost:{port}");
            if let Err(error) = axum::serve(listener, app).await {
                warn!(%error, "Lares server stopped");
            }
        }
        Err(error) => warn!(%error, "could not bind {address}"),
    }
}
