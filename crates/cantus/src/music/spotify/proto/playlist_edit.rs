/// Playlist entry identified by its Spotify URI.
#[derive(prost::Message)]
pub struct Item {
    #[prost(string, required, tag = "1")]
    pub uri: String,
}

/// Appends entries to a playlist.
#[derive(prost::Message)]
pub struct Add {
    #[prost(message, repeated, tag = "2")]
    pub items: Vec<Item>,
    #[prost(bool, optional, tag = "4")]
    pub add_last: Option<bool>,
}

/// Removes entries by URI.
#[derive(prost::Message)]
pub struct Remove {
    #[prost(message, repeated, tag = "3")]
    pub items: Vec<Item>,
    /// Treat the supplied entry URIs as removal keys.
    #[prost(bool, optional, tag = "7")]
    pub items_as_key: Option<bool>,
}

/// One playlist edit, with a kind that selects the matching payload.
#[derive(prost::Message)]
pub struct Operation {
    #[prost(enumeration = "OperationKind", required, tag = "1")]
    pub kind: i32,
    #[prost(message, optional, tag = "2")]
    pub add: Option<Add>,
    #[prost(message, optional, tag = "3")]
    pub remove: Option<Remove>,
}

/// Wire discriminator for the supported playlist edit payloads.
#[derive(Debug, prost::Enumeration)]
pub enum OperationKind {
    /// Default value for an unrecognized edit kind.
    Unknown = 0,
    /// Inserts playlist entries.
    Add = 2,
    /// Removes playlist entries.
    Remove = 3,
}

/// Ordered edits based on one playlist revision.
#[derive(prost::Message)]
pub struct Delta {
    #[prost(bytes, optional, tag = "1")]
    pub base_version: Option<Vec<u8>>,
    #[prost(message, repeated, tag = "2")]
    pub ops: Vec<Operation>,
}

/// Write request carrying the base revision and edits.
#[derive(prost::Message)]
pub struct ListChanges {
    #[prost(bytes, optional, tag = "1")]
    pub base_revision: Option<Vec<u8>>,
    #[prost(message, repeated, tag = "2")]
    pub deltas: Vec<Delta>,
}

/// Revision and change hints from a Dealer playlist or root-list notification.
#[derive(prost::Message)]
pub struct PlaylistNotification {
    /// Playlist URI, or the new revision in a root-list notification.
    #[prost(bytes, tag = "1")]
    pub uri_or_revision: Vec<u8>,
    /// Playlist revision, or the parent revision in a root-list notification.
    #[prost(bytes, tag = "2")]
    pub revision_or_parent: Vec<u8>,
    #[prost(message, repeated, tag = "4")]
    pub ops: Vec<NotificationOperation>,
}

/// Operation kind used to distinguish membership changes from possible playlist renames.
#[derive(prost::Message)]
pub struct NotificationOperation {
    #[prost(enumeration = "OperationKind", tag = "1")]
    pub kind: i32,
}
