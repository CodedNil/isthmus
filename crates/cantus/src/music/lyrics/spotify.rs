use super::{LyricLine, LyricSegment, Lyrics, Timing, fragment_timing, items, separate};
use crate::music::MusicResult;
use serde_json::Value;
use std::ops::Range;

fn milliseconds(value: &Value) -> Option<f32> {
    value.as_str()?.parse().ok()
}

fn spotify_fragments(line: &Value, time: &Range<f32>) -> Option<Vec<LyricSegment>> {
    let mut remaining = line["words"].as_str()?.trim();
    let mut parts: Vec<LyricSegment> = Vec::new();
    for part in items(&line["syllables"]) {
        let Some(text) = part["text"].as_str().map(str::trim).filter(|text| !text.is_empty()) else { continue };
        let prefix = remaining.len() - remaining.trim_start().len();
        if let Some(previous) = parts.last_mut() {
            previous.text.push_str(&remaining[..prefix]);
        }
        remaining = remaining.trim_start().strip_prefix(text)?;
        let start = milliseconds(&part["startTimeMs"])?;
        let end = milliseconds(&part["endTimeMs"])?;
        if !(time.start..end).contains(&start)
            || !(start..=time.end).contains(&end)
            || parts.last().is_some_and(|previous| previous.time.start > start)
        {
            return None;
        }
        parts.push(LyricSegment { time: start..end, text: text.into() });
    }
    if parts.is_empty() || !remaining.trim().is_empty() {
        return None;
    }
    separate(&mut parts);
    Some(parts)
}

pub(super) fn parse_spotify(response: &Value, duration: u32) -> MusicResult<Lyrics> {
    let sheet = &response["lyrics"];
    if !matches!(sheet["syncType"].as_str(), Some("LINE_SYNCED" | "WORD_SYNCED" | "SYLLABLE_SYNCED")) {
        return Ok(Lyrics::default());
    }
    let lines = items(&sheet["lines"]);
    let mut output = Vec::new();
    let mut all_word_synced = true;
    for (index, line) in lines.iter().enumerate() {
        let text = line["words"].as_str().unwrap_or_default().trim();
        if text.is_empty() || matches!(text, "♪" | "♫") {
            continue;
        }
        let start = milliseconds(&line["startTimeMs"]).ok_or("Spotify line has no start time")?;
        let end = milliseconds(&line["endTimeMs"])
            .filter(|&end| end > start)
            .or_else(|| lines.get(index + 1).and_then(|line| milliseconds(&line["startTimeMs"])))
            .unwrap_or(duration as f32);
        if !(0.0..end).contains(&start) || !(0.0..=duration as f32 + 1000.0).contains(&end) {
            return Err("Spotify line has invalid timing".into());
        }
        let words = spotify_fragments(line, &(start..end));
        if words.is_none() {
            all_word_synced = false;
        }
        output.push(LyricLine { text: format!("{text} "), time: start..end, words, lane: 0, singer: Some(0) });
    }
    let mut lyrics = Lyrics { lines: output, timing: Timing::Line, ..Default::default() };
    if all_word_synced && lyrics.lines.iter().any(|line| line.words.is_some()) {
        lyrics.timing = fragment_timing(&lyrics.lines);
    } else {
        for line in &mut lyrics.lines {
            line.words = None;
        }
    }
    Ok(lyrics)
}
