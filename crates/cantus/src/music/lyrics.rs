use super::{TRACK_SPACING_MS, Track, enrichment::Fetch, spotify::Spotify};
use crate::app::fetch_json;
use isthmus::glam::{FloatExt, vec2};
use isthmus_sdf::layout::{ShapedLine, TextCache};
use reqwest::Client;
use roxmltree::{Document, Node};
use serde::Deserialize;
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

#[derive(Default)]
pub struct Lyrics {
    segments: Option<Vec<LyricSegment>>,
    pub(crate) lines: [ShapedLine; 2],
    timeline: Vec<(f32, f32)>,
}

impl Lyrics {
    const SILENCE_SPEED: f32 = 0.035;
    const SONG_GAP: f32 = 96.0;

    pub(crate) fn prepare(&mut self, duration_ms: f32, text: &mut TextCache) {
        let Some(mut segments) = self.segments.take() else { return };
        segments.retain(|segment| !segment.text.trim().is_empty());
        segments.sort_by(|left, right| left.start_ms.total_cmp(&right.start_ms));
        if segments.is_empty() {
            return;
        }

        let mut words = Vec::with_capacity(segments.len());
        let mut timeline = vec![(0.0, 0.0)];
        let mut cursor = 0.0;
        let mut vocal_end = 0.0;
        let space = text.shape(" ", 15.0, 700.0).text.width;
        for segment in &mut segments {
            let silence = (segment.start_ms - vocal_end).max(0.0);
            cursor += silence * Self::SILENCE_SPEED;
            segment.text = segment.text.replace('🎵', "♪").replace('🎶', "♫");
            let value = segment.text.trim_start();
            let width = text.shape(value, 15.0, 700.0).text.width;
            let position = cursor;
            words.push((segment.background, value, position));
            cursor += width + space * f32::from(segment.break_after);
            let end_ms = segment.end_ms.max(segment.start_ms);
            vocal_end = vocal_end.max(end_ms);
            timeline.push((segment.start_ms, position));
            timeline.push((end_ms, position + width));
        }

        let position = cursor + (duration_ms - vocal_end).max(0.0) * Self::SILENCE_SPEED;
        timeline.push((duration_ms.max(vocal_end), position));
        timeline.sort_by(|left, right| left.0.total_cmp(&right.0));
        let lines = [false, true].map(|background| {
            text.shape_positioned(
                words.iter().filter(|word| word.0 == background).map(|word| (word.1, vec2(word.2, 0.0))),
                15.0,
                700.0,
            )
        });
        *self = Self { segments: None, lines, timeline };
        let end = self.position(duration_ms);
        self.timeline.retain(|&(at, _)| at < duration_ms);
        self.timeline.extend([(duration_ms, end), (duration_ms + TRACK_SPACING_MS, position + Self::SONG_GAP)]);
    }

    pub(crate) fn span(&self, duration_ms: f32) -> f32 {
        self.timeline.last().map_or((duration_ms + TRACK_SPACING_MS) * Self::SILENCE_SPEED, |&(_, x)| x)
    }

    pub(crate) fn position(&self, time: f32) -> f32 {
        let upper = self.timeline.partition_point(|&(at, _)| at <= time);
        match (upper.checked_sub(1), self.timeline.get(upper)) {
            (None, _) => self.timeline.first().map_or(time * Self::SILENCE_SPEED, |&(_, x)| x),
            (Some(lower), None) => self.timeline[lower].1,
            (Some(lower), Some(&(t1, x1))) => {
                let (t0, x0) = self.timeline[lower];
                x0.lerp(x1, ((time - t0) / (t1 - t0).max(f32::EPSILON)).clamp(0.0, 1.0))
            }
        }
    }
}

pub(super) async fn fetch(request: &Track, http: &Client, spotify: &Spotify) -> Fetch<Lyrics> {
    let result = match fetch_precise(http, request).await {
        Some(segments) => Ok(segments),
        None => match request.id {
            Some(id) => spotify.lyrics(id).await,
            None => Ok(Vec::new()),
        },
    };
    result
        .inspect_err(|error| warn!(%error, track = request.name, "Failed to fetch lyrics"))
        .map(|segments| Lyrics { segments: Some(segments), ..Default::default() })
        .map_or_else(|_| Fetch::retry(), Fetch::Ready)
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

async fn fetch_precise(http: &Client, query: &Track) -> Option<Vec<LyricSegment>> {
    let result = fetch_json::<SearchResponse>(http.get(API).query(&[
        ("track", query.name.clone()),
        ("artist", query.artist.clone()),
        ("album", query.album.clone()),
        ("duration", (query.duration_ms / 1000).to_string()),
    ]))
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

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes().find(|attribute| attribute.name() == name).map(|attribute| attribute.value())
}

fn parse_ttml(source: &str) -> Vec<LyricSegment> {
    let Ok(document) = Document::parse(source) else { return Vec::new() };
    let mut segments = Vec::new();
    for line in document.descendants().filter(|node| node.tag_name().name() == "p") {
        let first = segments.len();
        let mut line_text = String::new();
        for node in line.descendants().skip(1) {
            let roles = || {
                node.ancestors().take_while(|ancestor| *ancestor != line).filter_map(|ancestor| {
                    (ancestor.tag_name().name() == "span").then(|| attribute(ancestor, "role")).flatten()
                })
            };
            if roles().any(|role| matches!(role, "x-translation" | "x-roman")) {
                continue;
            }
            if node.tag_name().name() == "span"
                && let Some(start_ms) = attribute(node, "begin").and_then(time)
            {
                segments.push(LyricSegment {
                    start_ms,
                    end_ms: attribute(node, "end").and_then(time).unwrap_or(start_ms + 1_000.0),
                    text: String::new(),
                    background: roles().any(|role| role == "x-bg"),
                    break_after: false,
                });
            } else if node.is_text() {
                let value = node.text().unwrap_or_default();
                line_text.push_str(value);
                if let Some(segment) = segments[first..].last_mut() {
                    if !value.chars().all(char::is_whitespace) {
                        segment.text.push_str(value);
                    } else if !segment.text.ends_with(char::is_whitespace) {
                        segment.text.push(' ');
                    }
                }
            }
        }
        if segments.len() == first
            && !line_text.trim().is_empty()
            && let Some((start_ms, end_ms)) =
                attribute(line, "begin").and_then(time).zip(attribute(line, "end").and_then(time))
        {
            segments.push(LyricSegment { start_ms, end_ms, text: line_text, background: false, break_after: false });
        }
        if let Some(segment) = segments[first..].last_mut() {
            segment.break_after = true;
        }
    }
    segments
}
