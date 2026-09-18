use crate::{
    music::{Music, TRACK_SPACING_MS, lyrics::Lyrics},
    render::{BarLayout, PANEL_START, Program, TEXT_COLOR, UiContext},
};
use isthmus::prelude::*;
use isthmus_sdf::{
    layout::{ShapedLine, TextCache},
    prelude::*,
};
use std::{ops::Range, slice, sync::Arc};

pub const EXTENSION: f32 = 17.0;
const SHADOW_REACH: f32 = 3.0;
const GLOW_REACH: f32 = 6.0;
const LANE_GAP: f32 = 96.0;
const ROW_SPACING: f32 = 12.0;
const GROUP_COLOR: Vec3 = vec3(1.00, 0.72, 0.79); // rose, reserved for groups
const SECTION_SHADOW: f32 = 5.0;
const CHANNEL_COLORS: [Vec3; 3] = [
    TEXT_COLOR,
    vec3(0.66, 0.82, 1.00), // blue
    vec3(0.76, 0.94, 0.69), // sage
];

#[derive(Default)]
pub struct PreparedLyrics {
    source: Lyrics,
    channels: Vec<Channel>,
    // Time, position and velocity at each timing boundary.
    scroll: Vec<Vec3>,
    prepared_duration: Option<f32>,
}

struct Channel {
    singer: Option<usize>,
    runs: Vec<Run>,
}

struct Run {
    time: Range<f32>,
    text: String,
    x: f32,
    width: f32,
    line: Arc<ShapedLine>,
    scale: f32,
    /// Rows raised by earlier voices overlapping this section.
    lane: u32,
}

impl PreparedLyrics {
    const SILENCE_SPEED: f32 = 0.035;

    pub fn new(source: Lyrics) -> Self {
        Self { source, ..Default::default() }
    }

