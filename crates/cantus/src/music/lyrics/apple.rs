use super::{
    LyricLine, LyricSegment, Lyrics, Timing, fragment_timing, matches_track, prefer_lyrics, section_label, separate,
};
use crate::music::{MusicResult, Track};
use reqwest::{Client, StatusCode};
use roxmltree::{Document, Node};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) async fn fetch_apple_lyrics(track: &Track, http: &Client) -> MusicResult<Lyrics> {
    // lrc.red serves BiniLyrics search results and TTML.
    // This endpoint requires integer seconds, unlike LyricsPlus.
    let duration = (track.duration_ms / 1000).to_string();
    let mut request = http.get("https://lyrics-api.binimum.org/").query(&[
        ("track", track.name.as_str()),
        ("artist", track.primary_artist()),
        ("album", track.album.as_str()),
    ]);
    if track.duration_ms > 0 {
        request = request.query(&[("duration", duration.as_str())]);
    }
    if let Some(isrc) = track.isrc.as_deref() {
        request = request.query(&[("isrc", isrc)]);
    }
    let response = request.send().await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(Lyrics::default());
    }
    let response = response.error_for_status()?.json::<Value>().await?;
    let results = response["results"].as_array().ok_or("Apple lyrics response has no results array")?;
    let mut best = Ok(Lyrics::default());
    for timing in [Timing::Syllable, Timing::Word, Timing::Line] {
        for result in results.iter().filter(|result| {
            result["timing_type"].as_str() == Some(timing.as_ref()) && matches_apple_result(track, result)
        }) {
            let Some(url) = result["lyricsUrl"].as_str() else { continue };
            let fetched: MusicResult<Lyrics> = async {
                let source = http.get(url).send().await?.error_for_status()?.text().await?;
                let lyrics = parse_ttml(&source);
                if lyrics.is_empty() {
                    return Err("Lyrics document has no usable timed text".into());
                }
                Ok(lyrics)
            }
            .await;
            best = prefer_lyrics(best, fetched);
            if best.as_ref().is_ok_and(|lyrics| lyrics.timing == Timing::Syllable) {
                return best;
            }
        }
    }
    best
}

pub(super) fn matches_apple_result(track: &Track, matched: &Value) -> bool {
    matches_track(track, matched, "duration")
        && track.isrc.as_deref().zip(matched["isrc"].as_str()).is_none_or(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn time(value: &str) -> Option<f32> {
    let value = value.trim().replace(',', ".");
    let value = value.strip_suffix('s').unwrap_or(&value);
    value.split(':').try_fold(0.0, |ms, part| Some(ms * 60.0 + part.parse::<f32>().ok()? * 1000.0))
}

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes().find(|attribute| attribute.name() == name).map(|attribute| attribute.value())
}

fn inherited<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.ancestors().find_map(|node| attribute(node, name))
}

pub(super) fn parse_ttml(source: &str) -> Lyrics {
    let Ok(document) = Document::parse(source) else { return Lyrics::default() };
    let timing = if document
        .descendants()
        .any(|node| node.tag_name().name() == "span" && attribute(node, "begin").and_then(time).is_some())
    {
        Timing::Word
    } else {
        Timing::Line
    };
    let mut lyrics = Lyrics { timing, ..Default::default() };
    let mut agents: Vec<_> = document
        .descendants()
        .filter(|n| n.tag_name().name() == "agent")
        .filter_map(|n| Some((attribute(n, "id")?, attribute(n, "type") == Some("group"))))
        .collect();
    let mut lane_ids = BTreeMap::new();
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
        let mut line_channels: BTreeMap<(bool, usize, bool), Vec<LyricSegment>> = BTreeMap::new();
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
            let key = (!group, index, backing);
            if !lane_ids.contains_key(&key) {
                let lane = lane_ids.len();
                lane_ids.insert(key, lane);
            }
            let value = word.text().unwrap_or_default();
            if value.trim().is_empty() {
                separate(line_channels.entry(key).or_default());
            } else if let Some(start) = inherited(word, "begin").and_then(time) {
                line_channels.entry(key).or_default().push(LyricSegment {
                    time: start..inherited(word, "end").and_then(time).unwrap_or(f32::MAX),
                    text: value.into(),
                });
            }
        }
        for (key, mut words) in line_channels {
            separate(&mut words);
            if words.is_empty() {
                continue;
            }
            let lane = lane_ids[&key];
            let singer = (!key.0).then(|| agents[..key.1].iter().filter(|a| !a.1).count());
            let start = words.iter().map(|word| word.time.start).fold(f32::MAX, f32::min);
            let end = words.iter().map(|word| word.time.end).fold(start, f32::max);
            let text = words.iter().map(|word| word.text.as_str()).collect();
            let timed_words = (timing >= Timing::Word).then_some(words);
            lyrics.lines.push(LyricLine { text, time: start..end, words: timed_words, lane, singer });
        }
    }
    lyrics.sections.sort_by(|a, b| a.time.start.total_cmp(&b.time.start));
    if lyrics.timing == Timing::Word {
        lyrics.timing = fragment_timing(&lyrics.lines);
    }
    lyrics
}
