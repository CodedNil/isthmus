use super::{Enrichment, Fetch, TRACK_SPACING_MS, Track, TrackId, spotify::Spotify};
use crate::app::update;
use isthmus::{
    geometry::text,
    glam::{FloatExt, vec2},
};
use quick_xml::{
    Reader, XmlVersion,
    escape::unescape,
    events::{BytesStart, Event},
};
use reqwest::Client;
use serde::Deserialize;
use std::{mem, ops::Range};
use tracing::warn;

const API: &str = "https://lyrics-api.binimum.org/";
#[derive(Clone)]
pub struct LyricSegment {
    pub start_ms: f32,
    pub end_ms: f32,
    pub text: String,
    pub background: bool,
    pub break_after: bool,
}

impl LyricSegment {
    pub(super) fn line(start_ms: f32, next_start_ms: Option<f32>, text: String) -> Self {
        const CHARACTER_MS: f32 = 100.0;
        let estimated_end = start_ms + text.chars().count().max(10) as f32 * CHARACTER_MS;
        Self {
            start_ms,
            end_ms: next_start_ms.map_or(estimated_end, |next| estimated_end.min(next)),
            text,
            background: false,
            break_after: true,
        }
    }
}

pub(super) struct LyricsRequest {
    uri: String,
    track_id: Option<TrackId>,
    name: String,
    artist: String,
    album: String,
    duration_ms: u32,
}

impl From<&Track> for LyricsRequest {
    fn from(track: &Track) -> Self {
        Self {
            uri: track.uri.clone(),
            track_id: track.id,
            name: track.name.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
        }
    }
}

#[derive(Default)]
pub struct Lyrics {
    segments: Option<Vec<LyricSegment>>,
    words: Vec<PositionedLyric>,
    timeline: Vec<(f32, f32)>,
    pub(crate) span: f32,
}

struct PositionedLyric {
    text: String,
    background: bool,
    position: f32,
    width: f32,
}

impl Lyrics {
    pub(crate) const SILENCE_SPEED: f32 = 0.035;
    const SONG_GAP: f32 = 96.0;

    fn new(segments: Vec<LyricSegment>) -> Self {
        Self { segments: Some(segments), ..Self::default() }
    }

    pub(crate) fn prepare(&mut self, duration_ms: f32, shaper: &text::Shaper) {
        let Some(segments) = self.segments.take() else { return };
        *self = Self::shape(segments, duration_ms, shaper);
    }

    fn shape(mut segments: Vec<LyricSegment>, duration_ms: f32, shaper: &text::Shaper) -> Self {
        segments.retain(|segment| !segment.text.trim().is_empty());
        segments.sort_by(|left, right| left.start_ms.total_cmp(&right.start_ms));
        if segments.is_empty() {
            return Self::default();
        }

        let mut words = Vec::with_capacity(segments.len());
        let mut timeline = vec![(0.0, 0.0)];
        let mut cursor = 0.0;
        let mut vocal_end = 0.0;
        let space = shaper.width(" ", 15.0, 700.0);
        for segment in &segments {
            let silence = (segment.start_ms - vocal_end).max(0.0);
            cursor += silence * Self::SILENCE_SPEED;
            let value = segment.text.trim_start();
            let width = shaper.width(value, 15.0, 700.0);
            let position = cursor;
            words.push(PositionedLyric { text: value.into(), background: segment.background, position, width });
            cursor += width + space * f32::from(segment.break_after);
            let end_ms = segment.end_ms.max(segment.start_ms);
            vocal_end = vocal_end.max(end_ms);
            timeline.push((segment.start_ms, position));
            timeline.push((end_ms, position + width));
        }

        let position = cursor + (duration_ms - vocal_end).max(0.0) * Self::SILENCE_SPEED;
        timeline.push((duration_ms.max(vocal_end), position));
        timeline.sort_by(|left, right| left.0.total_cmp(&right.0));
        Self { segments: None, words, timeline, span: position + Self::SONG_GAP }
    }

    pub(crate) fn position(&self, time: f32, duration_ms: f32) -> f32 {
        if time > duration_ms {
            let end = self.timeline_position(duration_ms);
            return end.lerp(self.span, ((time - duration_ms) / TRACK_SPACING_MS).clamp(0.0, 1.0));
        }
        self.timeline_position(time)
    }

    fn timeline_position(&self, time: f32) -> f32 {
        let upper = self.timeline.partition_point(|&(at, _)| at <= time);
        match (upper.checked_sub(1), self.timeline.get(upper)) {
            (None, _) => self.timeline.first().map_or(0.0, |&(_, x)| x),
            (Some(lower), None) => self.timeline[lower].1,
            (Some(lower), Some(&(t1, x1))) => {
                let (t0, x0) = self.timeline[lower];
                x0.lerp(x1, ((time - t0) / (t1 - t0).max(f32::EPSILON)).clamp(0.0, 1.0))
            }
        }
    }

    pub(crate) fn visible(&self, shaper: &text::Shaper, range: Range<f32>, background: bool) -> text::ShapedLine {
        shaper.shape_positioned(
            self.words
                .iter()
                .filter(|word| {
                    word.background == background
                        && word.position <= range.end
                        && word.position + word.width >= range.start
                })
                .map(|word| (word.text.as_str(), vec2(word.position, 0.0))),
            15.0,
            700.0,
        )
    }
}

