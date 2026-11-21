use super::{
    AudioFeatures, CachedPlaylist, CatalogTrack, Metadata, MusicResult, PlaylistId, REQUEST_TIMEOUT, SpotifyLyrics,
    TrackId,
    protocol::{
        collection::{CollectionItem, WriteRequest},
        connect::{Cluster, PutStateRequest},
        extended_metadata::{
            BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionKind, ExtensionQuery,
        },
        list_read::{
            AttributeKind, Attributes, AttributesRequestedFields, GetListRequest, GetListsRequest, GetListsResponse,
            ItemRequestedFields, ListEntity, ListEntityRequestedFields, ListIdentifier, ListResponseRequestedFields,
            MetaItemsRequestedFields, RequestOptions, RequestedFields,
        },
        playlist4_external::{ListChanges, PlaylistWriteResult},
    },
    session::{Endpoint, Session},
};
use bytes::Bytes;
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use protobuf::{EnumOrUnknown, Message, MessageField};
use reqwest::{
    Client, Error as HttpError, IntoUrl, Method, StatusCode,
    header::{
        ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue,
        RETRY_AFTER,
    },
    redirect::Policy,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    error::Error as StdError,
    fmt::{Display, Formatter, Result as FmtResult},
    io::{self, Read},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;
use tracing::warn;

const PROTOBUF: HeaderValue = HeaderValue::from_static("application/x-protobuf");
const CONNECTION_ID: HeaderName = HeaderName::from_static("x-spotify-connection-id");

impl Session {
    async fn base_url(&self) -> String {
        let mut endpoint = self.api_endpoint().lock().await;
        if let Some(url) = endpoint.as_ref() {
            return url.clone();
        }
        let (host, port) = self.resolve(Endpoint::Api).await;
        let url = format!("https://{host}:{port}");
        *endpoint = Some(url.clone());
        url
    }

    async fn request_with_protobuf(
        &self,
        method: &Method,
        endpoint: &str,
        mut headers: HeaderMap,
        message: &impl Message,
    ) -> MusicResult<Bytes> {
        headers.insert(CONTENT_TYPE, PROTOBUF);

        self.request(method, endpoint, headers, message.write_to_bytes()?.into()).await
    }

    async fn get_json<T: DeserializeOwned>(&self, endpoint: &str, mut headers: HeaderMap) -> MusicResult<T> {
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let body = self.request(&Method::GET, endpoint, headers, Bytes::new()).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    async fn request(
        &self,
        method: &Method,
        endpoint: &str,
        mut headers: HeaderMap,
        body: Bytes,
    ) -> MusicResult<Bytes> {
        let max_attempts = if matches!(*method, Method::GET | Method::PUT) { 3 } else { 1 };
        let mut remaining = max_attempts;
        loop {
            let (base, token) = tokio::join!(self.base_url(), self.auth_token());
            let token = token?;
            headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}"))?);
            let result = self.http_client().request(method, format!("{base}{endpoint}"), &headers, body.clone()).await;
            if !result.as_ref().is_err_and(|error| retryable(error.as_ref())) {
                return result;
            }

            {
                let mut endpoint = self.api_endpoint().lock().await;
                if endpoint.as_ref() == Some(&base) {
                    *endpoint = None;
                }
            }
            remaining -= 1;
            if remaining == 0 {
                return result;
            }
        }
    }

    pub(super) async fn put_connect_state(&self, state: &PutStateRequest) -> MusicResult<Cluster> {
        let endpoint = format!("/connect-state/v1/devices/{}", self.device_id());
        let bytes = self.request_with_protobuf(&Method::PUT, &endpoint, self.connected_headers()?, state).await?;
        Ok(Cluster::parse_from_bytes(&bytes)?)
    }

    pub(super) async fn change_playlist(
        &self,
        id: PlaylistId,
        changes: &ListChanges,
    ) -> MusicResult<Option<PlaylistWriteResult>> {
        let path = format!("/playlist/v2/playlist/{id}/changes");
        let bytes = self.request_with_protobuf(&Method::POST, &path, self.connected_headers()?, changes).await?;
        // A successful write with an unfamiliar response still needs a snapshot.
        Ok(PlaylistWriteResult::parse_from_bytes(&bytes).ok())
    }

    pub(super) async fn write_liked_song(&self, uri: String, should_like: bool) -> MusicResult<()> {
        const MEDIA_TYPE: HeaderValue = HeaderValue::from_static("application/vnd.collection-v2.spotify.proto");
        let item = CollectionItem {
            uri,
            added_at: if should_like {
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
            } else {
                0
            },
            is_removed: !should_like,
            ..Default::default()
        };
        let request = WriteRequest {
            username: self.username(),
            set: "collection".into(),
            items: vec![item],
            ..Default::default()
        };
        let mut headers = self.connected_headers()?;
        headers.insert(CONTENT_TYPE, MEDIA_TYPE);
        headers.insert(ACCEPT, MEDIA_TYPE);
        self.request(&Method::POST, "/collection/v2/write", headers, request.write_to_bytes()?.into()).await?;
        Ok(())
    }

    pub(super) async fn post_player_command(&self, target: &str, command: &Value) -> MusicResult<()> {
        let path = format!("/connect-state/v1/player/command/from/{}/to/{target}", self.device_id());
        let mut body = gzip_encoder();
        serde_json::to_writer(
            &mut body,
            &json!({
                "command": command,
                "connection_type": "wlan",
                "intent_id": format!("{:032x}", fastrand::u128(..)),
            }),
        )?;
        let mut headers = self.connected_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        self.request(&Method::POST, &path, headers, body.finish()?.into()).await?;
        Ok(())
    }

    fn connected_headers(&self) -> MusicResult<HeaderMap> {
        let connection_id = self.connection_id();
        if connection_id.is_empty() {
            return Err("Spotify Dealer is not connected".into());
        }
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION_ID, connection_id.parse()?);
        Ok(headers)
    }

    pub(super) async fn track_metadata(&self, uris: &[String]) -> MusicResult<Metadata> {
        let kind = EnumOrUnknown::new(ExtensionKind::TRACK_V4);
        let request = BatchedEntityRequest {
            entity_request: uris
                .iter()
                .map(|uri| EntityRequest {
                    entity_uri: uri.clone(),
                    query: vec![ExtensionQuery { extension_kind: kind, ..Default::default() }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let bytes = self
            .request_with_protobuf(&Method::POST, "/extended-metadata/v0/extended-metadata", HeaderMap::new(), &request)
            .await?;
        Ok(BatchedExtensionResponse::parse_from_bytes(&bytes)?
            .extended_metadata
            .into_iter()
            .filter(|array| array.extension_kind == kind)
            .flat_map(|array| array.extension_data)
            .filter_map(|data| {
                if data.header.get_or_default().status_code == i32::from(StatusCode::NOT_FOUND.as_u16()) {
                    return Some((data.entity_uri, None));
                }
                let payload = data.extension_data.into_option()?;
                let track = CatalogTrack::parse_from_bytes(&payload.value)
                    .inspect_err(|error| warn!(%error, uri = %data.entity_uri, "Invalid Spotify track metadata"))
                    .ok()?;
                Some((data.entity_uri, Some(track)))
            })
            .collect())
    }

    pub(super) async fn audio_features(&self, ids: &[TrackId]) -> MusicResult<HashMap<TrackId, AudioFeatures>> {
        #[derive(Deserialize)]
        struct FeatureEntry {
            id: TrackId,
            #[serde(flatten)]
            features: AudioFeatures,
        }

        #[derive(Deserialize)]
        struct Response {
            audio_features: Vec<Option<FeatureEntry>>,
        }

        let ids = ids.iter().map(TrackId::as_str).collect::<Vec<_>>().join(",");
        let path = format!("/audio-attributes/v1/audio-features?format=json&ids={ids}");
        let batch: Response = self.get_json(&path, HeaderMap::new()).await?;
        Ok(batch.audio_features.into_iter().flatten().map(|entry| (entry.id, entry.features)).collect())
    }

    pub(super) async fn lyrics(&self, id: TrackId) -> MusicResult<Option<SpotifyLyrics>> {
        let path = format!("/color-lyrics/v2/track/{id}?format=json&vocalRemoval=false&market=from_token");
        let mut headers = HeaderMap::new();
        headers.insert("app-platform", HeaderValue::from_static("WebPlayer"));
        match self.get_json(&path, headers).await {
            Err(error) if error.downcast_ref::<HttpStatus>().is_some_and(HttpStatus::missing) => Ok(None),
            result => result,
        }
    }

    pub(super) async fn playlists(
        &self,
        ids: &[PlaylistId],
        include_artwork: bool,
    ) -> MusicResult<Vec<(CachedPlaylist, Option<Attributes>)>> {
        let uris = ids.iter().map(|id| format!("spotify:playlist:{id}")).collect::<Vec<_>>();
        Ok(self
            .read_lists(&uris, playlist_fields(false, include_artwork))
            .await?
            .into_iter()
            .map(|(list, revision)| {
                let items =
                    list.items.into_iter().map(|item| item.attributes.into_option().unwrap_or_default().uri).collect();
                (CachedPlaylist { revision, items }, list.attributes.into_option())
            })
            .collect())
    }

    pub(super) async fn root_playlist(&self) -> MusicResult<(ListEntity, Vec<u8>)> {
        let uri = format!("spotify:user:{}:rootlist", self.username());
        self.read_lists(&[uri], playlist_fields(true, false))
            .await?
            .pop()
            .ok_or_else(|| "Missing Spotify root list".into())
    }

    async fn read_lists(
        &self,
        uris: &[String],
        requested_fields: RequestedFields,
    ) -> MusicResult<Vec<(ListEntity, Vec<u8>)>> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }
        let request = GetListsRequest {
            requests: uris
                .iter()
                .map(|uri| GetListRequest {
                    identifier: MessageField::some(ListIdentifier { uri: uri.clone(), ..Default::default() }),
                    options: MessageField::some(RequestOptions {
                        requested_fields: MessageField::some(requested_fields.clone()),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let bytes = self
            .request_with_protobuf(
                &Method::POST,
                "/playlist/spotify.list.v1.ListReadService/GetLists",
                HeaderMap::new(),
                &request,
            )
            .await?;
        let response = GetListsResponse::parse_from_bytes(&bytes)?;
        if response.responses.len() != uris.len() {
            return Err("Spotify returned the wrong number of playlist results".into());
        }
        uris.iter()
            .zip(response.responses)
            .map(|(uri, list)| {
                let info = list.info.into_option().unwrap_or_default();
                if let Some(error) = info.request_error.as_ref() {
                    return Err(format!("Spotify could not read {uri}: {}", error.message).into());
                }
                let entity = list.entity.into_option().ok_or_else(|| io::Error::other("Missing Spotify playlist"))?;
                Ok((entity, info.revision))
            })
            .collect()
    }
}

fn playlist_fields(root: bool, include_artwork: bool) -> RequestedFields {
    const ARTWORK: [AttributeKind; 3] =
        [AttributeKind::KIND_NAME, AttributeKind::KIND_PICTURE_FILE_ID, AttributeKind::KIND_PICTURE_SIZES];
    let attributes = |kinds: &[AttributeKind]| {
        MessageField::some(AttributesRequestedFields {
            requested_field: kinds.iter().copied().map(EnumOrUnknown::new).collect(),
            ..Default::default()
        })
    };
    let mut entity = ListEntityRequestedFields {
        items: MessageField::some(ItemRequestedFields {
            attributes: attributes(&[AttributeKind::KIND_URI]),
            ..Default::default()
        }),
        ..Default::default()
    };
    if root {
        entity.meta_items =
            MessageField::some(MetaItemsRequestedFields { attributes: attributes(&ARTWORK), ..Default::default() });
    }
    if include_artwork {
        entity.attributes = attributes(&ARTWORK);
    }
    RequestedFields {
        list_response: MessageField::some(ListResponseRequestedFields {
            entity: MessageField::some(entity),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(super) fn decode_gzip(bytes: Bytes, compressed: bool) -> io::Result<Bytes> {
    if !compressed || bytes.is_empty() {
        return Ok(bytes);
    }
    let mut decoded = Vec::new();
    MultiGzDecoder::new(bytes.as_ref()).read_to_end(&mut decoded)?;
    Ok(decoded.into())
}

pub(super) fn gzip_encoder() -> GzEncoder<Vec<u8>> {
    GzEncoder::new(Vec::new(), Compression::fast())
}

const USER_AGENT: &str = concat!("Spotify/124200290 Linux/0 (Cantus/", env!("CARGO_PKG_VERSION"), ")");

pub struct HttpClient(Client);

impl HttpClient {
    pub fn new() -> MusicResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        Ok(Self(
            Client::builder()
                .user_agent(USER_AGENT)
                .default_headers(headers)
                .redirect(Policy::none())
                .timeout(REQUEST_TIMEOUT)
                .no_proxy()
                .build()?,
        ))
    }

    pub(super) async fn request(
        &self,
        method: &Method,
        url: impl IntoUrl + Clone,
        headers: &HeaderMap,
        body: Bytes,
    ) -> MusicResult<Bytes> {
        let mut retried = false;
        loop {
            let response = self
                .0
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await
                .map_err(HttpError::without_url)?;
            let status = response.status();
            if status.is_success() {
                let gzip = response
                    .headers()
                    .get(CONTENT_ENCODING)
                    .is_some_and(|encoding| encoding.as_bytes().eq_ignore_ascii_case(b"gzip"));
                return Ok(decode_gzip(response.bytes().await.map_err(HttpError::without_url)?, gzip)?);
            }
            if status == StatusCode::TOO_MANY_REQUESTS && !retried {
                let retry = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|seconds| *seconds <= 10);
                if let Some(seconds) = retry {
                    retried = true;
                    sleep(Duration::from_secs(seconds)).await;
                    continue;
                }
            }
            return Err(HttpStatus(status).into());
        }
    }
}

#[derive(Debug)]
struct HttpStatus(StatusCode);

impl HttpStatus {
    fn missing(&self) -> bool {
        matches!(
            self.0,
            StatusCode::NOT_FOUND
                | StatusCode::GONE
                | StatusCode::MOVED_PERMANENTLY
                | StatusCode::PERMANENT_REDIRECT
                | StatusCode::TEMPORARY_REDIRECT
        )
    }
}

impl Display for HttpStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "Spotify HTTP request returned {}", self.0)
    }
}

impl StdError for HttpStatus {}

fn retryable(error: &(dyn StdError + 'static)) -> bool {
    error.is::<io::Error>()
        || error.downcast_ref::<HttpError>().is_some_and(|error| error.is_timeout() || error.is_connect())
        || error.downcast_ref::<HttpStatus>().is_some_and(|status| {
            matches!(
                status.0,
                StatusCode::GATEWAY_TIMEOUT
                    | StatusCode::REQUEST_TIMEOUT
                    | StatusCode::INTERNAL_SERVER_ERROR
                    | StatusCode::MISDIRECTED_REQUEST
                    | StatusCode::SERVICE_UNAVAILABLE
                    | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS
            )
        })
}
