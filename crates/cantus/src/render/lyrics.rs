use crate::{
    music::{Music, TRACK_SPACING_MS, lyrics::Lyrics},
    render::{BarLayout, PANEL_START, Program, TEXT_SHADOW, TEXT_SMALL, UiContext},
};
use isthmus::prelude::*;
use isthmus_sdf::{
    layout::{ShapedLine, TextCache},
    prelude::*,
};
use std::{ops::Range, sync::Arc};

const LYRIC_SIZE: f32 = 14.0;
const LYRIC_WEIGHT: f32 = 800.0;
const SHADOW_REACH: f32 = 2.0;
/// Multiplier deepening the shared text shadow behind lyrics and section labels.
const SHADOW_STRENGTH: f32 = 4.0;
const BACKDROP_REACH: f32 = 12.0;
const BACKDROP_OPACITY: f32 = 0.24;
/// One physical pixel of coverage filtering prevents slow-moving stems from
/// gaining/losing ink as they cross fractional pixel phases.
const EDGE_WIDTH: f32 = 1.0;
const GLOW_REACH: f32 = 6.0;
const LANE_GAP: f32 = 96.0;
const ROW_SPACING: f32 = 16.0;
/// Distance from the panel's bottom edge to the first lyric row.
const LYRIC_INSET: f32 = 12.0;
/// Space reserved below the panel for one lyric row and the label beneath it.
#[cfg(target_os = "linux")]
pub const EXTENSION: f32 = LYRIC_INSET + ROW_SPACING;

