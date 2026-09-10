use super::{MusicResult, TRACK_SPACING_MS, Track, enrichment::Fetch, spotify::Spotify};
use isthmus::glam::{FloatExt, Vec2, Vec4, vec2, vec4};
use isthmus_sdf::layout::{ShapedLine, TextCache};
use reqwest::{Client, StatusCode};
use roxmltree::{Document, Node};
use std::{collections::BTreeMap, iter::once, mem, ops::Range, sync::Arc};
use tracing::{info, warn};

pub struct LyricSegment {
    pub time: Range<f32>,
    pub text: String,
    pub channel: usize,
}

#[derive(Default)]
pub struct Lyrics {
    pending: Vec<LyricSegment>,
    pub channels: BTreeMap<usize, Channel>,
    scroll: Vec<Vec2>,
}

#[derive(Default)]
pub struct Channel {
    pub runs: Vec<Run>,
    pub gaps: Vec<Vec4>,
}

pub struct Run {
    pub time: Range<f32>,
    pub x: Range<f32>,
    pub width: f32,
    pub line: Arc<ShapedLine>,
    pub scale: f32,
}

impl Run {
    // Slide inside the shared cell so the sung fraction stays exactly under the playhead.
    pub fn left(&self, time: f32, cursor: f32) -> f32 {
        let sung = ((time - self.time.start) / (self.time.end - self.time.start)).clamp(0.0, 1.0) * self.width;
        self.x.start + (cursor - self.x.start - sung).clamp(0.0, (self.x.end - self.x.start - self.width).max(0.0))
    }
}

impl Lyrics {
    const MERGE_WIDTH: f32 = 160.0;
    const SILENCE_SPEED: f32 = 0.035;

    pub(crate) fn prepare(&mut self, duration: f32, text: &mut TextCache) {
        let mut pending = mem::take(&mut self.pending);
        if pending.is_empty() {
            return;
        }
        pending.retain(|s| s.time.start.is_finite() && s.time.end.is_finite() && !s.text.trim().is_empty());
        for segment in &mut pending {
            segment.time.start = segment.time.start.clamp(0.0, duration);
        }
        pending.sort_by(|a, b| a.channel.cmp(&b.channel).then(a.time.start.total_cmp(&b.time.start)));
        pending.dedup_by(|b, a| {
            if a.channel != b.channel || !a.time.start.total_cmp(&b.time.start).is_eq() {
                return false;
            }
            a.text.push_str(&b.text);
            a.time.end = a.time.end.max(b.time.end);
            true
        });
        let space = text.shape(" ", 15.0, 700.0).text.width;
        let mut channels = mem::take(&mut self.channels);
        for group in pending.chunk_by(|a, b| a.channel == b.channel) {
            let channel = channels.entry(group[0].channel).or_default();
            for (index, segment) in group.iter().enumerate() {
                let next = group.get(index + 1);
                let value = segment.text.replace('🎵', "♪").replace('🎶', "♫");
                let line = text.shape(value.trim(), 15.0, 700.0);
                let bounds = line.text;
                let start = segment.time.start;
                let end = segment.time.end.clamp(start, next.map_or(duration, |s| s.time.start));
                if bounds.count == 0 || end <= start {
                    continue;
                }
                let scale = (1.0 + 0.25 * (0.06 * (end - start) / bounds.width.max(1.0) - 1.0)).clamp(0.9, 1.2);
                // Time the separator with the word so adjacent runs meet without a scrolling jump.
                let separated = value.ends_with(char::is_whitespace)
                    || next.is_some_and(|s| s.text.starts_with(char::is_whitespace));
                let width =
                    (bounds.width.max(bounds.max.x) - bounds.min.x.min(0.0)) * scale + space * f32::from(separated);
                channel.runs.push(Run { time: start..end, x: 0.0..0.0, width, line, scale });
            }
        }
        channels.retain(|_, channel| !channel.runs.is_empty());
        // A shared scroll speed reserves enough space for the fastest simultaneous voice.
        let mut times = vec![0.0, duration];
        times.extend(channels.values().flat_map(|c| &c.runs).flat_map(|run| [run.time.start, run.time.end]));
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| a.total_cmp(b).is_eq());
        self.scroll = vec![Vec2::ZERO];
        let mut x = 0.0;
        for pair in times.windows(2) {
            let speed = channels
                .values()
                .filter_map(|channel| {
                    let runs = &channel.runs;
                    let next = runs.partition_point(|r| r.time.start <= pair[0]);
                    let run = runs[..next].last()?;
                    (run.time.end > pair[0]).then_some(run.width / (run.time.end - run.time.start))
                })
                .fold(Self::SILENCE_SPEED, f32::max);
            x += (pair[1] - pair[0]) * speed;
            self.scroll.push(vec2(pair[1], x));
        }
        // Bound the slope even when several vacant rows merge at once.
        let bend = Self::MERGE_WIDTH * channels.len().saturating_sub(1).max(1) as f32;
        let mut later: Vec<Range<f32>> = Vec::new();
        for channel in channels.values_mut().rev() {
            let runs = &mut channel.runs;
            for run in runs.iter_mut() {
                run.x = self.position(run.time.start, duration)..self.position(run.time.end, duration);
            }
            let ends = once(0.0).chain(runs.iter().map(|r| r.time.end));
            let starts = runs.iter().map(|r| r.time.start).chain(once(duration));
            for (end, start) in ends.zip(starts) {
                if start - end <= 5_000.0 {
                    continue;
                }
                let a = if end == 0.0 { -bend } else { self.position(end, duration) };
                let b = self.position(start, duration) + if start >= duration { bend } else { 0.0 };
                if b - a < 2.0 * bend {
                    continue;
                }
                // Complete handovers in whitespace; bend text only across actual overlaps.
                let first = later.iter().filter(|r| r.end > end).map(|r| r.start).fold(duration, f32::min);
                let last = later.iter().filter(|r| r.start < start).map(|r| r.end).fold(0.0, f32::max);
                let merge = if first >= end { (a + bend).min(self.position(first, duration)) } else { a + bend };
                let split = if last <= start { (b - bend).max(self.position(last, duration)) } else { b - bend };
                channel.gaps.push(vec4(a, merge.max(a + 0.001), split.min(b - 0.001), b));
            }
            later.extend(runs.iter().map(|r| r.time.clone()));
        }
        self.channels = channels;
    }

    pub fn position(&self, time: f32, duration: f32) -> f32 {
        let at = time.min(duration);
        let next = self.scroll.partition_point(|point| point.x <= at);
        let x = match (self.scroll[..next].last(), self.scroll.get(next)) {
            (Some(a), Some(b)) => a.y.lerp(b.y, (at - a.x) / (b.x - a.x)),
            (Some(a), None) => a.y,
            _ => at * Self::SILENCE_SPEED,
        };
        x + (time - duration).clamp(0.0, TRACK_SPACING_MS) * 96.0 / TRACK_SPACING_MS
    }
}