impl Enrichment {
    pub(super) fn request_lyrics(&self, request: LyricsRequest, spotify: Spotify) {
        let http = self.http.clone();
        self.background.spawn_update(async move {
            let uri = request.uri.clone();
            let result = fetch(&request, &http, &spotify).await;
            Some(update(move |app| {
                for track in app
                    .music
                    .queue
                    .iter_mut()
                    .filter(|track| track.uri == uri && matches!(track.runtime.lyrics, Fetch::Fetching))
                {
                    track.runtime.lyrics = match &result {
                        Ok(segments) => Fetch::Ready(Lyrics::new(segments.clone())),
                        Err(()) => Fetch::retry(),
                    };
                }
            }))
        });
    }
}

async fn fetch(request: &LyricsRequest, http: &Client, spotify: &Spotify) -> Result<Vec<LyricSegment>, ()> {
    let result = match fetch_precise(http, request).await {
        Some(segments) => Ok(segments),
        None => match request.track_id {
            Some(id) => spotify.lyrics(id).await,
            None => Ok(Vec::new()),
        },
    };
    result.map_err(|error| {
        warn!(%error, track = request.name, "Failed to fetch lyrics");
    })
}

#[derive(Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    #[serde(rename = "lyricsUrl")]
    url: String,
    timing_type: String,
}

async fn fetch_precise(http: &Client, query: &LyricsRequest) -> Option<Vec<LyricSegment>> {
    let result = http
        .get(API)
        .query(&[
            ("track", query.name.clone()),
            ("artist", query.artist.clone()),
            ("album", query.album.clone()),
            ("duration", (query.duration_ms / 1000).to_string()),
        ])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<SearchResponse>()
        .await
        .ok()?
        .results
        .into_iter()
        .find(|result| result.timing_type == "word")?;
    let source = http.get(result.url).send().await.ok()?.error_for_status().ok()?.text().await.ok()?;
    let segments = parse_ttml(&source);
    (!segments.is_empty()).then_some(segments)
}

fn time(value: &str) -> Option<f32> {
    value
        .strip_suffix('s')
        .unwrap_or(value)
        .split(':')
        .try_fold(0.0, |total, part| Some(total * 60.0 + part.parse::<f32>().ok()?))
        .map(|seconds| seconds * 1000.0)
}

fn attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|attr| attr.key.local_name().as_ref() == name)?
        .normalized_value(XmlVersion::Implicit1_0)
        .ok()
        .map(std::borrow::Cow::into_owned)
}

fn parse_ttml(source: &str) -> Vec<LyricSegment> {
    let mut reader = Reader::from_str(source);
    let (mut segments, mut in_line) = (Vec::new(), false);
    let mut line_start = 0;
    let mut line_time = None;
    let mut line_text = String::new();
    let mut span_roles = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == "p" => {
                span_roles.clear();
                line_text.clear();
                line_time = attribute(&tag, "begin")
                    .as_deref()
                    .and_then(time)
                    .zip(attribute(&tag, "end").as_deref().and_then(time));
                in_line = true;
                line_start = segments.len();
            }
            Ok(Event::Start(tag)) if in_line && tag.local_name().as_ref() == "span" => {
                let start = attribute(&tag, "begin").as_deref().and_then(time);
                let end = attribute(&tag, "end").as_deref().and_then(time);
                span_roles.push(match attribute(&tag, "role").as_deref() {
                    Some("x-bg") => (true, false),
                    Some("x-translation" | "x-roman") => (false, true),
                    _ => (false, false),
                });
                if !span_roles.iter().any(|&(_, ignored)| ignored)
                    && let Some(start_ms) = start
                {
                    segments.push(LyricSegment {
                        start_ms,
                        end_ms: end.unwrap_or(start_ms + 1_000.0),
                        text: String::new(),
                        background: span_roles.iter().any(|&(background, _)| background),
                        break_after: false,
                    });
                }
            }
            Ok(Event::Text(value)) if in_line && !span_roles.iter().any(|&(_, ignored)| ignored) => {
                let value = value.xml_content(XmlVersion::Implicit1_0);
                let Ok(value) = unescape(&value) else { return Vec::new() };
                line_text.push_str(&value);
                if segments.len() > line_start {
                    let segment = &mut segments.last_mut().unwrap().text;
                    if value.chars().all(char::is_whitespace) {
                        if !segment.ends_with(char::is_whitespace) {
                            segment.push(' ');
                        }
                    } else {
                        segment.push_str(&value);
                    }
                }
            }
            Ok(Event::End(tag)) if tag.local_name().as_ref() == "span" => {
                span_roles.pop();
            }
            Ok(Event::End(tag)) if tag.local_name().as_ref() == "p" => {
                if segments.len() == line_start
                    && let Some((start_ms, end_ms)) = line_time
                    && !line_text.trim().is_empty()
                {
                    segments.push(LyricSegment {
                        start_ms,
                        end_ms,
                        text: mem::take(&mut line_text),
                        background: false,
                        break_after: false,
                    });
                }
                if segments.len() > line_start {
                    segments.last_mut().unwrap().break_after = true;
                }
                in_line = false;
                span_roles.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => return Vec::new(),
            _ => {}
        }
    }
    segments
}
