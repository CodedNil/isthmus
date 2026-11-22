//! Protobuf fields used from Spotify's v3 lyrics response.

#[derive(prost::Message)]
pub struct SpotifyLyrics {
    #[prost(message, optional, tag = "1")]
    pub lyrics: Option<Sheet>,
}

#[derive(prost::Message)]
pub struct Sheet {
    #[prost(int32, tag = "1")]
    pub sync_type: i32,
    #[prost(message, repeated, tag = "2")]
    pub lines: Vec<Line>,
}

#[derive(Clone, prost::Message)]
pub struct Line {
    #[prost(int64, tag = "1")]
    pub start_time_ms: i64,
    #[prost(int64, tag = "2")]
    pub end_time_ms: i64,
    #[prost(string, tag = "3")]
    pub words: String,
    #[prost(message, repeated, tag = "4")]
    pub sync_data: Vec<SyncData>,
}

#[derive(Clone, prost::Message)]
pub struct SyncData {
    #[prost(int64, tag = "1")]
    pub start_time_ms: i64,
    #[prost(int64, tag = "2")]
    pub end_time_ms: i64,
    #[prost(int64, tag = "3")]
    pub num_chars: i64,
}