pub(super) async fn fetch(track: &Track, http: &Client, spotify: &Spotify) -> Fetch<Lyrics> {
    let mut provider = "Apple Music";
    let result: MusicResult<Vec<LyricSegment>> = async {
        let response = http
            .get("https://lyrics-api.binimum.org/")
            .query(&[
                ("track", track.name.as_str()),
                ("artist", track.artist.as_str()),
                ("album", track.album.as_str()),
                ("duration", &(track.duration_ms / 1000).to_string()),
            ])
            .send()
            .await?;
        if response.status() != StatusCode::NOT_FOUND {
            let response = response.error_for_status()?.json::<serde_json::Value>().await?;
            let results = response["results"].as_array().ok_or("Lyrics search response has no results array")?;
            if let Some(result) = results.iter().find(|result| result["timing_type"] == "word") {
                let url = result["lyricsUrl"].as_str().ok_or("Lyrics search result has no URL")?;
                let source = http.get(url).send().await?.error_for_status()?.text().await?;
                let segments = parse_ttml(&source);
                if segments.is_empty() {
                    return Err("Lyrics document has no usable timed text".into());
                }
                return Ok(segments);
            }
        }
        if let Some(id) = track.id {
            provider = "Spotify";
            return spotify.lyrics(id).await;
        }
        Ok(Vec::new())
    }
    .await;

    match result {
        Ok(pending) => {
            if !pending.is_empty() {
                info!("Lyrics from {provider} for \"{}\" by {}", track.name, track.artist);
            }
            Fetch::Ready(Lyrics {
                channels: pending.iter().map(|s| (s.channel, Channel::default())).collect(),
                pending,
                ..Default::default()
            })
        }
        Err(error) => {
            warn!("Failed to fetch lyrics from {provider} for \"{}\" by {}: {error}", track.name, track.artist);
            Fetch::retry()
        }
    }
}

fn time(value: &str) -> Option<f32> {
    let value = value.strip_suffix('s').unwrap_or(value);
    value.split(':').try_fold(0.0, |ms, part| Some(ms * 60.0 + part.parse::<f32>().ok()? * 1000.0))
}

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes().find(|attribute| attribute.name() == name).map(|attribute| attribute.value())
}

fn separate(segments: &mut [LyricSegment], channel: usize) {
    if let Some(word) = segments.iter_mut().rev().find(|s| s.channel == channel)
        && !word.text.ends_with(char::is_whitespace)
    {
        word.text.push(' ');
    }
}

fn parse_ttml(source: &str) -> Vec<LyricSegment> {
    let Ok(document) = Document::parse(source) else { return Vec::new() };
    let mut agents: Vec<_> =
        document.descendants().filter(|n| n.tag_name().name() == "agent").filter_map(|n| attribute(n, "id")).collect();
    let mut segments: Vec<LyricSegment> = Vec::new();
    for line in document.descendants().filter(|node| node.tag_name().name() == "p") {
        let first = segments.len();
        for node in line.descendants().filter(Node::is_text) {
            let ancestors = node.ancestors().take_while(|n| Some(*n) != line.parent());
            let roles = || node.ancestors().filter_map(|n| attribute(n, "role"));
            if roles().any(|role| matches!(role, "x-translation" | "x-roman")) {
                continue;
            }
            let agent = node
                .ancestors()
                .find_map(|n| attribute(n, "agent"))
                .unwrap_or_else(|| agents.first().copied().unwrap_or(""));
            let singer = agents.iter().position(|id| *id == agent).unwrap_or_else(|| {
                agents.push(agent);
                agents.len() - 1
            });
            // Each singer (including a group) owns a lead/backing pair of channel IDs.
            let channel = singer * 2 + usize::from(roles().any(|role| role == "x-bg"));
            let value = node.text().unwrap_or_default();
            if value.trim().is_empty() {
                separate(&mut segments[first..], channel);
                continue;
            }
            let Some(start) = ancestors.clone().find_map(|n| attribute(n, "begin").and_then(time)) else { continue };
            segments.push(LyricSegment {
                time: start..ancestors.clone().find_map(|n| attribute(n, "end").and_then(time)).unwrap_or(f32::MAX),
                text: value.into(),
                channel,
            });
        }
        for channel in 0..agents.len() * 2 {
            separate(&mut segments[first..], channel);
        }
    }
    segments
}
