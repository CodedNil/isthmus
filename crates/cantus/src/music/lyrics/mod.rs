use super::{MusicResult, Track, spotify::Spotify};
use core::ops::Range;
pub(super) use lrclib::contributed_by_us;
pub use musixmatch::Session;
use reqwest::Client;
use serde_json::Value;
use strum_macros::{AsRefStr, Display, EnumIter, IntoStaticStr};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

mod apple;
mod lrclib;
mod musixmatch;
mod spotify;

pub(super) fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

pub(super) fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub(super) fn matches_track(track: &Track, matched: &Value, duration_key: &str) -> bool {
    let title = normalize(matched["track_name"].as_str().unwrap_or_default());
    let artist = normalize(matched["artist_name"].as_str().unwrap_or_default());
    !title.is_empty()
        && !artist.is_empty()
        && (title == normalize(&track.name) || title == normalize(track.compact_title()))
        && (track.artists.iter().any(|name| normalize(name) == artist) || normalize(&track.artists.join("")) == artist)
        && track.isrc.as_deref().zip(matched["isrc"].as_str()).is_none_or(|(a, b)| a.eq_ignore_ascii_case(b))
        && matched[duration_key]
            .as_f64()
            .is_none_or(|length| length == 0.0 || (length - f64::from(track.duration_ms) / 1000.0).abs() <= 5.0)
}

pub(super) fn separate(segments: &mut [LyricSegment]) {
    if let Some(word) = segments.last_mut()
        && !word.text.ends_with(char::is_whitespace)
    {
        word.text.push(' ');
    }
}

#[derive(Clone)]
pub struct LyricSegment {
    /// Playback position in milliseconds.
    pub time: Range<f32>,
    pub text: String,
}

/// One Lyricsfile line; line-synced text has no words, word-synced text does.
#[derive(Clone, Default)]
pub struct LyricLine {
    pub text: String,
    pub time: Range<f32>,
    pub words: Option<Vec<LyricSegment>>,
    /// Rendering metadata carried by providers that support overlapping voices.
    pub lane: usize,
    pub singer: Option<usize>,
}

#[derive(Default)]
pub struct Lyrics {
    /// Canonical synchronized content, shaped like Lyricsfile's `lines`.
    pub lines: Vec<LyricLine>,
    pub timing: Timing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, AsRefStr, Display, EnumIter)]
#[strum(serialize_all = "lowercase")]
pub enum Timing {
    #[default]
    Line,
    Word,
    Syllable,
}

impl Lyrics {
    pub(super) fn from_segments(segments: Vec<LyricSegment>, timing: Timing) -> Self {
        let lines = segments
            .into_iter()
            .map(|segment| LyricLine {
                text: segment.text.clone(),
                time: segment.time.clone(),
                words: (timing >= Timing::Word).then_some(vec![segment]),
                lane: 0,
                singer: Some(0),
            })
            .collect();
        Self { lines, timing }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.text.trim().is_empty())
    }
}

// Timing quality wins; ties retain provider priority. Keep errors only when no lyrics exist.
pub(super) fn prefer_lyrics(previous: MusicResult<Lyrics>, next: MusicResult<Lyrics>) -> MusicResult<Lyrics> {
    if prefer_next(&previous, &next) { next } else { previous }
}

fn prefer_next(previous: &MusicResult<Lyrics>, next: &MusicResult<Lyrics>) -> bool {
    let current = previous.as_ref().ok().filter(|lyrics| !lyrics.is_empty());
    next.as_ref().map_or_else(
        |_| current.is_none(),
        |candidate| !candidate.is_empty() && current.is_none_or(|lyrics| candidate.timing > lyrics.timing),
    )
}

pub(super) fn fragment_timing(lines: &[LyricLine]) -> Timing {
    if lines.iter().any(|line| {
        line.words.as_deref().unwrap_or_default().windows(2).any(|pair| {
            pair[0].text.ends_with(char::is_alphabetic)
                && pair[1].text.starts_with(char::is_alphabetic)
                && pair[0].time.start < pair[1].time.start
        })
    }) {
        Timing::Syllable
    } else {
        Timing::Word
    }
}

#[derive(Clone, Copy, IntoStaticStr, PartialEq, Eq)]
enum Source {
    #[strum(serialize = "LRCLIB")]
    Lrclib,
    Spotify,
    Musixmatch,
    Apple,
}