const GROUP_COLOR: Vec3 = vec3(1.00, 0.72, 0.79);
const CHANNEL_COLORS: [Vec3; 3] = [Vec3::ONE, vec3(0.66, 0.82, 1.00), vec3(0.76, 0.94, 0.69)];

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
        let space = text.shape(" ", LYRIC_SIZE, LYRIC_WEIGHT).text.width.max(6.0);
        let mut channels = Vec::new();
        let mut run_text = Vec::new();
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
                let line = text.shape(value.trim(), LYRIC_SIZE, LYRIC_WEIGHT);
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
                channel.runs.push(Run { time: start..end, x: 0.0, width, line, scale: 1.0, lane: 0 });
                run_text.push(value);
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
        for (run, value) in channels.iter_mut().flat_map(|c| &mut c.runs).zip(run_text) {
            let bounds = run.line.text;
            let held = (run.time.end - run.time.start) / bounds.width.max(1.0) / typical;
            let target_scale = 1.0 + ((held - 1.0) * 0.25).clamp(0.0, 0.25);
            // Resolve the width in the font; anything past its wdth range scales geometrically.
            if target_scale > 1.0 {
                let separator = run.width - ink_width(bounds);
                run.line = text.shape_width(value.trim(), bounds.size, LYRIC_WEIGHT, target_scale * 100.0);
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
        for run in channels.iter_mut().flat_map(|channel| &mut channel.runs) {
            run.line = text.cached_distance(&run.line);
        }
        self.channels = channels;
    }

    /// Lowest lyric row drawn at a track-local x, so a label can sit beneath it.
    fn rows_at(&self, x: f32) -> f32 {
        self.channels
            .iter()
            .flat_map(|channel| &channel.runs)
            .filter(|run| run.x <= x && x <= run.x + run.width)
            .map(|run| run.lane as f32 + 1.0)
            .fold(1.0, f32::max)
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

fn history_opacity(x: f32, playhead_x: f32) -> f32 {
    if x >= playhead_x { 1.0 } else { x.smoothstep(0.0, playhead_x.max(1.0)) }
}

fn lyric_shadow(distance: f32) -> Vec4 {
    let strength = distance.smoothstep(SHADOW_REACH, 0.0).powi(2) * SHADOW_STRENGTH;
    let backdrop = (-distance.max(0.0) * 0.25).exp()
        * (1.0 - distance.smoothstep(BACKDROP_REACH - 3.0, BACKDROP_REACH))
        * BACKDROP_OPACITY;
    source_over(TEXT_SHADOW.with_w(TEXT_SHADOW.w * strength), Vec3::ZERO.extend(backdrop))
}

fn lyric_sample(mut sample: Sample) -> Sample {
    // The shared SDF uses |dx| + |dy|, widening diagonal edges. Use the
    // Euclidean derivative magnitude here without changing other UI surfaces.
    let sum = sample.gradient.abs().element_sum().max(f32::MIN_POSITIVE);
    sample.gradient *= sample.gradient.length() * EDGE_WIDTH / sum;
    sample
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
    LYRIC_INSET + rows as f32 * ROW_SPACING
}

pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    let Some((index, time)) = music.queue.span_at_playhead() else { return };
    let tracks = music.queue.tracks.as_slice();
    let resources = &mut music.resources;
    let silence = PreparedLyrics::default();
    let screen = -BACKDROP_REACH * 1.2..context.frame.screen_size.x + BACKDROP_REACH * 1.2;
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
        let baseline = PANEL_START + context.config.height + LYRIC_INSET;
        for section in &lyrics.source.sections {
            let local = lyrics.position(section.time.start, track.duration_ms as f32);
            let label = context.frame.resources.shape(&section.text, TEXT_SMALL, f32::MAX);
            let origin = vec2(x + local, baseline + lyrics.rows_at(local) * ROW_SPACING);
            shader!(
                context
                    .frame
                    .upload({
                        let playhead_x: f32 = layout.playhead_x;
                        let line: Text = context.frame.resources.place(&label, origin).outlined(BACKDROP_REACH);
                    })
                    .primitive(line)
                    .fragment(|_, surface| {
                        let sample = lyric_sample(surface.sample);
                        let color = sample.paint(Vec4::ONE, lyric_shadow(sample.distance));
                        color.with_w(color.w / surface.coverage.max(f32::MIN_POSITIVE))
                            * vec4(1.0, 1.0, 1.0, history_opacity(surface.pixel.x, playhead_x))
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
                .expanded(BACKDROP_REACH * run.scale + 2.0);
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
                            let line: Text = context.frame.resources.place(&run.line, origin).outlined(BACKDROP_REACH);
                        })
                        .primitive(raster(bounds))
                        .fragment(|_, fragment| {
                            let distance = fragment.pixel.x - playhead_x;
                            let active = (1.0 - (distance / 52.0).powi(2)).max(0.0).powi(2);
                            let wave = distance * 0.16 - phase * 3.0;
                            let mut point = line.origin + (fragment.pixel - line.origin) / vec2(scale, 1.0);
                            point.y -= ROW_SPACING * row;
                            point.y += active * (0.6 + 0.4 * wave.sin());
                            let surface = lyric_sample(line.sample_at(point));
                            let mut ink = color.truncate();
                            let mut halo = lyric_shadow(surface.distance);
                            if active > 0.0 {
                                let sheen = (wave + (fragment.pixel.y - baseline) * 0.25).sin().max(0.0).powi(8);
                                ink = ink.lerp(Vec3::ONE, active * (0.3 + 0.7 * sheen));
                                let glow = active
                                    * (0.1 + 0.12 * sheen)
                                    * (-surface.distance.max(0.0) * 0.6).exp()
                                    * surface.distance.smoothstep(GLOW_REACH, GLOW_REACH - 2.0);
                                halo = source_over(color.truncate().lerp(Vec3::ONE, 0.7).extend(glow), halo);
                            }
                            let opacity = history_opacity(fragment.pixel.x, playhead_x);
                            surface.paint(ink.extend(1.0), halo) * vec4(1.0, 1.0, 1.0, opacity)
                        })
                );
            }
        }
        x += lyrics.position(track.queue_span_ms(), track.duration_ms as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        music::lyrics::{Channel as SourceChannel, LyricSegment},
        render::text_cache,
    };
    use isthmus::Resources;
    use isthmus_sdf::text::Glyphs;

    #[test]
    fn cached_glyph_fields_preserve_edges_and_reuse_storage() {
        let mut cache = text_cache();
        let original = cache.shape("AVOxy gj♪", LYRIC_SIZE, LYRIC_WEIGHT);
        let prefix = cache.data().persistent.to_vec();
        let cached = cache.cached_distance(&original);
        assert_eq!(&cache.data().persistent[..prefix.len()], prefix.as_slice());
        let words = cache.data().persistent.len();
        let again = cache.cached_distance(&original);
        assert!(Arc::ptr_eq(&cached, &again));
        assert_eq!(cache.data().persistent.len(), words);
        assert_eq!(again.text.width, original.text.width);
        let exact = cache.place(&original, Vec2::ZERO);
        let field = cache.place(&cached, Vec2::ZERO);
        let data = cache.data();
        let exact = Glyphs::new(exact, data);
        let field = Glyphs::new(field, data);
        let mut maximum_error = 0.0;
        for y in -50..50 {
            for x in -30..(original.text.width * 3.0) as i32 + 30 {
                let point = vec2(x as f32 / 3.0 + 0.13, y as f32 / 3.0 + 0.07);
                let reference = exact.distance_at(point);
                if reference.abs() < BACKDROP_REACH {
                    let error = (field.distance_at(point) - reference).abs();
                    maximum_error = maximum_error.max(error);
                }
            }
        }
        assert!(maximum_error < 0.18, "cached field error: {maximum_error} px");
    }

    #[test]
    fn lyric_edges_have_the_same_width_at_different_angles() {
        for gradient in [Vec2::X, Vec2::Y, Vec2::ONE.normalize()] {
            let sample = lyric_sample(Sample { distance: -0.2, gradient, coverage: 1.0 });
            assert!((sample.gradient.abs().element_sum() - EDGE_WIDTH).abs() < 0.0001);
            assert!((sample.fill() - (0.5 + 0.2 / EDGE_WIDTH)).abs() < 0.0001);
        }
    }

    #[test]
    fn moving_stems_preserve_coverage_across_pixel_phases() {
        for width in [1.5, 2.3, 3.7] {
            for phase in 0..16 {
                let offset = phase as f32 / 16.0;
                let coverage: f32 = (-8..9)
                    .map(|pixel| {
                        lyric_sample(Sample {
                            distance: (pixel as f32 - offset).abs() - width * 0.5,
                            gradient: Vec2::X,
                            coverage: 1.0,
                        })
                        .fill()
                    })
                    .sum();
                assert!((coverage - width).abs() < 0.0001, "coverage changed at phase {phase}");
            }
        }
    }

    #[test]
    fn backdrop_follows_text_and_disappears_outside_its_reach() {
        assert!(lyric_shadow(SHADOW_REACH + 1.0).w > 0.0);
        assert!(lyric_shadow(0.0).w > lyric_shadow(SHADOW_REACH + 1.0).w);
        assert!(lyric_shadow(0.0).w < 1.0);
        assert_eq!(lyric_shadow(BACKDROP_REACH).w, 0.0);
        assert_eq!(lyric_shadow(BACKDROP_REACH + 10.0).w, 0.0);
    }

    #[test]
    fn fades_only_to_the_left_of_the_playhead() {
        for playhead in [0.0, 0.5, 380.0] {
            for offset in [0.0, 0.25, 100.0, 2000.0] {
                assert_eq!(history_opacity(playhead + offset, playhead), 1.0);
            }
        }
        assert_eq!(history_opacity(0.0, 380.0), 0.0);
        assert_eq!(history_opacity(190.0, 380.0), 0.5);
        assert!(history_opacity(300.0, 380.0) > history_opacity(100.0, 380.0));
    }

    #[test]
    fn prepares_text_in_channel_order_across_empty_channels() {
        let mut text = text_cache();
        let source = Lyrics {
            channels: vec![
                SourceChannel::default(),
                SourceChannel {
                    segments: vec![LyricSegment { time: 0.0..100.0, text: "  ".into() }, LyricSegment {
                        time: 100.0..200.0,
                        text: "🎵 ".into(),
                    }],
                    ..
                },
                SourceChannel::default(),
                SourceChannel { segments: vec![LyricSegment { time: 200.0..1000.0, text: "Hello ".into() }], .. },
            ],
            ..Default::default()
        };
        let mut lyrics = PreparedLyrics::new(source);
        lyrics.prepare(1000.0, &mut text);
        assert_eq!(lyrics.channels.len(), 2);
        for (channel, value) in lyrics.channels.iter().zip(["♪", "Hello"]) {
            assert_eq!(channel.runs.len(), 1);
            let run = &channel.runs[0];
            let expected = text.shape(value, LYRIC_SIZE, LYRIC_WEIGHT);
            assert_eq!(run.line.text.width, expected.text.width);
            assert_eq!(run.line.text.count, expected.text.count);
            assert_eq!(run.line.text.min, expected.text.min);
            assert_eq!(run.line.text.max, expected.text.max);
        }

        lyrics.prepare(0.0, &mut text);
        assert!(lyrics.channels.is_empty());
        assert_eq!(lyrics.scroll.len(), 0);
    }

    #[test]
    fn stretches_held_words_without_losing_their_text() {
        let mut text = text_cache();
        let source = Lyrics {
            channels: vec![SourceChannel {
                segments: vec![
                    LyricSegment { time: 0.0..100.0, text: "sing ".into() },
                    LyricSegment { time: 100.0..200.0, text: "sing ".into() },
                    LyricSegment { time: 200.0..2000.0, text: "sing ".into() },
                ],
                ..
            }],
            ..Default::default()
        };
        let normal_width = text.shape("sing", LYRIC_SIZE, LYRIC_WEIGHT).text.width;
        let mut lyrics = PreparedLyrics::new(source);
        lyrics.prepare(2000.0, &mut text);
        let runs = &lyrics.channels[0].runs;
        assert_eq!(runs.len(), 3);
        assert!(runs[2].line.text.width * runs[2].scale > normal_width * 1.2);
        assert!(runs.windows(2).all(|pair| pair[0].x < pair[1].x));
    }
}
