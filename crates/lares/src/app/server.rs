//! Serves the browser app and the shared presence relay.
use crate::{
    irradiance::bake::{baked_dir, ensure_assets},
    presence,
};
use std::{
    env,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use tokio::{net::TcpListener, task::spawn_blocking};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

pub const DEFAULT_PORT: u16 = 8128;

/// The web app: `LARES_ASSETS` if set, such as an installed package's share directory, else the source tree.
fn frontend() -> PathBuf {
    env::var_os("LARES_ASSETS")
        .map_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")), PathBuf::from)
}

pub async fn serve(port: u16) {
    match spawn_blocking(ensure_assets).await {
        Ok(Ok(())) => {}
        result => {
            warn!(?result, "could not prepare lighting assets");
            return;
        }
    }
    let frontend = frontend();
    // Baked lighting is served from wherever it was baked, which need not be inside the web app's directory.
    let app = presence::router().nest_service("/baked", ServeDir::new(baked_dir())).fallback_service(
        ServeDir::new(&frontend).precompressed_gzip().not_found_service(ServeFile::new(frontend.join("index.html"))),
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
