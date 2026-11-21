use super::{LyricLine, LyricSegment, Lyrics, Timing, fragment_timing, separate};
use crate::music::{
    MusicResult,
    spotify::lyrics::{Line, Milliseconds, SpotifyLyrics, SyncType},
};
use core::ops::Range;

fn spotify_fragments(line: &Line, time: &Range<f32>) -> Option<Vec<LyricSegment>> {
    let mut remaining = line.words.as_deref()?.trim();
    let mut parts: Vec<LyricSegment> = Vec::new();
    for part in line.syllables.as_deref().unwrap_or_default() {
        let Some(text) = part.text.as_deref().map(str::trim).filter(|text| !text.is_empty()) else { continue };
        let prefix = remaining.len() - remaining.trim_start().len();
        if let Some(previous) = parts.last_mut() {
            previous.text.push_str(remaining.get(..prefix).unwrap_or_default());
        }
        remaining = remaining.trim_start().strip_prefix(text)?;
        let start = part.start_time_ms.as_ref()?.value()?;
        let end = part.end_time_ms.as_ref()?.value()?;
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

pub(super) fn parse_spotify(response: Option<SpotifyLyrics>, duration: u32) -> MusicResult<Lyrics> {
    let Some(sheet) = response.and_then(|response| response.lyrics) else { return Ok(Lyrics::default()) };
    if !matches!(sheet.sync_type, Some(SyncType::LineSynced | SyncType::WordSynced | SyncType::SyllableSynced)) {
        return Ok(Lyrics::default());
    }
    let lines = sheet.lines.unwrap_or_default();
    let mut output = Vec::new();
    let mut all_word_synced = true;
    for (index, line) in lines.iter().enumerate() {
        let text = line.words.as_deref().unwrap_or_default().trim();
        if text.is_empty() || matches!(text, "♪" | "♫") {
            continue;
        }
        let start =
            line.start_time_ms.as_ref().and_then(Milliseconds::value).ok_or("Spotify line has no start time")?;
        let end = line
            .end_time_ms
            .as_ref()
            .and_then(Milliseconds::value)
            .filter(|&end| end > start)
            .or_else(|| lines.get(index + 1).and_then(|line| line.start_time_ms.as_ref().and_then(Milliseconds::value)))
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
    let mut lyrics = Lyrics { lines: output, timing: Timing::Line };
    if all_word_synced && lyrics.lines.iter().any(|line| line.words.is_some()) {
        lyrics.timing = fragment_timing(&lyrics.lines);
    } else {
        for line in &mut lyrics.lines {
            line.words = None;
        }
    }
    Ok(lyrics)
}
