use super::{MusicResult, auth::Auth};
use reqwest::Client;
use serde::Deserialize;
use std::{collections::VecDeque, io};
use tokio::sync::Mutex;

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

#[derive(Deserialize)]
struct ServiceEndpoints {
    dealer: Vec<String>,
    spclient: Vec<String>,
}

#[derive(Default)]
struct EndpointState {
    dealer: VecDeque<String>,
    api_url: Option<String>,
}

impl Session {
    pub(super) async fn dealer_url(&self) -> MusicResult<String> {
        let mut endpoints = self.endpoints.lock().await;
        if endpoints.dealer.is_empty() {
            *endpoints = self.discover().await?;
        }
        endpoints
            .dealer
            .pop_front()
            .ok_or_else(|| io::Error::other("Spotify AP Resolve returned no Dealer address").into())
    }

    pub(super) async fn base_url(&self) -> MusicResult<String> {
        let mut endpoints = self.endpoints.lock().await;
        if endpoints.api_url.is_none() {
            *endpoints = self.discover().await?;
        }
        endpoints
            .api_url
            .clone()
            .ok_or_else(|| io::Error::other("Spotify AP Resolve returned no spclient address").into())
    }

    async fn discover(&self) -> MusicResult<EndpointState> {
        let hosts: ServiceEndpoints = self
            .http
            .get("https://apresolve.spotify.com/?type=dealer&type=spclient")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let url = |scheme, address: &String| {
            let (host, port) = address.rsplit_once(':')?;
            let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
            (!host.is_empty() && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')))
                .then(|| format!("{scheme}://{host}:{port}"))
        };
        Ok(EndpointState {
            dealer: hosts.dealer.iter().filter_map(|host| url("wss", host)).collect(),
            api_url: hosts.spclient.iter().find_map(|host| url("https", host)),
        })
    }
}
