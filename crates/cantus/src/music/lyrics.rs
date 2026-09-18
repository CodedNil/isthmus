use super::{MusicResult, Track};
use crate::{app::fetch_json, platform::captions};
use reqwest::{Client, StatusCode};
use roxmltree::{Document, Node};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    time::Duration,
};
use tokio::sync::Mutex;
use tracing::{info, warn};
use web_time::Instant;

pub struct LyricSegment {
    pub time: Range<f32>,
    pub text: String,
}

#[derive(Default)]
pub struct Lyrics {
    pub channels: Vec<Channel>,
    pub sections: Vec<LyricSegment>,
    pub(crate) timing: &'static str = "word",
}

#[derive(Default)]
pub struct Channel {
    pub singer: Option<usize> = Some(0),
    pub segments: Vec<LyricSegment>,
}

pub(super) async fn fetch(track: &Track, http: &Client, session: &Mutex<Session>) -> MusicResult<Lyrics> {
    if track.uri.starts_with("http://") || track.uri.starts_with("https://") {
        match captions(track.uri.clone()).await {
            Ok(file) if !file.source.is_empty() => {
                let lyrics = match file.format.as_str() {
                    "json3" => parse_json3(&file.source),
                    "srv3" | "ttml" => parse_ttml(&file.source),
                    // SubRip is WebVTT without a header, and both use `-->` cue timing.
                    "vtt" | "srt" => parse_webvtt(&file.source),
                    _ => Lyrics::default(),
                };
                if !lyrics.channels.is_empty() {
                    return Ok(Lyrics { timing: "caption", ..lyrics });
                }
            }
            Err(error) => tracing::debug!(%error, url = %track.uri, "caption extraction unavailable"),
            _ => {}
        }
    }
    let mut session = session.lock().await;
    let mut failure = None;
    let title = track.compact_title().split(" (").next().unwrap_or(&track.name);
    let artist = track.primary_artist();
    for source in ["MXM", "APL"] {
        let result = if source == "MXM" {
            if cfg!(target_arch = "wasm32") || session.retry_after.is_some_and(|time| Instant::now() < time) {
                continue;
            }
            session.fetch(track, http).await
        } else {
            fetch_apple_lyrics(track, http).await
        };
        match result {
            Ok(lyrics) if !lyrics.channels.is_empty() => {
                let timing = lyrics.timing;
                let segments: usize = lyrics.channels.iter().map(|c| c.segments.len()).sum();
                let lanes = lyrics.channels.len();
                let sections = lyrics.sections.len();
                info!(
                    "{source} ({timing}): {title:.28} — {artist:.20} · {segments} seg · {lanes} lanes · {sections} sections"
                );
                return Ok(lyrics);
            }
            Ok(_) => {}
            Err(error) => {
                warn!("{source}: {title:?} — {artist} · {error}");
                failure = Some(error);
            }
        }
    }
    failure.map_or_else(|| Ok(Lyrics::default()), Err)
}

