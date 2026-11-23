use super::{LyricLine, LyricSegment, Lyrics, Timing, fragment_timing, separate};
use crate::music::{
    MusicResult,
    spotify::proto::lyrics::{Line, SpotifyLyrics},
};
use core::ops::Range;

fn spotify_fragments(line: &Line, time: &Range<f32>) -> Option<Vec<LyricSegment>> {
    // Spotify counts UTF-16 code units in the original line, including spaces.
    let mut remaining = line.words.as_str();
    let mut parts: Vec<LyricSegment> = Vec::new();
    for part in &line.sync_data {
        let count = usize::try_from(part.num_chars).ok()?;
        let mut units = 0;
        let mut bytes = 0;
        for ch in remaining.chars() {
            if units >= count {
                break;
            }
            units += ch.len_utf16();
            bytes += ch.len_utf8();
        }
        if units != count {
            return None;
        }
        let (text, rest) = remaining.split_at(bytes);
        remaining = rest;
        if text.trim().is_empty() {
            if let Some(previous) = parts.last_mut() {
                previous.text.push_str(text);
            }
            continue;
        }
        let start = part.start_time_ms as f32;
        let end = part.end_time_ms as f32;
        if !(time.start..end).contains(&start)
            || !(start..=time.end).contains(&end)
            || parts.last().is_some_and(|previous| previous.time.start > start)
        {
            return None;
        }
        let trimmed = text.trim_start();
        if let Some(previous) = parts.last_mut() {
            previous.text.push_str(text.get(..text.len() - trimmed.len())?);
        }
        parts.push(LyricSegment { time: start..end, text: trimmed.into() });
    }
    if parts.is_empty() || !remaining.trim().is_empty() {
        return None;
    }
    separate(&mut parts);
    Some(parts)
}

pub(super) fn parse_spotify(response: Option<SpotifyLyrics>, duration: u32) -> MusicResult<Lyrics> {
    let Some(sheet) = response.and_then(|response| response.lyrics) else { return Ok(Lyrics::default()) };
    // V3 sync type 1 is line-synced and type 2 carries fragment timings.
    if !matches!(sheet.sync_type, 1 | 2) {
        return Ok(Lyrics::default());
    }
    let lines = sheet.lines;
    let mut output = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let text = line.words.trim();
        if text.is_empty() || matches!(text, "♪" | "♫") {
            continue;
        }
        let start = line.start_time_ms as f32;
        let end = if line.end_time_ms > line.start_time_ms {
            line.end_time_ms as f32
        } else {
            lines.get(index + 1).map_or(duration as f32, |next| next.start_time_ms as f32)
        };
        if !(0.0..end).contains(&start) || !(0.0..=duration as f32 + 1000.0).contains(&end) {
            return Err("Spotify line has invalid timing".into());
        }
        let time = start..end;
        let words = spotify_fragments(line, &time);
        output.push(LyricLine { text: format!("{text} "), time, words, lane: 0, singer: Some(0) });
    }
    let mut lyrics = Lyrics { lines: output, timing: Timing::Line };
    if !lyrics.lines.is_empty() && lyrics.lines.iter().all(|line| line.words.is_some()) {
        lyrics.timing = fragment_timing(&lyrics.lines);
    } else {
        for line in &mut lyrics.lines {
            line.words = None;
        }
    }
    Ok(lyrics)
}