pub(super) async fn fetch(
    track: &Track,
    http: &Client,
    session: &Mutex<Session>,
    spotify: &Spotify,
    publish_enabled: bool,
) -> MusicResult<Lyrics> {
    let title = track.compact_title().split(" (").next().unwrap_or(&track.name);
    let artist = track.primary_artist();
    let cached = lrclib::fetch_lrclib(track, http).await;
    if let Err(error) = &cached {
        debug!(source = "LRCLIB", %error, "Lyrics provider unavailable");
    }
    let cached_source = cached.as_ref().ok().filter(|lyrics| !lyrics.is_empty()).map(|_| Source::Lrclib);
    let mut best = (cached_source, cached);
    if !best.1.as_ref().is_ok_and(|lyrics| !lyrics.is_empty() && lyrics.timing == Timing::Syllable) {
        let (spotify, musixmatch, apple) = tokio::join!(
            async {
                let Some(id) = track.id else { return Ok(Lyrics::default()) };
                spotify.get_lyrics(id).await.and_then(|response| spotify::parse_spotify(response, track.duration_ms))
            },
            async {
                let mut session = session.lock().await;
                if session.is_backing_off() {
                    return Ok(Lyrics::default());
                }
                session.fetch(track, http).await
            },
            apple::fetch_apple_lyrics(track, http),
        );
        for (source, result) in [(Source::Spotify, spotify), (Source::Musixmatch, musixmatch), (Source::Apple, apple)] {
            if let Err(error) = &result {
                debug!(source = Into::<&str>::into(source), %error, "Lyrics provider unavailable");
            }
            if prefer_next(&best.1, &result) {
                best = (result.is_ok().then_some(source), result);
            }
        }
    }
    let (selected_source, lyrics) = best;
    match (&lyrics, selected_source) {
        (Ok(lyrics), Some(source)) => {
            let source: &str = source.into();
            info!("♫  {title:.32}  ·  {artist:.24}  ·  {source}  ·  {} sync", lyrics.timing);
        }
        (Ok(_), None) => info!("♫  {title:.32}  ·  {artist:.24}  ·  no lyrics"),
        (Err(error), _) => info!("♫  {title:.32}  ·  {artist:.24}  ·  lyrics unavailable  ·  {error}"),
    }
    if let (Some(source), Ok(lyrics)) = (selected_source, &lyrics)
        && source != Source::Lrclib
        && publish_enabled
        && !contributed_by_us(track, lyrics)
    {
        match lrclib::publish_lrclib(track, lyrics, http, source.into()).await {
            Ok(status) => info!("LRCLIB upload succeeded: {title:.28} — {artist:.20} · HTTP {status}"),
            Err(error) => warn!("LRCLIB upload failed: {title:.28} — {artist:.20} · {error}"),
        }
    }
    lyrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{
        TrackRuntime,
        spotify::proto::lyrics::{Line, Sheet, SpotifyLyrics, SyncData},
    };
    use apple::parse_ttml;
    use prost::Message;
    use serde_json::json;

    fn parse_spotify(lines: Vec<Line>, sync_type: i32) -> MusicResult<Lyrics> {
        let response = SpotifyLyrics { lyrics: Some(Sheet { sync_type, lines }) };
        let decoded = SpotifyLyrics::decode(response.encode_to_vec().as_slice())?;
        spotify::parse_spotify(Some(decoded), 5000)
    }

    fn track() -> Track {
        Track {
            id: Some("4SJuENxvGDMBrjRJJjbYVe".parse().unwrap()),
            isrc: Some("NL2VN2000027".into()),
            uri: String::new(),
            name: "Lego Bricks".into(),
            original_title: None,
            artists: vec!["Euan Allison".into(), "Lily Williams".into()],
            album: "Lego Bricks".into(),
            image: None,
            duration_ms: 236_000,
            queue_uid: String::new(),
            interaction_id: Track::next_interaction_id(),
            runtime: TrackRuntime::default(),
        }
    }

    fn candidate(timing: Timing, text: &str) -> Lyrics {
        Lyrics::from_segments(vec![LyricSegment { time: 0.0..1000.0, text: text.into() }], timing)
    }

    #[test]
    fn quality_beats_provider_order_and_ties_keep_first() {
        for (first, second, expected) in [
            (Timing::Line, Timing::Word, "second"),
            (Timing::Word, Timing::Syllable, "second"),
            (Timing::Syllable, Timing::Word, "first"),
            (Timing::Word, Timing::Line, "first"),
            (Timing::Line, Timing::Line, "first"),
            (Timing::Word, Timing::Word, "first"),
            (Timing::Syllable, Timing::Syllable, "first"),
        ] {
            let chosen =
                prefer_lyrics(Ok(candidate(first, "first")), Ok(candidate(second, "second"))).expect("timed lyrics");
            assert_eq!(chosen.lines[0].text, expected);
        }
        let chosen =
            prefer_lyrics(Ok(candidate(Timing::Line, "line")), Err("provider down".into())).expect("keep lines");
        assert_eq!(chosen.timing, Timing::Line);
        assert!(prefer_lyrics(Err("provider down".into()), Ok(Lyrics::default())).is_err());
    }

    #[test]
    fn joint_artist_credit_matches_the_recording() {
        let track = track();
        let mut result = json!({"track_name":"Lego Bricks", "artist_name":"Euan Allison, Lily Williams",
            "duration":236,"isrc":"NL2VN2000027"});
        assert!(matches_track(&track, &result, "duration"));
        result["artist_name"] = json!("Euan Allison, Someone Else");
        assert!(!matches_track(&track, &result, "duration"));
        result["artist_name"] = json!("Euan Allison, Lily Williams");
        result["isrc"] = json!("OTHER");
        assert!(!matches_track(&track, &result, "duration"));
        result["isrc"] = json!("NL2VN2000027");
        result["duration"] = json!(210);
        assert!(!matches_track(&track, &result, "duration"));
    }

    #[test]
    fn spotify_uses_silence_markers_and_track_end_as_boundaries() {
        let lines = vec![
            Line { start_time_ms: 1000, words: "First".into(), ..Default::default() },
            Line { start_time_ms: 2000, words: "♪".into(), ..Default::default() },
            Line { start_time_ms: 3000, end_time_ms: 4200, words: "Last".into(), ..Default::default() },
            Line { start_time_ms: 4400, words: "End".into(), ..Default::default() },
        ];
        let lyrics = parse_spotify(lines, 1).expect("line timing");
        assert_eq!(lyrics.timing, Timing::Line);
        let lines = &lyrics.lines;
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].time, 1000.0..2000.0);
        assert_eq!(lines[1].time, 3000.0..4200.0);
        assert_eq!(lines[2].time, 4400.0..5000.0);
        assert!(parse_spotify(Vec::new(), 0).expect("no sync").is_empty());
    }

    #[test]
    fn spotify_syllables_use_utf16_counts_and_keep_word_spaces() {
        let mut line = Line {
            start_time_ms: 1000,
            end_time_ms: 4000,
            words: "🎵Hello world".into(),
            sync_data: vec![
                SyncData { start_time_ms: 1000, end_time_ms: 1500, num_chars: 5 },
                SyncData { start_time_ms: 1500, end_time_ms: 2000, num_chars: 2 },
                SyncData { start_time_ms: 2500, end_time_ms: 4000, num_chars: 6 },
            ],
        };
        let lyrics = parse_spotify(vec![line.clone()], 2).expect("syllable timing");
        assert_eq!(lyrics.timing, Timing::Syllable);
        let segments = lyrics.lines[0].words.as_ref().expect("word timings");
        assert_eq!(segments.iter().map(|segment| segment.text.as_str()).collect::<String>(), "🎵Hello world ");
        assert_eq!(segments[1].time, 1500.0..2000.0);
        line.sync_data[0].num_chars = 1;
        let lyrics = parse_spotify(vec![line], 2).expect("fallback to whole line");
        assert_eq!(lyrics.timing, Timing::Line);
        assert_eq!(lyrics.lines[0].text, "🎵Hello world ");
    }

    #[test]
    fn ttml_distinguishes_syllables_from_words() {
        for (source, expected) in [
            (r#"<tt><body><p begin="1s" end="3s">Hello world</p></body></tt>"#, Timing::Line),
            (
                r#"<tt><body><p><span begin="1s" end="2s">Hello</span> <span begin="2s" end="3s">world</span></p></body></tt>"#,
                Timing::Word,
            ),
            (
                r#"<tt><body><p><span begin="1s" end="1.5s">Hel</span><span begin="1.5s" end="2s">lo</span> <span begin="2s" end="3s">world</span></p></body></tt>"#,
                Timing::Syllable,
            ),
        ] {
            assert_eq!(parse_ttml(source).timing, expected);
        }
    }
}