// JSON3 contains each phrase once, unlike rolling WebVTT captions.
fn parse_json3(source: &str) -> Lyrics {
    #[derive(Deserialize)]
    struct Captions {
        events: Vec<Event>,
    }
    #[derive(Deserialize)]
    struct Event {
        #[serde(rename = "tStartMs")]
        start: f32,
        #[serde(rename = "dDurationMs", default)]
        duration: f32,
        #[serde(default)]
        segs: Vec<Part>,
    }
    #[derive(Deserialize)]
    struct Part {
        utf8: String,
        #[serde(rename = "tOffsetMs", default)]
        offset: f32,
    }
    let Ok(captions) = serde_json::from_str::<Captions>(source) else { return Lyrics::default() };
    let mut segments = Vec::<LyricSegment>::new();
    for event in captions.events {
        if event.segs.iter().all(|part| part.utf8.trim().is_empty()) {
            continue; // Window definitions and line breaks carry no speech.
        }
        if let Some(previous) = segments.last_mut() {
            previous.time.end = previous.time.end.min(event.start);
        }
        let mut annotation = false;
        let mut text = String::new();
        let mut start = event.start;
        for part in event.segs {
            let spoken: String = part
                .utf8
                .chars()
                .filter_map(|ch| match ch {
                    '[' => {
                        annotation = true;
                        None
                    }
                    ']' => {
                        annotation = false;
                        Some(' ')
                    }
                    _ => (!annotation).then_some(ch),
                })
                .collect();
            if text.trim().is_empty() && !spoken.trim().is_empty() {
                start = event.start + part.offset;
            }
            text.push_str(&spoken);
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        segments.push(LyricSegment { time: start..event.start + event.duration, text: format!("{text} ") });
    }
    segments.retain(|segment| !segment.text.trim().is_empty() && segment.time.end > segment.time.start);
    let channels = if segments.is_empty() { Vec::new() } else { vec![Channel { segments, ..Default::default() }] };
    Lyrics { channels, ..Default::default() }
}

fn caption_time(value: &str) -> Option<f32> {
    let value = value.trim().replace(',', ".");
    let mut parts = value.split(':');
    let first = parts.next()?.parse::<f32>().ok()?;
    let second = parts.next()?.parse::<f32>().ok()?;
    let third = parts.next().and_then(|part| part.parse::<f32>().ok());
    Some(third.map_or(first * 60.0 + second, |third| first * 3600.0 + second * 60.0 + third))
}

fn clean_caption(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut tag = false;
    for ch in text.chars() {
        match ch {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => result.push(ch),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_webvtt(source: &str) -> Lyrics {
    let mut segments = Vec::new();
    for block in source.replace("\r\n", "\n").split("\n\n") {
        let mut lines = block.lines();
        let timing = lines.find(|line| line.contains("-->"));
        let Some(timing) = timing else { continue };
        let mut times = timing.split("-->");
        let Some(start) = times.next().and_then(caption_time) else { continue };
        let Some(end) =
            times.next().and_then(|value| caption_time(value.split_whitespace().next().unwrap_or_default()))
        else {
            continue;
        };
        let text = clean_caption(&lines.collect::<Vec<_>>().join(" "));
        if !text.is_empty() && end > start {
            segments.push(LyricSegment { time: start..end, text: format!("{text} ") });
        }
    }
    let channels = if segments.is_empty() { Vec::new() } else { vec![Channel { segments, ..Default::default() }] };
    Lyrics { channels, ..Default::default() }
}

async fn fetch_apple_lyrics(track: &Track, http: &Client) -> MusicResult<Lyrics> {
    let response = http
        .get("https://lyrics-api.binimum.org/")
        .query(&[
            ("track", track.name.as_str()),
            ("artist", track.primary_artist()),
            ("album", track.album.as_str()),
            ("duration", &(track.duration_ms / 1000).to_string()),
        ])
        .send()
        .await?;
    if response.status() != StatusCode::NOT_FOUND {
        let response = response.error_for_status()?.json::<Value>().await?;
        let results = response["results"].as_array().ok_or("Lyrics search response has no results array")?;
        let preferred = ["word", "line"].into_iter().find_map(|timing| {
            results.iter().find(|result| result["timing_type"] == timing).map(|result| (result, timing))
        });
        if let Some((result, timing)) = preferred {
            let url = result["lyricsUrl"].as_str().ok_or("Lyrics search result has no URL")?;
            let source = http.get(url).send().await?.error_for_status()?.text().await?;
            let mut lyrics = parse_ttml(&source);
            if lyrics.channels.is_empty() {
                return Err("Lyrics document has no usable timed text".into());
            }
            lyrics.timing = timing;
            return Ok(lyrics);
        }
    }
    Ok(Lyrics::default())
}

fn time(value: &str) -> Option<f32> {
    let value = value.strip_suffix('s').unwrap_or(value);
    value.split(':').try_fold(0.0, |ms, part| Some(ms * 60.0 + part.parse::<f32>().ok()? * 1000.0))
}

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes().find(|attribute| attribute.name() == name).map(|attribute| attribute.value())
}

fn separate(segments: &mut [LyricSegment]) {
    if let Some(word) = segments.last_mut()
        && !word.text.ends_with(char::is_whitespace)
    {
        word.text.push(' ');
    }
}

fn inherited<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.ancestors().find_map(|node| attribute(node, name))
}

fn section_label(name: &str) -> String {
    let mut label = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() && label.ends_with(char::is_lowercase) {
            label.push(' ');
        }
        if label.is_empty() {
            label.extend(ch.to_uppercase());
        } else {
            label.push(ch);
        }
    }
    label
}

fn parse_ttml(source: &str) -> Lyrics {
    let Ok(document) = Document::parse(source) else { return Lyrics::default() };
    let mut lyrics = Lyrics::default();
    let mut agents: Vec<_> = document
        .descendants()
        .filter(|n| n.tag_name().name() == "agent")
        .filter_map(|n| Some((attribute(n, "id")?, attribute(n, "type") == Some("group"))))
        .collect();
    let mut channels = BTreeMap::new();
    for node in document.descendants().filter(Node::is_element) {
        if node.tag_name().name() == "div" {
            let name = attribute(node, "songPart").or_else(|| attribute(node, "song-part"));
            let timing = attribute(node, "begin").and_then(time).zip(attribute(node, "end").and_then(time));
            if let (Some(name), Some((start, end))) = (name, timing)
                && start.is_finite()
                && end.is_finite()
                && end > start
            {
                lyrics.sections.push(LyricSegment { text: section_label(name), time: start..end });
            }
        }
        if node.tag_name().name() != "p" {
            continue;
        }
        for word in node.descendants().filter(Node::is_text) {
            let roles: Vec<_> =
                word.ancestors().filter_map(|n| attribute(n, "role")).flat_map(str::split_whitespace).collect();
            if roles.contains(&"x-translation") || roles.contains(&"x-roman") {
                continue;
            }
            let agent = inherited(word, "agent").unwrap_or_else(|| agents.first().map_or("", |a| a.0));
            let index = agents.iter().position(|a| a.0 == agent).unwrap_or_else(|| {
                agents.push((agent, false));
                agents.len() - 1
            });
            let group = agents[index].1;
            let backing = roles.contains(&"x-bg");
            let channel = channels.entry((!group, index, backing)).or_insert_with(|| Channel {
                singer: (!group).then(|| agents[..index].iter().filter(|a| !a.1).count()),
                ..Default::default()
            });
            let value = word.text().unwrap_or_default();
            if value.trim().is_empty() {
                separate(&mut channel.segments);
            } else if let Some(start) = inherited(word, "begin").and_then(time) {
                channel.segments.push(LyricSegment {
                    time: start..inherited(word, "end").and_then(time).unwrap_or(f32::MAX),
                    text: value.into(),
                });
            }
        }
        for channel in channels.values_mut() {
            separate(&mut channel.segments);
        }
    }
    lyrics.sections.sort_by(|a, b| a.time.start.total_cmp(&b.time.start));
    lyrics.channels = channels.into_values().filter(|c| !c.segments.is_empty()).collect();
    lyrics
}
const API: &str = "https://apic-appmobile.musixmatch.com/ws/1.1/";
const APP_ID: &str = "mac-ios-v2.0";
const BACKOFF: Duration = Duration::from_mins(5);

// Shared by lyric requests to reuse authentication and rate-limit backoff.
#[derive(Default)]
pub struct Session {
    token: String,
    retry_after: Option<Instant>,
}

impl Session {
    async fn request(&mut self, http: &Client, endpoint: &str, params: &[(&str, &str)]) -> MusicResult<Value> {
        let response = fetch_json::<Value>(
            http.get(format!("{API}{endpoint}"))
                .query(&[("app_id", APP_ID), ("format", "json")])
                .query(&[("usertoken", self.token.as_str())])
                .query(params)
                .header("Cookie", "x-mxm-token-guid=")
                .header("x-mxm-app-version", "10.1.1")
                .header("X-User-Agent", "Musixmatch/2025120901 CFNetwork/3860.300.31 Darwin/25.2.0")
                .timeout(Duration::from_secs(5)),
        )
        .await;
        let result = match response {
            Ok(response) => self.body(response),
            Err(error) if error.status() == Some(StatusCode::NOT_FOUND) => Ok(Value::Null),
            // Never log the request URL: it contains the anonymous token.
            Err(error) => Err(error.without_url().into()),
        };
        if result.is_err() {
            self.token.clear();
            self.retry_after = Some(Instant::now() + BACKOFF);
        }
        result
    }

    async fn fetch(&mut self, track: &Track, http: &Client) -> MusicResult<Lyrics> {
        if self.token.is_empty() {
            self.retry_after = Some(Instant::now() + BACKOFF);
            let response = self.request(http, "token.get", &[]).await?;
            response["user_token"]
                .as_str()
                .filter(|s| !s.is_empty() && !s.starts_with("UpgradeOnly"))
                .ok_or("Musixmatch token missing")?
                .clone_into(&mut self.token);
            self.retry_after = None;
        }
        let duration = (track.duration_ms / 1000).to_string();
        let spotify_uri = track.id.map(|id| format!("spotify:track:{id}")).unwrap_or_default();
        let mut response = self
            .request(http, "macro.subtitles.get", &[
                ("q_track", &track.name),
                ("q_artist", track.primary_artist()),
                ("q_album", &track.album),
                ("q_duration", &duration),
                ("f_subtitle_length", &duration),
                ("track_spotify_id", &spotify_uri),
                ("namespace", "lyrics_richsynched"),
                ("subtitle_format", "mxm"),
                ("optional_calls", "track.richsync"),
                ("richsync_compact_type", "words"),
                ("part", "track_structure,track_performer_tagging"),
            ])
            .await?;
        let mut calls = response["macro_calls"].take();
        let matched = self.body(calls["matcher.track.get"].take())?["track"].take();
        if !matches_track(track, &matched) || flag(&matched["restricted"]) || flag(&matched["instrumental"]) {
            return Ok(Lyrics::default());
        }
        if flag(&calls["track.lyrics.get"]["message"]["body"]["lyrics"]["restricted"]) {
            return Ok(Lyrics::default());
        }
        // The current mobile client bundles richsync; avoid another lookup when no word timing exists.
        let richsync = self.body(calls["track.richsync.get"].take())?["richsync"].take();
        if richsync.is_null()
            || flag(&richsync["restricted"])
            || richsync["richsync_length"]
                .as_f64()
                .is_some_and(|length| (length - f64::from(track.duration_ms) / 1000.0).abs() > 5.0)
        {
            return Ok(Lyrics::default());
        }
        let lines: Vec<Line> =
            serde_json::from_str(richsync["richsync_body"].as_str().ok_or("Musixmatch richsync body missing")?)?;
        let mut lyrics = parse_richsync(&lines, &matched, track.duration_ms as f32)?;
        if lyrics.channels.is_empty() {
            return Ok(lyrics);
        }
        if flag(&matched["has_track_structure"])
            && let Some(id) = matched["commontrack_id"].as_u64()
        {
            // Structure is optional: an unavailable annotation must not discard valid word timing.
            if let Ok(metadata) =
                self.request(http, "crowd.track.metadata.get", &[("commontrack_id", &id.to_string())]).await
            {
                lyrics.sections = sections(&lines, &metadata);
            }
        }
        Ok(lyrics)
    }

    fn body(&mut self, mut response: Value) -> MusicResult<Value> {
        match response["message"]["header"]["status_code"].as_u64() {
            Some(200) => Ok(response["message"]["body"].take()),
            Some(404) => Ok(Value::Null),
            None if response.is_null() => Ok(response),
            Some(code) => {
                self.token.clear();
                self.retry_after = Some(Instant::now() + BACKOFF);
                Err(format!("Musixmatch API status {code}").into())
            }
            None => Err("Musixmatch response has no status code".into()),
        }
    }
}

fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or_else(|| value.as_u64().is_some_and(|v| v != 0))
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn snippet_lines(value: &Value) -> impl Iterator<Item = String> + '_ {
    value.as_str().unwrap_or_default().lines().map(normalize).filter(|text| !text.is_empty())
}

fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn matches_track(track: &Track, matched: &Value) -> bool {
    let title = normalize(matched["track_name"].as_str().unwrap_or_default());
    let artist = normalize(matched["artist_name"].as_str().unwrap_or_default());
    !title.is_empty()
        && !artist.is_empty()
        && (title == normalize(&track.name) || title == normalize(track.compact_title()))
        && track.artists.iter().any(|name| normalize(name) == artist)
        && matched["track_length"]
            .as_f64()
            .is_none_or(|length| length == 0.0 || (length - f64::from(track.duration_ms) / 1000.0).abs() <= 5.0)
}

#[derive(Deserialize)]
struct Line {
    ts: f32,
    te: f32,
    l: Vec<Chunk>,
    x: String,
}

#[derive(Deserialize)]
struct Chunk {
    c: String,
    o: f32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Voice {
    singer: Option<usize> = Some(0),
    backing: bool,
}

// Section snippets must match in sequence, so repeated choruses cannot shift their labels.
fn sections(lines: &[Line], metadata: &Value) -> Vec<LyricSegment> {
    let mut index = 0;
    let mut sections = Vec::new();
    for part in items(&metadata["metadata"]["track_structure"]) {
        let Some(description) = part["description"].as_str() else { return Vec::new() };
        let first = index;
        for text in snippet_lines(&part["snippet"]) {
            if lines.get(index).is_none_or(|line| normalize(&line.x) != text) {
                return Vec::new();
            }
            index += 1;
        }
        if index > first {
            sections.push(LyricSegment {
                time: lines[first].ts * 1000.0..lines[index - 1].te * 1000.0,
                text: section_label(description),
            });
        }
    }
    if index == lines.len() { sections } else { Vec::new() }
}

fn performers(lines: &[Line], matched: &Value) -> Vec<Voice> {
    let mut artists = BTreeMap::new();
    let mut tagged = Vec::new();
    for entry in items(&matched["performer_tagging"]["content"]) {
        let performers = items(&entry["performers"]);
        let has_role = |role: &str| performers.iter().any(|p| p["type"] == role);
        let singers: BTreeSet<_> = performers
            .iter()
            .filter(|p| p["type"] == "artist")
            .filter_map(|p| p["fqid"].as_str())
            .map(|id| {
                let next = artists.len();
                *artists.entry(id).or_insert(next)
            })
            .collect();
        let voice = Voice {
            singer: if has_role("fan_chant") || singers.len() > 1 {
                None
            } else {
                singers.first().copied().or(Some(0))
            },
            backing: has_role("backing_vocalist"),
        };
        tagged.extend(snippet_lines(&entry["snippet"]).map(|text| (text, voice)));
    }
    let aligned =
        tagged.len() == lines.len() && tagged.iter().zip(lines).all(|((text, _), line)| *text == normalize(&line.x));
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if aligned {
                return tagged[index].1;
            }
            // Different line breaks/editions may still have unambiguous singer annotations.
            let text = normalize(&line.x);
            let mut candidates = tagged.iter().filter(|(snippet, _)| *snippet == text).map(|(_, voice)| *voice);
            let first = candidates.next().unwrap_or_default();
            if candidates.all(|voice| voice == first) { first } else { Voice::default() }
        })
        .collect()
}

