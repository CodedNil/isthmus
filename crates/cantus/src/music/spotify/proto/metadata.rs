/// Artist credit returned in track metadata.
#[derive(prost::Message)]
pub struct Artist {
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
}

/// Album name and cover images returned with a track.
#[derive(prost::Message)]
pub struct Album {
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    #[prost(message, repeated, tag = "9")]
    pub cover: Vec<Image>,
    #[prost(message, optional, tag = "17")]
    pub cover_group: Option<ImageGroup>,
}

/// Subset of Spotify catalog track metadata used by Cantus.
#[derive(prost::Message)]
pub struct Track {
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    #[prost(message, optional, tag = "3")]
    pub album: Option<Album>,
    #[prost(message, repeated, tag = "4")]
    pub artist: Vec<Artist>,
    #[prost(sint32, optional, tag = "7")]
    pub duration: Option<i32>,
    #[prost(message, repeated, tag = "10")]
    pub external_id: Vec<ExternalId>,
    #[prost(string, optional, tag = "27")]
    pub original_title: Option<String>,
}

/// Cover image file ID and pixel width.
#[derive(prost::Message)]
pub struct Image {
    #[prost(bytes, optional, tag = "1")]
    pub file_id: Option<Vec<u8>>,
    #[prost(sint32, optional, tag = "3")]
    pub width: Option<i32>,
}

/// Set of cover image variants for one album.
#[derive(prost::Message)]
pub struct ImageGroup {
    #[prost(message, repeated, tag = "1")]
    pub image: Vec<Image>,
}

/// External identifier with a scheme and value, such as an ISRC.
#[derive(prost::Message)]
pub struct ExternalId {
    #[prost(string, optional, tag = "1")]
    pub r#type: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub id: Option<String>,
}
