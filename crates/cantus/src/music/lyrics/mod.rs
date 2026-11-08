use super::{MusicResult, Track, spotify::Spotify};
pub use musixmatch::Session;
use reqwest::Client;
use serde_json::Value;
use std::ops::Range;
use strum_macros::{AsRefStr, Display};
use tokio::sync::Mutex;
use tracing::{info, warn};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, AsRefStr, Display)]
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

    pub fn word_count(&self) -> usize {
        self.lines.iter().map(|line| line.words.as_ref().map_or(0, Vec::len)).sum()
    }
}

// Timing quality wins; ties retain provider priority. Keep errors only when no lyrics exist.
pub(super) fn prefer_lyrics(previous: MusicResult<Lyrics>, next: MusicResult<Lyrics>) -> MusicResult<Lyrics> {
    match (previous, next) {
        (Ok(previous), Ok(next)) if improves(&next, Some(&previous)) => Ok(next),
        (Ok(lyrics), _) | (_, Ok(lyrics)) if !lyrics.is_empty() => Ok(lyrics),
        (_, Err(error)) => Err(error),
        (previous, _) => previous,
    }
}

fn improves(candidate: &Lyrics, previous: Option<&Lyrics>) -> bool {
    !candidate.is_empty() && previous.is_none_or(|previous| previous.is_empty() || candidate.timing > previous.timing)
}

fn describe_result(result: &MusicResult<Lyrics>) -> String {
    match result {
        Ok(lyrics) if !lyrics.is_empty() => {
            let lanes = lyrics.lines.iter().map(|line| line.lane).max().unwrap_or(0) + 1;
            format!(
                "{} sync  ·  {} lines  ·  {} words  ·  {lanes} lanes",
                lyrics.timing,
                lyrics.lines.len(),
                lyrics.word_count()
            )
        }
        Ok(_) => "no lyrics".to_owned(),
        Err(error) => format!("unavailable · {error}"),
    }
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

#[derive(Clone, Copy, Display, PartialEq, Eq)]
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
    let mut best = lrclib::fetch_lrclib(track, http).await;
    let mut selected_source = best.as_ref().is_ok_and(|lyrics| !lyrics.is_empty()).then_some(Source::Lrclib);
    let title = track.compact_title().split(" (").next().unwrap_or(&track.name);
    let artist = track.primary_artist();
    let mut options = vec![(Source::Lrclib, describe_result(&best))];
    for source in [Source::Spotify, Source::Musixmatch, Source::Apple] {
        if best.as_ref().is_ok_and(|lyrics| lyrics.timing >= Timing::Word) {
            break;
        }
        let result = match source {
            Source::Lrclib => continue,
            Source::Spotify => {
                let Some(id) = track.id else { continue };
                spotify.get_lyrics(id).await.and_then(|response| spotify::parse_spotify(&response, track.duration_ms))
            }
            Source::Musixmatch => {
                let mut session = session.lock().await;
                if cfg!(target_arch = "wasm32") || session.is_backing_off() {
                    continue;
                }
                session.fetch(track, http).await
            }
            Source::Apple => apple::fetch_apple_lyrics(track, http).await,
        };
        options.push((source, describe_result(&result)));
        if result.as_ref().is_ok_and(|candidate| improves(candidate, best.as_ref().ok())) {
            selected_source = Some(source);
        }
        best = prefer_lyrics(best, result);
        if best.as_ref().is_ok_and(|lyrics| lyrics.timing >= Timing::Word) {
            break;
        }
    }
    let mut report = vec![format!("♫  {title:.32}  ·  {artist:.24}")];
    for (source, details) in options {
        let selected = selected_source == Some(source) && best.as_ref().is_ok_and(|lyrics| !lyrics.is_empty());
        let marker = if selected { "★" } else { "·" };
        report.push(format!(" {marker} {source:<10} {details}"));
    }
    info!("{}", report.join("\n"));
    if selected_source.is_some_and(|source| source != Source::Lrclib)
        && let Ok(lyrics) = &best
        && !lyrics.is_empty()
        && publish_enabled
    {
        let already_uploaded = match lrclib::was_uploaded(track, lyrics) {
            Ok(already_uploaded) => already_uploaded,
            Err(error) => {
                warn!("Could not read LRCLIB upload history; continuing with publish: {error}");
                false
            }
        };
        if !already_uploaded {
            // Contribution is best-effort; playback must not fail because of publishing.
            let words = lyrics.word_count();
            let source = selected_source.map_or_else(|| "unknown".to_owned(), |source| source.to_string());
            match lrclib::publish_lrclib(track, lyrics, http, &source).await {
                Ok(status) => info!(
                    "LRCLIB upload succeeded: {title:.28} — {artist:.20} · {} · {} lines · {words} words · HTTP {status}",
                    lyrics.timing,
                    lyrics.lines.len()
                ),
                Err(error) => warn!(
                    "LRCLIB upload failed: {title:.28} — {artist:.20} · {} · {} lines · {words} words · {error}",
                    lyrics.timing,
                    lyrics.lines.len()
                ),
            }
        }
    }
    best
}