    pub(crate) fn prepare(&mut self, duration: f32, text: &mut TextCache) {
        if self.prepared_duration == Some(duration) {
            return;
        }
        self.prepared_duration = Some(duration);
        if !duration.is_finite() || duration <= 0.0 {
            self.channels.clear();
            self.scroll.clear();
            return;
        }
        // Leave visible air between words even with their dark readability outlines.
        let space = text.shape(" ", 15.0, 700.0).text.width.max(6.0);
        let mut channels = Vec::new();
        for source in &mut self.source.channels {
            let mut channel = Channel { singer: source.singer, runs: Vec::new() };
            let segments = &mut source.segments;
            segments.retain(|s| s.time.start.is_finite() && s.time.end.is_finite() && !s.text.trim().is_empty());
            segments.sort_by(|a, b| a.time.start.total_cmp(&b.time.start));
            segments.dedup_by(|b, a| {
                if !a.time.start.total_cmp(&b.time.start).is_eq() {
                    return false;
                }
                a.text.push_str(&b.text);
                a.time.end = a.time.end.max(b.time.end);
                true
            });
            for (index, segment) in segments.iter().enumerate() {
                let next = segments.get(index + 1);
                let value = segment.text.replace('🎵', "♪").replace('🎶', "♫");
                let line = text.shape(value.trim(), 15.0, 700.0);
                let bounds = line.text;
                let start = segment.time.start.clamp(0.0, duration);
                let end =
                    segment.time.end.clamp(start, next.map_or(duration, |s| s.time.start.min(duration)).max(start));
                if bounds.count == 0 || end <= start {
                    continue;
                }
                // Time the separator with the word so adjacent runs meet without a scrolling jump.
                let separated = value.ends_with(char::is_whitespace)
                    || next.is_some_and(|s| s.text.starts_with(char::is_whitespace));
                let width = ink_width(bounds) + space * f32::from(separated);
                channel.runs.push(Run {
                    time: start..end,
                    text: value.trim().to_owned(),
                    x: 0.0,
                    width,
                    line,
                    scale: 1.0,
                    lane: 0,
                });
            }
            channels.push(channel);
        }
        channels.retain(|channel| !channel.runs.is_empty());
        // Compare fragments with this track's typical pace, retaining supplied syllable boundaries.
        let mut pace: Vec<_> = channels
            .iter()
            .flat_map(|c| &c.runs)
            .map(|r| (r.time.end - r.time.start) / r.line.text.width.max(1.0))
            .collect();
        pace.sort_by(f32::total_cmp);
        let typical = pace.get(pace.len() / 2).copied().unwrap_or(1.0);
        for run in channels.iter_mut().flat_map(|c| &mut c.runs) {
            let bounds = run.line.text;
            let held = (run.time.end - run.time.start) / bounds.width.max(1.0) / typical;
            let target_scale =
                if self.source.timing == "caption" { 1.0 } else { 1.0 + ((held - 1.0) * 0.25).clamp(0.0, 0.25) };
            // Resolve the width in the font; anything past its wdth range scales geometrically.
            if target_scale > 1.0 {
                let separator = run.width - ink_width(bounds);
                run.line = text.shape_width(&run.text, bounds.size, 700.0, target_scale * 100.0);
                let shaped = run.line.text;
                run.scale = (target_scale * bounds.width.max(1.0) / shaped.width.max(1.0)).max(1.0);
                run.width = ink_width(shaped) * run.scale + separator;
            }
        }
        // A shared scroll speed reserves enough space for the fastest simultaneous voice.
        let mut times = vec![0.0, duration];
        times.extend(channels.iter().flat_map(|c| &c.runs).flat_map(|run| [run.time.start, run.time.end]));
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| a.total_cmp(b).is_eq());
        self.scroll = vec![Vec3::ZERO];
        let mut x = 0.0;
        for pair in times.windows(2) {
            let speed = channels
                .iter()
                .filter_map(|channel| {
                    let runs = &channel.runs;
                    let next = runs.partition_point(|r| r.time.start <= pair[0]);
                    let run = runs[..next].last()?;
                    (run.time.end > pair[0]).then_some(run.width / (run.time.end - run.time.start))
                })
                .reduce(f32::max)
                .unwrap_or(Self::SILENCE_SPEED);
            x += (pair[1] - pair[0]) * speed;
            self.scroll.push(vec3(pair[1], x, 0.0));
        }
        // Harmonic tangents make velocity continuous without overshooting timing anchors.
        let speeds: Vec<_> = self.scroll.windows(2).map(|p| (p[1].y - p[0].y) / (p[1].x - p[0].x)).collect();
        for (i, point) in self.scroll.iter_mut().enumerate() {
            let before = speeds[i.saturating_sub(1)];
            let after = speeds[i.min(speeds.len() - 1)];
            point.z = if before + after > 0.0 { 2.0 * before * after / (before + after) } else { 0.0 };
        }
        // A whole section of a voice shares one lane, so a single word never drifts off it.
        let mut section_starts: Vec<f32> = self.source.sections.iter().map(|section| section.time.start).collect();
        section_starts.sort_by(f32::total_cmp);
        let mut occupied: Vec<Vec<(f32, f32)>> = Vec::new();
        for channel in &mut channels {
            for run in &mut channel.runs {
                run.x = self.position(run.time.start, duration);
            }
            let section = |time: f32| section_starts.partition_point(|start| *start <= time);
            let mut phrases = Vec::new();
            let mut first = 0;
            for index in 0..channel.runs.len() {
                let boundary = channel.runs.get(index + 1).is_none_or(|next| {
                    if section_starts.is_empty() {
                        // Without section labels, split only at pauses long enough to be a gap.
                        next.x - (channel.runs[index].x + channel.runs[index].width) > LANE_GAP
                    } else {
                        section(next.time.start) != section(channel.runs[index].time.start)
                    }
                });
                if !boundary {
                    continue;
                }
                let phrase = (channel.runs[first].x, channel.runs[index].x + channel.runs[index].width);
                let lane = occupied
                    .iter()
                    .filter(|voice| {
                        voice.iter().any(|span| {
                            phrase.1.min(span.1) - phrase.0.max(span.0)
                                > (phrase.1 - phrase.0).min(span.1 - span.0) * 0.5
                        })
                    })
                    .count() as u32;
                for run in &mut channel.runs[first..=index] {
                    run.lane = lane;
                }
                phrases.push(phrase);
                first = index + 1;
            }
            occupied.push(phrases);
        }
        self.channels = channels;
    }

    pub fn position(&self, time: f32, duration: f32) -> f32 {
        let at = time.min(duration);
        let next = self.scroll.partition_point(|point| point.x <= at);
        let x = match (self.scroll[..next].last(), self.scroll.get(next)) {
            (Some(a), Some(b)) => {
                let span = b.x - a.x;
                let t = (at - a.x) / span;
                let delta = b.y - a.y;
                a.y + t
                    * (span * a.z
                        + t * (3.0 * delta - span * (2.0 * a.z + b.z) + t * (span * (a.z + b.z) - 2.0 * delta)))
            }
            (Some(a), None) => a.y,
            _ => at * Self::SILENCE_SPEED,
        };
        x + (time - duration).clamp(0.0, TRACK_SPACING_MS) * 96.0 / TRACK_SPACING_MS
    }
}

/// Horizontal ink extent of a shaped line, including glyphs that overhang the advance.
fn ink_width(bounds: Text) -> f32 {
    bounds.width.max(bounds.max.x) - bounds.min.x.min(0.0)
}

#[cfg(target_os = "linux")]
pub fn height(music: &Music) -> f32 {
    let rows = music
        .resources
        .lyrics
        .values()
        .filter_map(|state| state.ready())
        .map(|lyrics| lyrics.source.channels.len())
        .max()
        .unwrap_or(1);
    EXTENSION + rows.saturating_sub(1) as f32 * ROW_SPACING
}

pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    let (tracks, index, time) = if let Some(local) = music.local.as_ref().filter(|_| music.local_foreground()) {
        (slice::from_ref(&local.track), 0, -local.timeline.queue_start_ms)
    } else {
        let Some((index, time)) = music.timeline.span_at_playhead(&music.queue) else { return };
        (music.queue.as_slice(), index, time)
    };
    let resources = &mut music.resources;
    let silence = PreparedLyrics::default();
    let screen = -GLOW_REACH * 1.2..context.frame.screen_size.x + GLOW_REACH * 1.2;
    let current = &tracks[index];
    let lyrics = resources.lyrics(current, context.frame.resources).unwrap_or(&silence);
    let mut x = layout.playhead_x - lyrics.position(time, current.duration_ms as f32);
    let mut first = index;
    while first > 0 && x >= screen.start {
        first -= 1;
        let track = &tracks[first];
        let lyrics = resources.lyrics(track, context.frame.resources).unwrap_or(&silence);
        x -= lyrics.position(track.queue_span_ms(), track.duration_ms as f32);
    }
    for track in &tracks[first..] {
        if x > screen.end {
            break;
        }
        let lyrics = resources.lyrics(track, context.frame.resources).unwrap_or(&silence);
        let baseline = PANEL_START + context.config.height + EXTENSION;
        for section in &lyrics.source.sections {
            let start = x + lyrics.position(section.time.start, track.duration_ms as f32);
            let label = context.frame.resources.shape(&section.text, 10.0, f32::MAX);
            let origin = vec2(start, baseline - 10.0);
            shader!(
                context
                    .frame
                    .upload({
                        let line: Text = context.frame.resources.place(&label, origin).outlined(SECTION_SHADOW);
                    })
                    .primitive(line)
                    .fragment(|_, surface| {
                        let shadow = (-surface.distance.max(0.0) * 0.7).exp() * 0.3;
                        surface.paint(TEXT_COLOR.extend(0.9), Vec3::splat(0.2).extend(shadow))
                    })
            );
        }
        for channel in &lyrics.channels {
            let color = channel.singer.map_or(GROUP_COLOR, |singer| CHANNEL_COLORS[singer % CHANNEL_COLORS.len()]);
            for run in channel
                .runs
                .iter()
                .skip_while(|run| x + run.x + run.width < screen.start)
                .take_while(|run| x + run.x <= screen.end)
            {
                let origin = vec2(x + run.x - run.line.text.min.x.min(0.0) * run.scale, baseline);
                let stretch = vec2(run.scale, 1.0);
                let bounds = Rect::new(
                    origin + run.line.text.min * stretch,
                    origin + run.line.text.max * stretch + vec2(0.0, run.lane as f32 * ROW_SPACING),
                )
                .expanded(GLOW_REACH * run.scale + 2.0);
                // One continuous light field crosses every fragment and vocal lane at the playhead.
                shader!(
                    context
                        .frame
                        .upload({
                            let playhead_x: f32 = layout.playhead_x;
                            let row: f32 = run.lane as f32;
                            let scale: f32 = run.scale;
                            let baseline: f32;
                            let phase: f32 = context.frame.time;
                            let color: Vec4 = color.extend(1.0);
                            let bounds: Rect;
                            let line: Text = context.frame.resources.place(&run.line, origin).outlined(GLOW_REACH);
                        })
                        .primitive(raster(bounds))
                        .fragment(|_, fragment| {
                            let distance = fragment.pixel.x - playhead_x;
                            let active = (1.0 - (distance / 52.0).powi(2)).max(0.0).powi(2);
                            let wave = distance * 0.16 - phase * 3.0;
                            let mut point = line.origin + (fragment.pixel - line.origin) / vec2(scale, 1.0);
                            point.y -= ROW_SPACING * row;
                            point.y += active * (0.6 + 0.4 * wave.sin());
                            let surface = line.sample_at(point);
                            let shadow = surface.distance.smoothstep(SHADOW_REACH, 0.0).powi(2);
                            let mut ink = color.truncate();
                            let mut halo = Vec3::splat(0.2).extend(shadow);
                            if active > 0.0 {
                                let sheen = (wave + (fragment.pixel.y - baseline) * 0.25).sin().max(0.0).powi(8);
                                ink = ink.lerp(Vec3::ONE, active * (0.3 + 0.7 * sheen));
                                let glow = active
                                    * (0.1 + 0.12 * sheen)
                                    * (-surface.distance.max(0.0) * 0.6).exp()
                                    * surface.distance.smoothstep(GLOW_REACH, GLOW_REACH - 2.0);
                                halo = source_over(color.truncate().lerp(Vec3::ONE, 0.7).extend(glow), halo);
                            }
                            let opacity = 0.6 + 0.4 * (fragment.pixel.x - playhead_x).smoothstep(-8.0, 0.0);
                            surface.paint(ink.extend(1.0), halo) * vec4(1.0, 1.0, 1.0, opacity)
                        })
                );
            }
        }
        x += lyrics.position(track.queue_span_ms(), track.duration_ms as f32);
    }
}
