use super::{LyricLine, LyricSegment, Lyrics, Timing, fragment_timing, normalize};
#[cfg(not(target_arch = "wasm32"))]
use crate::config::directory;
use crate::music::{MusicResult, Track};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    sync::{Mutex, MutexGuard, OnceLock},
};
use tokio::task;

#[derive(Deserialize)]
struct LrclibRecord {
    #[serde(rename = "trackName")]
    track_name: String,
    #[serde(rename = "artistName")]
    artist_name: String,
    #[serde(rename = "albumName", default)]
    album_name: String,
    #[serde(default)]
    duration: f64,
    lyricsfile: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize, Serialize)]
struct UploadEntry {
    uploaded_at: String,
    title: String,
    artist: String,
    album: String,
    duration_ms: u32,
    source: String,
    timing: String,
    lines: usize,
    words: usize,
    lrclib_status: u16,
    key: String,
}

#[derive(Deserialize, Serialize)]
struct LyricsFile {
    version: String,
    metadata: Value,
    #[serde(default)]
    lines: Vec<FileLine>,
}

#[derive(Deserialize, Serialize)]
struct FileLine {
    text: String,
    start_ms: i64,
    end_ms: Option<i64>,
    words: Option<Vec<FileWord>>,
}

#[derive(Deserialize, Serialize)]
struct FileWord {
    text: String,
    start_ms: i64,
    end_ms: Option<i64>,
}

pub(super) async fn fetch_lrclib(track: &Track, http: &Client) -> MusicResult<Lyrics> {
    let records = http
        .get("https://lrclib.net/api/search")
        .query(&[("track_name", track.compact_title()), ("artist_name", track.primary_artist())])
        .header("User-Agent", concat!("Cantus/", env!("CARGO_PKG_VERSION"), " (https://github.com/isthmus)"));
    let records = records.send().await?.error_for_status()?.json::<Vec<LrclibRecord>>().await?;
    let title = normalize(track.compact_title());
    let artists = track.artists.iter().map(|artist| normalize(artist)).collect::<Vec<_>>();
    let target_album = normalize(&track.album);
    let target_duration = f64::from(track.duration_ms) / 1000.0;
    let mut best: Option<(Lyrics, bool, f64)> = None;
    for record in records {
        let matched_title = normalize(&record.track_name);
        let matched_artist = normalize(&record.artist_name);
        let album_match = !target_album.is_empty() && normalize(&record.album_name) == target_album;
        let duration_delta = if record.duration > 0.0 { (record.duration - target_duration).abs() } else { f64::MAX };
        let duration_match = track.duration_ms == 0 || record.duration <= 0.0 || duration_delta <= 5.0;
        let title_match = matched_title == title
            || matched_title == normalize(&track.name)
            || (record.track_name.split_once(" (").is_some_and(|(base, _)| normalize(base) == title) && duration_match);
        if !title_match || !artists.iter().any(|artist| artist == &matched_artist) || (!album_match && !duration_match)
        {
            continue;
        }
        let Some(file) = record.lyricsfile else { continue };
        let Ok(lyrics) = parse_lyricsfile(&file) else { continue };
        if lyrics.is_empty() {
            continue;
        }
        let wins = best.as_ref().is_none_or(|(current, current_album, current_delta)| {
            (lyrics.timing, album_match) > (current.timing, *current_album)
                || ((lyrics.timing, album_match) == (current.timing, *current_album)
                    && current_delta.total_cmp(&duration_delta).is_gt())
        });
        if wins {
            best = Some((lyrics, album_match, duration_delta));
        }
    }
    Ok(best.map_or_else(Lyrics::default, |(lyrics, ..)| lyrics))
}

fn parse_lyricsfile(source: &str) -> MusicResult<Lyrics> {
    let file: LyricsFile = serde_saphyr::from_str(source)?;
    if file.version != "1.0" {
        return Err("Unsupported Lyricsfile version".into());
    }
    for field in ["title", "artist"] {
        if file.metadata[field].as_str().is_none_or(|value| value.trim().is_empty()) {
            return Err(format!("Lyricsfile metadata is missing {field}").into());
        }
    }
    let mut canonical = Vec::new();
    let mut has_words = false;
    for line in file.lines {
        let start = line.start_ms as f32;
        let end = line.end_ms.unwrap_or_else(|| line.start_ms.saturating_add(5000)) as f32;
        if start < 0.0 || end < start {
            continue;
        }
        let words = line.words.unwrap_or_default();
        let mut word_segments = Vec::new();
        if !words.is_empty() {
            has_words = true;
            for word in words {
                let start = word.start_ms as f32;
                let end = word.end_ms.unwrap_or_else(|| word.start_ms.saturating_add(500)) as f32;
                if start >= 0.0 && end >= start {
                    let fragment = LyricSegment { time: start..end, text: word.text };
                    word_segments.push(fragment);
                }
            }
        }
        canonical.push(LyricLine {
            text: line.text,
            time: start..end,
            words: (!word_segments.is_empty()).then_some(word_segments),
            lane: 0,
            singer: Some(0),
        });
    }
    let mut lyrics =
        Lyrics { lines: canonical, timing: if has_words { Timing::Word } else { Timing::Line }, ..Default::default() };
    if has_words {
        lyrics.timing = fragment_timing(&lyrics.lines);
    }
    Ok(lyrics)
}

#[derive(Serialize)]
struct PublishBody<'a> {
    #[serde(rename = "trackName")]
    track_name: &'a str,
    #[serde(rename = "artistName")]
    artist_name: &'a str,
    #[serde(rename = "albumName")]
    album_name: &'a str,
    duration: u32,
    lyricsfile: String,
}

