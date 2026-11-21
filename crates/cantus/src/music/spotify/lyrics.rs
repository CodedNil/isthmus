use serde::Deserialize;

#[derive(Deserialize)]
pub struct SpotifyLyrics {
    pub lyrics: Option<Sheet>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    pub sync_type: Option<SyncType>,
    pub lines: Option<Vec<Line>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncType {
    LineSynced,
    WordSynced,
    SyllableSynced,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub words: Option<String>,
    pub start_time_ms: Option<Milliseconds>,
    pub end_time_ms: Option<Milliseconds>,
    pub syllables: Option<Vec<Syllable>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Syllable {
    pub text: Option<String>,
    pub start_time_ms: Option<Milliseconds>,
    pub end_time_ms: Option<Milliseconds>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Milliseconds {
    Number(f32),
    String(String),
}

impl Milliseconds {
    pub fn value(&self) -> Option<f32> {
        let value = match self {
            Self::Number(value) => *value,
            Self::String(value) => value.parse().ok()?,
        };
        value.is_finite().then_some(value)
    }
}
