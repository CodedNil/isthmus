use serde::{Deserialize, Deserializer, de::Error as _};
use std::{collections::HashMap, fmt::Display, str::FromStr};

/// Dealer push carrying an updated Spotify Connect cluster.
#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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
#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cluster {
    #[prost(string, tag = "2")]
    #[serde(alias = "active_device_id", deserialize_with = "default_on_null")]
    pub active_device_id: String,
    #[prost(message, optional, tag = "3")]
    #[serde(alias = "player_state")]
    pub player_state: Option<PlayerState>,
    #[prost(int64, tag = "9")]
    #[serde(alias = "server_timestamp_ms", deserialize_with = "number")]
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
#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlayerState {
    /// Server timestamp of this playback state in milliseconds.
    #[prost(int64, tag = "1")]
    #[serde(deserialize_with = "number")]
    pub timestamp: i64,
    #[prost(string, tag = "2")]
    #[serde(alias = "context_uri", deserialize_with = "default_on_null")]
    pub context_uri: String,
    #[prost(message, optional, tag = "7")]
    pub track: Option<ProvidedTrack>,
    #[prost(double, tag = "9")]
    #[serde(alias = "playback_speed", deserialize_with = "number")]
    pub playback_speed: f64,
    /// Track position in milliseconds at `timestamp`.
    #[prost(int64, tag = "10")]
    #[serde(alias = "position_as_of_timestamp", deserialize_with = "number")]
    pub position_as_of_timestamp: i64,
    #[prost(int64, tag = "11")]
    #[serde(deserialize_with = "number")]
    pub duration: i64,
    #[prost(bool, tag = "12")]
    #[serde(alias = "is_playing", deserialize_with = "default_on_null")]
    pub is_playing: bool,
    #[prost(bool, tag = "13")]
    #[serde(alias = "is_paused", deserialize_with = "default_on_null")]
    pub is_paused: bool,
    #[prost(bool, tag = "14")]
    #[serde(alias = "is_buffering", deserialize_with = "default_on_null")]
    pub is_buffering: bool,
    /// Cantus sends an empty options message when registering its observer.
    #[prost(message, optional, tag = "16")]
    #[serde(skip)]
    pub options: Option<()>,
    /// Cantus sends an empty suppressions message when registering its observer.
    #[prost(message, optional, tag = "18")]
    #[serde(skip)]
    pub suppressions: Option<()>,
    #[prost(message, repeated, tag = "19")]
    #[serde(alias = "prev_tracks", deserialize_with = "default_on_null")]
    pub prev_tracks: Vec<ProvidedTrack>,
    #[prost(message, repeated, tag = "20")]
    #[serde(alias = "next_tracks", deserialize_with = "default_on_null")]
    pub next_tracks: Vec<ProvidedTrack>,
    #[prost(string, tag = "23")]
    #[serde(alias = "session_id", deserialize_with = "default_on_null")]
    pub session_id: String,
}

/// One queue occurrence with its Spotify URI, occurrence ID, and metadata.
#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProvidedTrack {
    #[prost(string, tag = "1")]
    #[serde(deserialize_with = "default_on_null")]
    pub uri: String,
    /// Distinguishes repeated occurrences of the same URI in the queue.
    #[prost(string, tag = "2")]
    #[serde(deserialize_with = "default_on_null")]
    pub uid: String,
    #[prost(map = "string, string", tag = "3")]
    #[serde(deserialize_with = "default_on_null")]
    pub metadata: HashMap<String, String>,
}

/// Reason Spotify Connect uses to distinguish device registration from reconnects.
#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum PutStateReason {
    /// Default value when a reason is unavailable.
    Unknown = 0,
    /// Registers a device for the first time.
    NewDevice = 3,
    /// Registers again after reconnecting the Dealer socket.
    NewConnection = 9,
}

/// Connect protocol used by the registering device.
#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum MemberType {
    /// Legacy Spotify remote-control membership.
    SpircV2 = 0,
    /// Membership through the Connect state service.
    ConnectState = 2,
}

/// Device role advertised to Spotify Connect.
#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum DeviceType {
    /// Default value when a device role is unavailable.
    Unknown = 0,
    /// Observes playback on another Spotify device.
    Observer = 102,
}

// Dealer may use explicit JSON nulls for absent protobuf fields.
fn default_on_null<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(input: D) -> Result<T, D::Error> {
    Ok(Option::deserialize(input)?.unwrap_or_default())
}

// Dealer sends some numbers as JSON strings.
fn number<'de, D, T>(input: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default + FromStr<Err: Display>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number<T> {
        Number(T),
        String(String),
    }
    match Option::<Number<T>>::deserialize(input)? {
        Some(Number::Number(value)) => Ok(value),
        Some(Number::String(value)) => value.parse().map_err(D::Error::custom),
        None => Ok(T::default()),
    }
}
