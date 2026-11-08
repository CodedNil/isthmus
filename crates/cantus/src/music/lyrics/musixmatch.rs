use super::{
    LyricLine, LyricSegment, Lyrics, Timing, fragment_timing, items, matches_track, normalize, prefer_lyrics, separate,
};
use crate::{
    app::fetch_json,
    music::{MusicResult, Track},
};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use web_time::Instant;

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
    pub(super) fn is_backing_off(&self) -> bool {
        self.retry_after.is_some_and(|time| Instant::now() < time)
    }

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
            self.backoff();
        }
        result
    }

    pub(super) async fn fetch(&mut self, track: &Track, http: &Client) -> MusicResult<Lyrics> {
        if self.token.is_empty() {
            self.backoff();
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
                ("part", "track_performer_tagging"),
            ])
            .await?;
        let mut calls = response["macro_calls"].take();
        let matched = self.body(calls["matcher.track.get"].take())?["track"].take();
        if !matches_track(track, &matched, "track_length")
            || flag(&matched["restricted"])
            || flag(&matched["instrumental"])
            || flag(&calls["track.lyrics.get"]["message"]["body"]["lyrics"]["restricted"])
        {
            return Ok(Lyrics::default());
        }
        let subtitles =
            self.body(calls["track.subtitles.get"].take()).and_then(|body| parse_subtitles(&body, track.duration_ms));
        // An unavailable or malformed richsync must not discard usable line timing.
        let words = async {
            let richsync = self.body(calls["track.richsync.get"].take())?["richsync"].take();
            if richsync.is_null()
                || flag(&richsync["restricted"])
                || richsync["richsync_length"]
                    .as_f64()
                    .is_some_and(|length| (length - f64::from(track.duration_ms) / 1000.0).abs() > 5.0)
            {
                return Ok(Lyrics::default());
            }
            let lines: Vec<RichsyncLine> =
                serde_json::from_str(richsync["richsync_body"].as_str().ok_or("Musixmatch richsync body missing")?)?;
            parse_richsync(&lines, &matched, track.duration_ms as f32)
        }
        .await;
        prefer_lyrics(subtitles, words)
    }

    fn body(&mut self, mut response: Value) -> MusicResult<Value> {
        match response["message"]["header"]["status_code"].as_u64() {
            Some(200) => Ok(response["message"]["body"].take()),
            Some(404) => Ok(Value::Null),
            None if response.is_null() => Ok(response),
            Some(code) => {
                self.backoff();
                Err(format!("Musixmatch API status {code}").into())
            }
            None => Err("Musixmatch response has no status code".into()),
        }
    }

    fn backoff(&mut self) {
        self.token.clear();
        self.retry_after = Some(Instant::now() + BACKOFF);
    }
}

fn parse_subtitles(body: &Value, duration: u32) -> MusicResult<Lyrics> {
    #[derive(Deserialize)]
    struct SubtitleLine {
        text: String,
        time: SubtitleTime,
    }
    #[derive(Deserialize)]
    struct SubtitleTime {
        total: f32,
    }
    for entry in items(&body["subtitle_list"]) {
        let subtitle = &entry["subtitle"];
        if flag(&subtitle["restricted"])
            || subtitle["subtitle_length"].as_f64().is_some_and(|length| {
                duration > 0 && length > 0.0 && (length - f64::from(duration) / 1000.0).abs() > 5.0
            })
        {
            continue;
        }
        let Some(source) = subtitle["subtitle_body"].as_str() else { continue };
        let lines: Vec<SubtitleLine> = serde_json::from_str(source)?;
        let mut segments = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if line.text.trim().is_empty() {
                continue;
            }
            let start = line.time.total * 1000.0;
            let end = lines.get(index + 1).map_or(duration as f32, |line| line.time.total * 1000.0);
            if !start.is_finite()
                || !end.is_finite()
                || start < 0.0
                || end <= start
                || (duration > 0 && end > duration as f32 + 1000.0)
            {
                return Err("Musixmatch subtitle has invalid timing".into());
            }
            segments.push(LyricSegment { time: start..end, text: format!("{} ", line.text.trim()) });
        }
        if !segments.is_empty() {
            return Ok(Lyrics::from_segments(segments, Timing::Line));
        }
    }
    Ok(Lyrics::default())
}

fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or_else(|| value.as_u64().is_some_and(|v| v != 0))
}

fn snippet_lines(value: &Value) -> impl Iterator<Item = String> + '_ {
    value.as_str().unwrap_or_default().lines().map(normalize).filter(|text| !text.is_empty())
}

#[derive(Deserialize)]
struct RichsyncLine {
    #[serde(rename = "ts")]
    start: f32,
    #[serde(rename = "te")]
    end: f32,
    #[serde(rename = "l")]
    chunks: Vec<RichsyncChunk>,
    #[serde(rename = "x")]
    text: String,
}

#[derive(Deserialize)]
struct RichsyncChunk {
    #[serde(rename = "c")]
    text: String,
    #[serde(rename = "o")]
    offset: f32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Voice {
    singer: Option<usize> = Some(0),
    backing: bool,
}

fn performers(lines: &[RichsyncLine], matched: &Value) -> Vec<Voice> {
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
        tagged.len() == lines.len() && tagged.iter().zip(lines).all(|((text, _), line)| *text == normalize(&line.text));
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if aligned {
                return tagged[index].1;
            }
            // Different line breaks/editions may still have unambiguous singer annotations.
            let text = normalize(&line.text);
            let mut candidates = tagged.iter().filter(|(snippet, _)| *snippet == text).map(|(_, voice)| *voice);
            let first = candidates.next().unwrap_or_default();
            if candidates.all(|voice| voice == first) { first } else { Voice::default() }
        })
        .collect()
}

fn parse_richsync(lines: &[RichsyncLine], matched: &Value, duration: f32) -> MusicResult<Lyrics> {
    let mut lane_slots: BTreeMap<Voice, Vec<(usize, f32)>> = BTreeMap::new();
    let mut next_lane = 0;
    let mut output = Vec::new();
    for (index, (line, voice)) in lines.iter().zip(performers(lines, matched)).enumerate() {
        if !(0.0..line.end).contains(&line.start) || !(0.0..=duration / 1000.0 + 1.0).contains(&line.end) {
            return Err("Musixmatch line has invalid timing".into());
        }
        // Offsets can invert by a rounding millisecond, so order them rather than rejecting the line.
        if line.chunks.iter().any(|c| !(-0.05..=line.end - line.start + 0.05).contains(&c.offset)) {
            return Err("Musixmatch word has invalid timing".into());
        }
        if normalize(&line.chunks.iter().map(|c| c.text.as_str()).collect::<String>()) != normalize(&line.text) {
            return Err("Musixmatch word timing does not cover the line text".into());
        }
        let mut cursor = 0.0;
        let mut segments: Vec<LyricSegment> = Vec::new();
        for chunk in &line.chunks {
            cursor = chunk.offset.max(cursor);
            if chunk.text.trim().is_empty() {
                separate(&mut segments);
                continue;
            }
            let start = (line.start + cursor) * 1000.0;
            if let Some(previous) = segments.last_mut() {
                // Space offsets are not vocal boundaries: hold until the next sung chunk.
                previous.time.end = start;
            }
            segments.push(LyricSegment { time: start..line.end * 1000.0, text: chunk.text.clone() });
        }
        // Some editions place the final word at the line's end, leaving its end unspecified.
        if let Some(last) = segments.last_mut().filter(|last| last.time.end <= last.time.start) {
            last.time.end = lines[index + 1..]
                .iter()
                .map(|line| line.start * 1000.0)
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
        let rows = lane_slots.entry(voice).or_default();
        let lane = if let Some((lane, end)) = rows.iter_mut().find(|(_, end)| *end <= segments[0].time.start) {
            *end = segments.last().map_or(*end, |last| last.time.end);
            *lane
        } else {
            let lane = next_lane;
            next_lane += 1;
            rows.push((lane, segments.last().map_or(0.0, |last| last.time.end)));
            lane
        };
        let text = segments.iter().map(|word| word.text.as_str()).collect();
        output.push(LyricLine {
            text,
            time: segments.first().unwrap().time.start..segments.last().unwrap().time.end,
            words: Some(segments),
            lane,
            singer: voice.singer,
        });
    }
    let mut lyrics = Lyrics { lines: output, timing: Timing::Word };
    lyrics.timing = fragment_timing(&lyrics.lines);
    Ok(lyrics)
}