pub(super) async fn publish_lrclib(
    track: &Track,
    lyrics: &Lyrics,
    http: &Client,
    source: &str,
) -> MusicResult<StatusCode> {
    // Publish support is deliberately best-effort and currently requires a fresh PoW token.
    let challenge =
        http.post("https://lrclib.net/api/request-challenge").send().await?.error_for_status()?.json::<Value>().await?;
    let prefix = challenge["prefix"].as_str().ok_or("LRCLIB challenge has no prefix")?;
    let target = challenge["target"].as_str().ok_or("LRCLIB challenge has no target")?;
    let target = hex::decode(target)?;
    if target.len() != 32 {
        return Err("LRCLIB challenge target is not a SHA-256 digest".into());
    }
    let mut nonce = 0u64;
    let mut attempts = 0;
    loop {
        let digest = Sha256::digest(format!("{prefix}{nonce}").as_bytes());
        if digest.as_slice() <= target.as_slice() {
            break;
        }
        nonce += 1;
        attempts += 1;
        if attempts == 4096 {
            attempts = 0;
            #[cfg(not(target_arch = "wasm32"))]
            task::yield_now().await;
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
    }
    let lines: Vec<_> = lyrics
        .lines
        .iter()
        .map(|line| FileLine {
            text: line.text.clone(),
            start_ms: line.time.start.round() as i64,
            end_ms: Some(line.time.end.round() as i64),
            words: line.words.as_ref().filter(|words| !words.is_empty()).map(|words| {
                words
                    .iter()
                    .map(|word| FileWord {
                        text: word.text.clone(),
                        start_ms: word.time.start.round() as i64,
                        end_ms: Some(word.time.end.round() as i64),
                    })
                    .collect()
            }),
        })
        .collect();
    let file = serde_saphyr::to_string(&LyricsFile {
        version: "1.0".into(),
        metadata: json!({
            "title": track.compact_title(),
            "artist": track.primary_artist(),
            "album": track.album,
            "duration_ms": track.duration_ms,
        }),
        lines,
    })?;
    let body = PublishBody {
        track_name: track.compact_title(),
        artist_name: track.primary_artist(),
        album_name: &track.album,
        duration: track.duration_ms / 1000,
        lyricsfile: file,
    };
    let token = format!("{prefix}:{nonce}");
    let response =
        http.post("https://lrclib.net/api/publish").header("X-Publish-Token", token).json(&body).send().await?;
    let status = response.status();
    response.error_for_status()?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(error) = append_upload_log(track, lyrics, source, status) {
        tracing::warn!("LRCLIB upload succeeded, but its local history could not be written: {error}");
    }
    Ok(status)
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn was_uploaded(track: &Track, lyrics: &Lyrics) -> MusicResult<bool> {
    Ok(upload_history()?.contains(&upload_key(track, lyrics)))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn was_uploaded(_: &Track, _: &Lyrics) -> MusicResult<bool> {
    Ok(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn append_upload_log(track: &Track, lyrics: &Lyrics, source: &str, status: StatusCode) -> MusicResult<()> {
    let directory = directory();
    fs::create_dir_all(&directory)?;
    let mut file = OpenOptions::new().create(true).append(true).open(directory.join("lyrics-uploads.jsonl"))?;
    let entry = UploadEntry {
        uploaded_at: jiff::Zoned::now().to_string(),
        title: track.compact_title().to_owned(),
        artist: track.primary_artist().to_owned(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        source: source.to_owned(),
        timing: lyrics.timing.to_string(),
        lines: lyrics.lines.len(),
        words: lyrics.word_count(),
        lrclib_status: status.as_u16(),
        key: upload_key(track, lyrics),
    };
    serde_json::to_writer(&mut file, &entry)?;
    writeln!(file)?;
    upload_history()?.insert(entry.key);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn upload_history() -> MusicResult<MutexGuard<'static, HashSet<String>>> {
    static HISTORY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let history = HISTORY.get_or_init(|| {
        let mut keys = HashSet::new();
        match File::open(directory().join("lyrics-uploads.jsonl")) {
            Ok(file) => {
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    let Ok(entry) = serde_json::from_str::<UploadEntry>(&line) else { continue };
                    keys.insert(entry.key);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!("Could not read LRCLIB upload history: {error}"),
        }
        Mutex::new(keys)
    });
    Ok(history.lock().map_err(|_error| io::Error::other("LRCLIB upload history lock poisoned"))?)
}

#[cfg(not(target_arch = "wasm32"))]
fn upload_key(track: &Track, lyrics: &Lyrics) -> String {
    let mut hash = Sha256::new();
    for value in [track.compact_title(), track.primary_artist(), &track.album] {
        let value = normalize(value);
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(track.duration_ms.to_le_bytes());
    hash.update((lyrics.lines.len() as u64).to_le_bytes());
    for line in &lyrics.lines {
        hash.update((line.text.len() as u64).to_le_bytes());
        hash.update(line.text.as_bytes());
        hash.update((line.time.start.round() as i64).to_le_bytes());
        hash.update((line.time.end.round() as i64).to_le_bytes());
        if let Some(words) = &line.words {
            hash.update([1]);
            hash.update((words.len() as u64).to_le_bytes());
            for word in words {
                hash.update((word.text.len() as u64).to_le_bytes());
                hash.update(word.text.as_bytes());
                hash.update((word.time.start.round() as i64).to_le_bytes());
                hash.update((word.time.end.round() as i64).to_le_bytes());
            }
        } else {
            hash.update([0]);
        }
    }
    hex::encode(hash.finalize())
}