fn parse_richsync(lines: &[Line], matched: &Value, duration: f32) -> MusicResult<Lyrics> {
    let mut channels: BTreeMap<Voice, Vec<Channel>> = BTreeMap::new();
    for (index, (line, voice)) in lines.iter().zip(performers(lines, matched)).enumerate() {
        if !(0.0..line.te).contains(&line.ts) || !(0.0..=duration / 1000.0 + 1.0).contains(&line.te) {
            return Err("Musixmatch line has invalid timing".into());
        }
        // Offsets can invert by a rounding millisecond, so order them rather than rejecting the line.
        if line.l.iter().any(|c| !(-0.05..=line.te - line.ts + 0.05).contains(&c.o)) {
            return Err("Musixmatch word has invalid timing".into());
        }
        if normalize(&line.l.iter().map(|c| c.c.as_str()).collect::<String>()) != normalize(&line.x) {
            return Err("Musixmatch word timing does not cover the line text".into());
        }
        let mut cursor = 0.0;
        let mut segments: Vec<LyricSegment> = Vec::new();
        for chunk in &line.l {
            cursor = chunk.o.max(cursor);
            if chunk.c.trim().is_empty() {
                separate(&mut segments);
                continue;
            }
            let start = (line.ts + cursor) * 1000.0;
            if let Some(previous) = segments.last_mut() {
                // Space offsets are not vocal boundaries: hold until the next sung chunk.
                previous.time.end = start;
            }
            segments.push(LyricSegment { time: start..line.te * 1000.0, text: chunk.c.clone() });
        }
        // Some editions place the final word at the line's end, leaving its end unspecified.
        if let Some(last) = segments.last_mut().filter(|last| last.time.end <= last.time.start) {
            last.time.end = lines[index + 1..]
                .iter()
                .map(|line| line.ts * 1000.0)
                .filter(|&start| start > last.time.start)
                .fold(duration, f32::min);
            if last.time.end <= last.time.start {
                return Err("Musixmatch final word has no end boundary".into());
            }
        }
        separate(&mut segments);
        if segments.is_empty() {
            continue;
        }
        // Overlapping lines need separate lanes even when no vocal role was supplied.
        let rows = channels.entry(voice).or_default();
        let index = rows
            .iter()
            .position(|lane| lane.segments.last().is_none_or(|last| last.time.end <= segments[0].time.start))
            .unwrap_or_else(|| {
                rows.push(Channel { singer: voice.singer, ..Default::default() });
                rows.len() - 1
            });
        rows[index].segments.extend(segments);
    }
    Ok(Lyrics { channels: channels.into_values().flatten().collect(), ..Default::default() })
}
