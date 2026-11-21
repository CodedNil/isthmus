/// Save or remove a track in a user’s Spotify collection.
///
/// Wire fields verified against go-librespot’s collection/v2 implementation.
#[derive(prost::Message)]
pub struct CollectionItem {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(int64, tag = "2")]
    pub added_at: i64,
    #[prost(bool, tag = "3")]
    pub is_removed: bool,
}

/// Batch of saved-track changes for a user and collection set.
#[derive(prost::Message)]
pub struct WriteRequest {
    #[prost(string, tag = "1")]
    pub username: String,
    #[prost(string, tag = "2")]
    pub set: String,
    #[prost(message, repeated, tag = "3")]
    pub items: Vec<CollectionItem>,
}
