use super::{
    AudioFeatures, CachedPlaylist, CatalogTrack, Metadata, MusicResult, PlaylistId, SpotifyLyrics, TrackId,
    proto::{
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
        playlist_edit::ListChanges,
    },
    session::Session,
    track_id_from_uri,
};
use bytes::Bytes;
use flate2::{Compression, write::GzEncoder};
use prost::Message;
use reqwest::{
    Error as HttpError, Method, StatusCode,
    header::{ACCEPT, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::warn;

const PROTOBUF: HeaderValue = HeaderValue::from_static("application/x-protobuf");
const CONNECTION_ID: HeaderName = HeaderName::from_static("x-spotify-connection-id");
const GET_LISTS: &str = "/playlist/spotify.list.v1.ListReadService/GetLists";
const TRACK_METADATA: &str = "/extended-metadata/v0/extended-metadata";
const SPOTIFY_USER_AGENT: HeaderValue =
    HeaderValue::from_static(concat!("Spotify/124200290 Linux/0 (Cantus/", env!("CARGO_PKG_VERSION"), ")"));

type ListResult = MusicResult<(ListEntity, Vec<u8>)>;
pub(super) type PlaylistResult = MusicResult<(CachedPlaylist, Option<Attributes>)>;

impl Session {
    async fn request_with_protobuf(
        &self,
        method: &Method,
        endpoint: &str,
        mut headers: HeaderMap,
        message: &impl Message,
    ) -> MusicResult<Bytes> {
        headers.insert(CONTENT_TYPE, PROTOBUF);

        self.request(method, endpoint, headers, message.encode_to_vec().into()).await
    }

    async fn get_json<T: DeserializeOwned>(&self, endpoint: &str, headers: HeaderMap) -> MusicResult<T> {
        let body = self.request(&Method::GET, endpoint, headers, Bytes::new()).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    async fn request(&self, method: &Method, endpoint: &str, headers: HeaderMap, body: Bytes) -> MusicResult<Bytes> {
        let (base, token) = tokio::join!(self.base_url(), self.auth_token());
        let response = self
            .http
            .request(method.clone(), format!("{}{endpoint}", base?))
            .headers(headers)
            .bearer_auth(token?)
            .body(body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(HttpError::without_url)?;
        Ok(response.bytes().await.map_err(HttpError::without_url)?)
    }

    pub(super) async fn put_connect_state(&self, connection_id: &str, state: &PutStateRequest) -> MusicResult<Cluster> {
        let endpoint = format!("/connect-state/v1/devices/{}", self.device_id);
        let bytes =
            self.request_with_protobuf(&Method::PUT, &endpoint, connected_headers(connection_id)?, state).await?;
        Ok(Cluster::decode(bytes)?)
    }

    pub(super) async fn change_playlist(
        &self,
        connection_id: &str,
        id: PlaylistId,
        changes: &ListChanges,
    ) -> MusicResult<()> {
        let path = format!("/playlist/v2/playlist/{id}/changes");
        self.request_with_protobuf(&Method::POST, &path, connected_headers(connection_id)?, changes).await.map(drop)
    }

    pub(super) async fn write_liked_song(
        &self,
        connection_id: &str,
        uri: String,
        should_like: bool,
    ) -> MusicResult<()> {
        const MEDIA_TYPE: HeaderValue = HeaderValue::from_static("application/vnd.collection-v2.spotify.proto");
        let item = CollectionItem {
            uri,
            added_at: if should_like {
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
            } else {
                0
            },
            is_removed: !should_like,
        };
        let request = WriteRequest { username: self.username.clone(), set: "collection".into(), items: vec![item] };
        let mut headers = connected_headers(connection_id)?;
        headers.insert(CONTENT_TYPE, MEDIA_TYPE);
        headers.insert(ACCEPT, MEDIA_TYPE);
        self.request(&Method::POST, "/collection/v2/write", headers, request.encode_to_vec().into()).await.map(drop)
    }

    pub(super) async fn post_player_command(
        &self,
        connection_id: &str,
        target: &str,
        command: &Value,
    ) -> MusicResult<()> {
        let path = format!("/connect-state/v1/player/command/from/{}/to/{target}", self.device_id);
        let mut body = gzip_encoder();
        serde_json::to_writer(
            &mut body,
            &json!({
                "command": command,
                "connection_type": "wlan",
                "intent_id": format!("{:032x}", fastrand::u128(..)),
            }),
        )?;
        let mut headers = connected_headers(connection_id)?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        self.request(&Method::POST, &path, headers, body.finish()?.into()).await.map(drop)
    }

    pub(super) async fn track_metadata(&self, uris: &[String]) -> MusicResult<Metadata> {
        let kind = ExtensionKind::TrackV4 as i32;
        let request = BatchedEntityRequest {
            entity_request: uris
                .iter()
                .map(|uri| EntityRequest {
                    entity_uri: uri.clone(),
                    query: vec![ExtensionQuery { extension_kind: kind }],
                })
                .collect(),
        };
        let bytes = self.request_with_protobuf(&Method::POST, TRACK_METADATA, HeaderMap::new(), &request).await?;
        Ok(BatchedExtensionResponse::decode(bytes)?
            .extended_metadata
            .into_iter()
            .filter(|array| array.extension_kind == kind)
            .flat_map(|array| array.extension_data)
            .filter_map(|data| {
                if data.header.is_some_and(|header| header.status_code == i32::from(StatusCode::NOT_FOUND.as_u16())) {
                    return Some((data.entity_uri, None));
                }
                let payload = data.extension_data?;
                let track = CatalogTrack::decode(payload.value.as_slice())
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
        let path = format!("/color-lyrics/v3/track/{id}?prototype=true");
        let mut headers = spotify_client_headers();
        headers.insert("app-platform", HeaderValue::from_static("Android"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.google.protobuf"));
        match self.request(&Method::GET, &path, headers, Bytes::new()).await {
            Ok(bytes) => Ok(Some(SpotifyLyrics::decode(bytes)?)),
            Err(error)
                if matches!(
                    error.downcast_ref::<HttpError>().and_then(HttpError::status),
                    Some(StatusCode::NOT_FOUND | StatusCode::GONE)
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn playlists(&self, ids: &[PlaylistId]) -> MusicResult<Vec<PlaylistResult>> {
        let uris = ids.iter().map(|id| format!("spotify:playlist:{id}")).collect::<Vec<_>>();
        Ok(self.read_lists(&uris, false).await?.into_iter().map(playlist_result).collect())
    }

    pub(super) async fn root_playlist(&self) -> ListResult {
        let root = format!("spotify:user:{}:rootlist", self.username);
        self.read_lists(&[root], true).await?.pop().ok_or("Missing Spotify root list")?
    }

    async fn read_lists(&self, uris: &[String], root: bool) -> MusicResult<Vec<ListResult>> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }
        let request = GetListsRequest {
            requests: uris
                .iter()
                .map(|uri| GetListRequest {
                    identifier: Some(ListIdentifier { uri: uri.clone() }),
                    options: Some(RequestOptions { requested_fields: Some(playlist_fields(root)) }),
                })
                .collect(),
        };
        let bytes = self.request_with_protobuf(&Method::POST, GET_LISTS, HeaderMap::new(), &request).await?;
        decode_list_results(&bytes, uris)
    }
}

fn playlist_result(result: ListResult) -> PlaylistResult {
    result.map(|(list, revision)| {
        let tracks = list.items.iter().filter_map(|item| track_id_from_uri(&item.attributes.as_ref()?.uri)).collect();
        let name = list.attributes.as_ref().map_or_else(String::new, |attributes| attributes.name.clone());
        (CachedPlaylist { revision, tracks: Arc::new(tracks), name }, list.attributes)
    })
}

fn connected_headers(connection_id: &str) -> MusicResult<HeaderMap> {
    if connection_id.is_empty() {
        return Err("Spotify Dealer is not connected".into());
    }
    let mut headers = spotify_client_headers();
    headers.insert(CONNECTION_ID, connection_id.parse()?);
    Ok(headers)
}

fn spotify_client_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, SPOTIFY_USER_AGENT);
    headers
}

fn decode_list_results(bytes: &[u8], uris: &[String]) -> MusicResult<Vec<ListResult>> {
    let response = GetListsResponse::decode(bytes)?;
    if response.responses.len() != uris.len() {
        return Err("Spotify returned the wrong number of playlist results".into());
    }
    Ok(uris
        .iter()
        .zip(response.responses)
        .map(|(uri, list)| {
            let info = list.info.unwrap_or_default();
            if let Some(error) = info.request_error {
                return Err(format!("Spotify could not read {uri}: {}", error.message).into());
            }
            let entity = list.entity.ok_or_else(|| format!("Spotify omitted the list entity for {uri}"))?;
            Ok((entity, info.revision))
        })
        .collect())
}

fn playlist_fields(root: bool) -> RequestedFields {
    let artwork = Some(AttributesRequestedFields {
        requested_field: [AttributeKind::Name, AttributeKind::PictureFileId, AttributeKind::PictureSizes]
            .into_iter()
            .map(|kind| kind as i32)
            .collect(),
    });
    let mut entity = ListEntityRequestedFields {
        items: Some(ItemRequestedFields {
            attributes: Some(AttributesRequestedFields { requested_field: vec![AttributeKind::Uri as i32] }),
        }),
        ..Default::default()
    };
    if root {
        entity.meta_items = Some(MetaItemsRequestedFields { attributes: artwork });
    } else {
        entity.attributes = artwork;
    }
    RequestedFields { list_response: Some(ListResponseRequestedFields { entity: Some(entity) }) }
}

pub(super) fn gzip_encoder() -> GzEncoder<Vec<u8>> {
    GzEncoder::new(Vec::new(), Compression::fast())
}

#[cfg(test)]
mod tests {
    use super::{
        super::proto::list_read::{GetListResponse, RequestError, ResponseInfo},
        *,
    };

    #[test]
    fn list_batch_preserves_results_around_failed_lists() {
        let success = |revision| GetListResponse {
            entity: Some(ListEntity::default()),
            info: Some(ResponseInfo { revision: vec![revision], ..Default::default() }),
        };
        let missing = GetListResponse {
            info: Some(ResponseInfo {
                request_error: Some(RequestError { message: "not found".into() }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let bytes = GetListsResponse { responses: vec![success(1), missing, GetListResponse::default(), success(4)] }
            .encode_to_vec();
        let uris = ["a".into(), "b".into(), "c".into(), "d".into()];
        let results = decode_list_results(&bytes, &uris).unwrap();
        assert_eq!(results[0].as_ref().unwrap().1, [1]);
        assert_eq!(results[1].as_ref().unwrap_err().to_string(), "Spotify could not read b: not found");
        assert_eq!(results[2].as_ref().unwrap_err().to_string(), "Spotify omitted the list entity for c");
        assert_eq!(results[3].as_ref().unwrap().1, [4]);
        assert_eq!(
            decode_list_results(&bytes, &uris[..3]).unwrap_err().to_string(),
            "Spotify returned the wrong number of playlist results"
        );
    }
}
