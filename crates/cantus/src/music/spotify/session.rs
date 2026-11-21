use super::{MusicResult, auth::Auth};
use reqwest::Client;
use serde::Deserialize;
use std::{collections::VecDeque, mem};
use strum::IntoEnumIterator as _;
use strum_macros::{EnumIter, IntoStaticStr};
use tokio::sync::Mutex;
use tracing::warn;

pub(super) struct Session {
    pub(super) device_id: String,
    pub(super) id: String,
    pub(super) username: String,
    pub(super) http: Client,
    pub(super) auth: Mutex<Auth>,
    endpoints: Mutex<EndpointState>,
}

impl Session {
    pub(super) fn new(http: Client) -> MusicResult<Self> {
        Ok(Self {
            device_id: device_id()?,
            id: hex::encode(random_uuid_bytes()?),
            username: String::new(),
            http,
            auth: Mutex::default(),
            endpoints: Mutex::default(),
        })
    }
}

fn random_uuid_bytes() -> Result<[u8; 16], getrandom::Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(bytes)
}

fn device_id() -> Result<String, getrandom::Error> {
    let mut id = hex::encode(random_uuid_bytes()?);
    for index in [20, 16, 12, 8] {
        id.insert(index, '-');
    }
    Ok(id)
}

pub(super) type SocketAddress = (String, u16);

#[derive(Clone, Copy, EnumIter, IntoStaticStr)]
pub(super) enum Endpoint {
    #[strum(serialize = "dealer.spotify.com")]
    Dealer,
    #[strum(serialize = "spclient.wg.spotify.com")]
    Api,
}

impl Endpoint {
    fn fallback(self) -> SocketAddress {
        let host: &str = self.into();
        (host.to_owned(), 443)
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ServiceEndpoints {
    dealer: VecDeque<String>,
    spclient: VecDeque<String>,
}

#[derive(Default)]
struct EndpointState {
    services: ServiceEndpoints,
    api_url: Option<String>,
}

impl ServiceEndpoints {
    const fn queue(&mut self, endpoint: Endpoint) -> &mut VecDeque<String> {
        match endpoint {
            Endpoint::Dealer => &mut self.dealer,
            Endpoint::Api => &mut self.spclient,
        }
    }
}

impl Session {
    pub(super) async fn resolve(&self, endpoint: Endpoint) -> SocketAddress {
        let mut endpoints = self.endpoints.lock().await;
        self.resolve_endpoint(&mut endpoints.services, endpoint).await
    }

    pub(super) async fn base_url(&self) -> String {
        let mut endpoints = self.endpoints.lock().await;
        if let Some(url) = &endpoints.api_url {
            return url.clone();
        }

        let (host, port) = self.resolve_endpoint(&mut endpoints.services, Endpoint::Api).await;
        let url = format!("https://{host}:{port}");
        endpoints.api_url = Some(url.clone());
        url
    }

    pub(super) async fn invalidate_base_url(&self, url: &str) {
        let mut endpoints = self.endpoints.lock().await;
        if endpoints.api_url.as_deref() == Some(url) {
            endpoints.api_url = None;
        }
    }

    async fn resolve_endpoint(&self, points: &mut ServiceEndpoints, endpoint: Endpoint) -> SocketAddress {
        if points.queue(endpoint).is_empty() {
            match self.fetch_endpoints().await {
                Ok(mut resolved) => {
                    // Preserve other queues so resolving the dealer cannot reset an API failover.
                    for kind in Endpoint::iter() {
                        if points.queue(kind).is_empty() {
                            *points.queue(kind) = mem::take(resolved.queue(kind));
                        }
                    }
                }
                Err(error) => warn!(%error, "Spotify service lookup failed"),
            }
        }
        while let Some(address) = points.queue(endpoint).pop_front() {
            if let Some((host, port)) = address.rsplit_once(':')
                && let Ok(port) = port.parse()
            {
                return (host.to_owned(), port);
            }
        }
        endpoint.fallback()
    }

    async fn fetch_endpoints(&self) -> MusicResult<ServiceEndpoints> {
        Ok(self
            .http
            .get("https://apresolve.spotify.com/?type=dealer&type=spclient")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}
