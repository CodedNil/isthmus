use std::collections::HashMap;

/// Batch of playlist and root-list reads sent to `GetLists`.
#[derive(prost::Message)]
pub struct GetListsRequest {
    #[prost(message, repeated, tag = "1")]
    pub requests: Vec<GetListRequest>,
}

/// Identifies one list and selects the fields to return.
#[derive(prost::Message)]
pub struct GetListRequest {
    #[prost(message, optional, tag = "1")]
    pub identifier: Option<ListIdentifier>,
    #[prost(message, optional, tag = "2")]
    pub options: Option<RequestOptions>,
}

/// Spotify URI of a playlist or a user’s root list.
#[derive(prost::Message)]
pub struct ListIdentifier {
    #[prost(string, tag = "1")]
    pub uri: String,
}

/// Options for reading one list, including the response field selection.
#[derive(prost::Message)]
pub struct RequestOptions {
    #[prost(message, optional, tag = "4")]
    pub requested_fields: Option<RequestedFields>,
}

/// Top-level selection of fields in the list response.
#[derive(Clone, prost::Message)]
pub struct RequestedFields {
    #[prost(message, optional, tag = "2")]
    pub list_response: Option<ListResponseRequestedFields>,
}

/// Selects the fields to return from the response’s list entity.
#[derive(Clone, prost::Message)]
pub struct ListResponseRequestedFields {
    #[prost(message, optional, tag = "1")]
    pub entity: Option<ListEntityRequestedFields>,
}

/// Selects list-level, entry, and referenced-list attributes.
#[derive(Clone, prost::Message)]
pub struct ListEntityRequestedFields {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<AttributesRequestedFields>,
    #[prost(message, optional, tag = "2")]
    pub items: Option<ItemRequestedFields>,
    #[prost(message, optional, tag = "3")]
    pub meta_items: Option<MetaItemsRequestedFields>,
}

/// Selects the attributes returned for each list entry.
#[derive(Clone, prost::Message)]
pub struct ItemRequestedFields {
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<AttributesRequestedFields>,
}

/// Selects the attributes returned for lists referenced by a root list.
#[derive(Clone, prost::Message)]
pub struct MetaItemsRequestedFields {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<AttributesRequestedFields>,
}

/// Set of attribute IDs to include in a response.
#[derive(Clone, prost::Message)]
pub struct AttributesRequestedFields {
    #[prost(enumeration = "AttributeKind", repeated, tag = "1")]
    pub requested_field: Vec<i32>,
}

/// Per-list results in the order they were requested.
#[derive(prost::Message)]
pub struct GetListsResponse {
    #[prost(message, repeated, tag = "1")]
    pub responses: Vec<GetListResponse>,
}

/// One requested list and its revision or error information.
#[derive(prost::Message)]
pub struct GetListResponse {
    #[prost(message, optional, tag = "1")]
    pub entity: Option<ListEntity>,
    #[prost(message, optional, tag = "2")]
    pub info: Option<ResponseInfo>,
}

/// Playlist or root list with entries and metadata for referenced lists.
#[derive(prost::Message)]
pub struct ListEntity {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<Attributes>,
    #[prost(message, repeated, tag = "2")]
    pub items: Vec<ListItem>,
    #[prost(map = "string, message", tag = "3")]
    pub meta_items: HashMap<String, MetaItem>,
}

/// One entry in a playlist or root list.
#[derive(prost::Message)]
pub struct ListItem {
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<Attributes>,
}

/// Attributes and revision information for a list referenced by a root list.
#[derive(PartialEq, Eq, prost::Message)]
pub struct MetaItem {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<Attributes>,
    #[prost(message, optional, tag = "2")]
    pub info: Option<ResponseInfo>,
}

/// Requested Spotify URI, name, and artwork fields for a list or entry.
#[derive(PartialEq, Eq, prost::Message)]
pub struct Attributes {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(bytes, tag = "20")]
    pub picture_file_id: Vec<u8>,
    #[prost(message, optional, tag = "21")]
    pub picture_sizes: Option<PictureSizes>,
}

/// Available artwork URLs for a list, in Spotify’s response order.
#[derive(PartialEq, Eq, prost::Message)]
pub struct PictureSizes {
    #[prost(message, repeated, tag = "1")]
    pub picture_size: Vec<PictureSize>,
}

/// Artwork URL for one size variant.
#[derive(PartialEq, Eq, prost::Message)]
pub struct PictureSize {
    #[prost(string, tag = "2")]
    pub url: String,
}

/// Revision and possible error for an individual list read.
#[derive(PartialEq, Eq, prost::Message)]
pub struct ResponseInfo {
    #[prost(message, optional, tag = "1")]
    pub request_error: Option<RequestError>,
    #[prost(bytes, tag = "7")]
    pub revision: Vec<u8>,
}

/// Spotify’s error message for a failed list read.
#[derive(PartialEq, Eq, prost::Message)]
pub struct RequestError {
    #[prost(string, tag = "6")]
    pub message: String,
}

/// Attribute IDs that can be requested from `GetLists`.
#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum AttributeKind {
    /// Default when no attribute has been selected.
    Unspecified = 0,
    /// Spotify URI.
    Uri = 1,
    /// Display name.
    Name = 3,
    /// Artwork file ID.
    PictureFileId = 20,
    /// Artwork URLs by size.
    PictureSizes = 21,
}
