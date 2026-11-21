use super::{
    MusicResult,
    auth::{Credentials, Token},
    client::HttpClient,
};
use bytes::Bytes;
use reqwest::{Method, header::HeaderMap};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    mem,
    sync::{Arc, RwLock},
};
use strum::IntoEnumIterator as _;
use strum_macros::{EnumIter, IntoStaticStr};
use tokio::sync::Mutex;
use tracing::warn;

const SESSION_DATA_POISON_MSG: &str = "session data rwlock should not be poisoned";

struct SessionInternal {
    device_id: String,
    session_id: String,
    connection_id: RwLock<String>,
    credentials: RwLock<Option<Credentials>>,

    http_client: HttpClient,

    endpoints: Mutex<ServiceEndpoints>,
    access_token: Mutex<Option<Token>>,
    api_endpoint: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct Session(Arc<SessionInternal>);

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

impl Session {
    pub(super) fn access_token_cache(&self) -> &Mutex<Option<Token>> {
        &self.0.access_token
    }

    pub(super) fn api_endpoint(&self) -> &Mutex<Option<String>> {
        &self.0.api_endpoint
    }
}

impl Session {
    pub fn new() -> MusicResult<Self> {
        Ok(Self(Arc::new(SessionInternal {
            device_id: device_id()?,
            session_id: hex::encode(random_uuid_bytes()?),
            connection_id: RwLock::default(),
            credentials: RwLock::default(),
            http_client: HttpClient::new()?,
            endpoints: Mutex::default(),
            access_token: Mutex::default(),
            api_endpoint: Mutex::default(),
        })))
    }

    pub(super) fn http_client(&self) -> &HttpClient {
        &self.0.http_client
    }

    pub(super) fn device_id(&self) -> &str {
        &self.0.device_id
    }

    pub(super) fn session_id(&self) -> &str {
        &self.0.session_id
    }

    pub(super) fn connection_id(&self) -> String {
        self.0.connection_id.read().expect(SESSION_DATA_POISON_MSG).clone()
    }

    pub(super) fn set_connection_id(&self, id: &str) {
        id.clone_into(&mut self.0.connection_id.write().expect(SESSION_DATA_POISON_MSG));
    }

    pub(super) fn credentials(&self) -> Option<Credentials> {
        self.0.credentials.read().expect(SESSION_DATA_POISON_MSG).clone()
    }

    pub(super) fn set_credentials(&self, credentials: Credentials) {
        *self.0.credentials.write().expect(SESSION_DATA_POISON_MSG) = Some(credentials);
    }

    pub(super) fn username(&self) -> String {
        self.0
            .credentials
            .read()
            .expect(SESSION_DATA_POISON_MSG)
            .as_ref()
            .map(|credentials| credentials.username.clone())
            .unwrap_or_default()
    }
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
        let mut points = self.0.endpoints.lock().await;
        let queue = points.queue(endpoint);
        if queue.is_empty() {
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
        let body = self
            .http_client()
            .request(
                &Method::GET,
                "https://apresolve.spotify.com/?type=dealer&type=spclient",
                &HeaderMap::new(),
                Bytes::new(),
            )
            .await?;
        Ok(serde_json::from_slice(&body)?)
    }
}
