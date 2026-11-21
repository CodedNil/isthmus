/// Selects a metadata extension to fetch for an entity.
#[derive(prost::Message)]
pub struct ExtensionQuery {
    #[prost(enumeration = "ExtensionKind", tag = "1")]
    pub extension_kind: i32,
}

/// Metadata extensions requested for one Spotify URI.
#[derive(prost::Message)]
pub struct EntityRequest {
    #[prost(string, tag = "1")]
    pub entity_uri: String,
    #[prost(message, repeated, tag = "2")]
    pub query: Vec<ExtensionQuery>,
}

/// Request for metadata on multiple Spotify entities.
#[derive(prost::Message)]
pub struct BatchedEntityRequest {
    #[prost(message, repeated, tag = "2")]
    pub entity_request: Vec<EntityRequest>,
}

/// Results grouped by metadata extension kind.
#[derive(prost::Message)]
pub struct EntityExtensionDataArray {
    #[prost(enumeration = "ExtensionKind", tag = "2")]
    pub extension_kind: i32,
    #[prost(message, repeated, tag = "3")]
    pub extension_data: Vec<EntityExtensionData>,
}

/// Response containing the groups returned for an extended metadata request.
#[derive(prost::Message)]
pub struct BatchedExtensionResponse {
    #[prost(message, repeated, tag = "2")]
    pub extended_metadata: Vec<EntityExtensionDataArray>,
}

/// One entity’s status and protobuf `Any` payload for the requested extension.
#[derive(prost::Message)]
pub struct EntityExtensionData {
    #[prost(message, optional, tag = "1")]
    pub header: Option<EntityExtensionDataHeader>,
    #[prost(string, tag = "2")]
    pub entity_uri: String,
    #[prost(message, optional, tag = "3")]
    pub extension_data: Option<::prost_types::Any>,
}

/// Status reported for an individual metadata result.
#[derive(prost::Message)]
pub struct EntityExtensionDataHeader {
    #[prost(int32, tag = "1")]
    pub status_code: i32,
}

/// Metadata extension IDs recognized by Cantus.
#[derive(Debug, prost::Enumeration)]
pub enum ExtensionKind {
    /// Default when an extension is unspecified or unrecognized.
    Unknown = 0,
    /// Catalog track metadata encoded as a `Track` message.
    TrackV4 = 10,
}