pub(super) fn contributed_by_us(track: &Track, lyrics: &Lyrics) -> bool {
    match lrclib::was_uploaded(track, lyrics) {
        Ok(contributed) => contributed,
        Err(error) => {
            warn!("Could not read LRCLIB upload history: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apple::{matches_apple_result, parse_ttml};
    use serde_json::json;
    use spotify::parse_spotify;

    fn track() -> Track {
        serde_json::from_value(json!({
            "id":"4SJuENxvGDMBrjRJJjbYVe", "name":"Lego Bricks", "album":"Lego Bricks",
            "artists":["Euan Allison", "Lily Williams"], "duration_ms":236_000, "isrc":"NL2VN2000027"
        }))
        .expect("track fixture")
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
        assert!(matches_apple_result(&track, &result));
        result["artist_name"] = json!("Euan Allison, Someone Else");
        assert!(!matches_apple_result(&track, &result));
        result["artist_name"] = json!("Euan Allison, Lily Williams");
        result["isrc"] = json!("OTHER");
        assert!(!matches_apple_result(&track, &result));
        result["isrc"] = json!("NL2VN2000027");
        result["duration"] = json!(210);
        assert!(!matches_apple_result(&track, &result));
    }

    #[test]
    fn spotify_uses_silence_markers_and_track_end_as_boundaries() {
        let response = json!({"lyrics":{"syncType":"LINE_SYNCED","lines":[
            {"startTimeMs":"1000","endTimeMs":"0","words":"First"},
            {"startTimeMs":"2000","endTimeMs":"0","words":"♪"},
            {"startTimeMs":"3000","endTimeMs":"0","words":"Last"}
        ]}});
        let lyrics = parse_spotify(&response, 5000).expect("line timing");
        assert_eq!(lyrics.timing, Timing::Line);
        let lines = &lyrics.lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time, 1000.0..2000.0);
        assert_eq!(lines[1].time, 3000.0..5000.0);
        assert!(parse_spotify(&json!({"lyrics":{"syncType":"UNSYNCED"}}), 5000).expect("no sync").is_empty());
    }

    #[test]
    fn spotify_syllables_must_cover_the_line_and_keep_word_spaces() {
        let mut response = json!({"lyrics":{"syncType":"LINE_SYNCED","lines":[
            {"startTimeMs":"1000","endTimeMs":"4000","words":"Hello world","syllables":[
                {"startTimeMs":"1000","endTimeMs":"1500","text":"Hel"},
                {"startTimeMs":"1500","endTimeMs":"2000","text":"lo"},
                {"startTimeMs":"2500","endTimeMs":"4000","text":"world"}
            ]}
        ]}});
        let lyrics = parse_spotify(&response, 5000).expect("syllable timing");
        assert_eq!(lyrics.timing, Timing::Syllable);
        let segments = lyrics.lines[0].words.as_ref().expect("word timings");
        assert_eq!(segments.iter().map(|segment| segment.text.as_str()).collect::<String>(), "Hello world ");
        assert_eq!(segments[1].time, 1500.0..2000.0);
        response["lyrics"]["lines"][0]["syllables"][2]["text"] = json!("wrong");
        let lyrics = parse_spotify(&response, 5000).expect("fallback to whole line");
        assert_eq!(lyrics.timing, Timing::Line);
        assert_eq!(lyrics.lines[0].text, "Hello world ");
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
