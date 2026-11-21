use std::collections::HashMap;

/// Dealer push carrying an updated Spotify Connect cluster.
#[derive(prost::Message)]
pub struct ClusterUpdate {
    #[prost(message, optional, tag = "1")]
    pub cluster: Option<Cluster>,
}

/// Device identity and playback state sent during Connect registration.
#[derive(prost::Message)]
pub struct Device {
    #[prost(message, optional, tag = "1")]
    pub device_info: Option<DeviceInfo>,
    #[prost(message, optional, tag = "2")]
    pub player_state: Option<PlayerState>,
}

/// Playback state for the active device and the server timestamp used to align clocks.
#[derive(prost::Message)]
pub struct Cluster {
    #[prost(string, tag = "2")]
    pub active_device_id: String,
    #[prost(message, optional, tag = "3")]
    pub player_state: Option<PlayerState>,
    #[prost(int64, tag = "9")]
    pub server_timestamp_ms: i64,
}

/// Registers or refreshes Cantus as a Spotify Connect observer.
#[derive(prost::Message)]
pub struct PutStateRequest {
    #[prost(message, optional, tag = "2")]
    pub device: Option<Device>,
    #[prost(enumeration = "MemberType", tag = "3")]
    pub member_type: i32,
    #[prost(enumeration = "PutStateReason", tag = "5")]
    pub put_state_reason: i32,
    #[prost(uint64, tag = "12")]
    pub client_side_timestamp: u64,
}

/// Client and device identifiers, display name, and advertised capabilities.
#[derive(prost::Message)]
pub struct DeviceInfo {
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(message, optional, tag = "4")]
    pub capabilities: Option<Capabilities>,
    #[prost(enumeration = "DeviceType", tag = "7")]
    pub device_type: i32,
    #[prost(string, tag = "10")]
    pub device_id: String,
    #[prost(string, tag = "13")]
    pub client_id: String,
}

/// Connect features advertised by the Cantus observer.
#[expect(clippy::struct_excessive_bools, reason = "Spotify assigns a separate protobuf tag to each capability")]
#[derive(prost::Message)]
pub struct Capabilities {
    #[prost(bool, tag = "7")]
    pub is_observable: bool,
    #[prost(string, repeated, tag = "9")]
    pub supported_types: Vec<String>,
    #[prost(bool, tag = "12")]
    pub hidden: bool,
    #[prost(bool, tag = "15")]
    pub supports_playlist_v2: bool,
    #[prost(bool, tag = "22")]
    pub needs_full_player_state: bool,
    #[prost(bool, tag = "23")]
    pub supports_gzip_pushes: bool,
}

/// Reported track, queue, playback flags, and timing for a Connect session.
#[derive(prost::Message)]
pub struct PlayerState {
    /// Server timestamp of this playback state in milliseconds.
    #[prost(int64, tag = "1")]
    pub timestamp: i64,
    #[prost(string, tag = "2")]
    pub context_uri: String,
    #[prost(message, optional, tag = "7")]
    pub track: Option<ProvidedTrack>,
    #[prost(double, tag = "9")]
    pub playback_speed: f64,
    /// Track position in milliseconds at `timestamp`.
    #[prost(int64, tag = "10")]
    pub position_as_of_timestamp: i64,
    #[prost(int64, tag = "11")]
    pub duration: i64,
    #[prost(bool, tag = "12")]
    pub is_playing: bool,
    #[prost(bool, tag = "13")]
    pub is_paused: bool,
    #[prost(bool, tag = "14")]
    pub is_buffering: bool,
    #[prost(message, optional, tag = "16")]
    pub options: Option<ContextPlayerOptions>,
    #[prost(message, optional, tag = "18")]
    pub suppressions: Option<Suppressions>,
    #[prost(message, repeated, tag = "19")]
    pub prev_tracks: Vec<ProvidedTrack>,
    #[prost(message, repeated, tag = "20")]
    pub next_tracks: Vec<ProvidedTrack>,
    #[prost(string, tag = "23")]
    pub session_id: String,
}

/// Playback preferences reported for the current context.
#[derive(prost::Message)]
pub struct ContextPlayerOptions {
    #[prost(bool, tag = "1")]
    pub shuffling_context: bool,
    #[prost(bool, tag = "2")]
    pub repeating_context: bool,
    #[prost(bool, tag = "3")]
    pub repeating_track: bool,
    #[prost(float, tag = "4")]
    pub playback_speed: f32,
    #[prost(map = "string, string", tag = "5")]
    pub modes: HashMap<String, String>,
}

/// Track providers suppressed by the active player.
#[derive(prost::Message)]
pub struct Suppressions {
    #[prost(string, repeated, tag = "1")]
    pub providers: Vec<String>,
}

/// One queue occurrence with its Spotify URI, occurrence ID, and metadata.
#[derive(prost::Message)]
pub struct ProvidedTrack {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(string, tag = "2")]
    pub uid: String,
    #[prost(map = "string, string", tag = "3")]
    pub metadata: HashMap<String, String>,
}

/// Reason Spotify Connect uses to distinguish device registration from reconnects.
#[derive(Debug, prost::Enumeration)]
pub enum PutStateReason {
    /// Default value when a reason is unavailable.
    Unknown = 0,
    /// Registers a device for the first time.
    NewDevice = 3,
    /// Registers again after reconnecting the Dealer socket.
    NewConnection = 9,
}

/// Connect protocol used by the registering device.
#[derive(Debug, prost::Enumeration)]
pub enum MemberType {
    /// Legacy Spotify remote-control membership.
    SpircV2 = 0,
    /// Membership through the Connect state service.
    ConnectState = 2,
}

/// Device role advertised to Spotify Connect.
#[derive(Debug, prost::Enumeration)]
pub enum DeviceType {
    /// Default value when a device role is unavailable.
    Unknown = 0,
    /// Observes playback on another Spotify device.
    Observer = 102,
}
