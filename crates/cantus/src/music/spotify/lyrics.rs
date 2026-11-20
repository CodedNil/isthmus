use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct SpotifyLyrics {
    pub(super) lyrics: Option<Sheet>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Sheet {
    pub(super) sync_type: Option<SyncType>,
    pub(super) lines: Option<Vec<Line>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum SyncType {
    LineSynced,
    WordSynced,
    SyllableSynced,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Line {
    pub(super) words: Option<String>,
    pub(super) start_time_ms: Option<Milliseconds>,
    pub(super) end_time_ms: Option<Milliseconds>,
    pub(super) syllables: Option<Vec<Syllable>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Syllable {
    pub(super) text: Option<String>,
    pub(super) start_time_ms: Option<Milliseconds>,
    pub(super) end_time_ms: Option<Milliseconds>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Milliseconds {
    Number(f32),
    String(String),
}

impl Milliseconds {
    pub(super) fn value(&self) -> Option<f32> {
        let value = match self {
            Self::Number(value) => *value,
            Self::String(value) => value.parse().ok()?,
        };
        value.is_finite().then_some(value)
    }
}
