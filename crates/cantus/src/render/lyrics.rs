use crate::{
    music::{
        Music, Track,
        enrichment::{Fetch, TrackCache},
        lyrics::Lyrics,
    },
    render::{BarLayout, PANEL_START, Program, UiContext},
};
use isthmus::prelude::*;
use isthmus_sdf::{Text, layout::TextCache};

pub const EXTENSION: f32 = 10.0;
const SHADOW_REACH: f32 = 3.0;
const LYRIC_COLOR: Vec4 = Vec4::new(0.94, 0.94, 0.94, 1.0);
const BACKGROUND_LYRIC_COLOR: Vec4 = Vec4::new(0.72, 0.86, 1.0, 1.0);

pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    let Some((index, progress_ms)) = music.timeline.span_at_playhead(&music.queue) else {
        return;
    };
    let prepare = |track: &Track, resources: &mut TrackCache, text: &mut TextCache| {
        if let Some(Fetch::Ready(lyrics)) = resources.lyrics.get_mut(&track.uri) {
            lyrics.prepare(track.duration_ms as f32, text);
        }
    };
    let span = |track: &Track, resources: &TrackCache| {
        resources
            .lyrics
            .get(&track.uri)
            .and_then(Fetch::ready)
            .filter(|lyrics| lyrics.span > 0.0)
            .map_or_else(|| track.queue_span_ms() * Lyrics::SILENCE_SPEED, |lyrics| lyrics.span)
    };
    prepare(&music.queue[index], &mut music.resources, context.frame.resources);
    let current = &music.queue[index];
    let progress = music
        .resources
        .lyrics
        .get(&current.uri)
        .and_then(Fetch::ready)
        .map_or(progress_ms * Lyrics::SILENCE_SPEED, |lyrics| lyrics.position(progress_ms, current.duration_ms as f32));
    let mut first = index;
    let mut x = layout.playhead_x - progress;
    while first > 0 && x >= -SHADOW_REACH {
        first -= 1;
        prepare(&music.queue[first], &mut music.resources, context.frame.resources);
        x -= span(&music.queue[first], &music.resources);
    }

    let y = PANEL_START + context.config.height + EXTENSION;
    for track in &mut music.queue[first..] {
        if x > context.frame.screen_size.x + SHADOW_REACH {
            break;
        }
        prepare(track, &mut music.resources, context.frame.resources);
        let track_x = x;
        x += span(track, &music.resources);
        let Some(lyrics) = music.resources.lyrics.get(&track.uri).and_then(Fetch::ready) else {
            continue;
        };
        for (index, line) in lyrics.lines.iter().enumerate() {
            if line.text.width <= 0.0 {
                continue;
            }
            shader!(
                context
                    .frame
                    .upload({
                        let playhead_x: f32 = layout.playhead_x;
                        let lyric_layer: u32 = index as u32;
                        let line: Text = context.frame.resources.place(line, vec2(track_x, y)).outlined(SHADOW_REACH);
                    })
                    .vertex(line)
                    .fragment(|_, text| {
                        let ahead = text.pixel.x - playhead_x;
                        let weight = 700.0.lerp(745.0, ahead.abs().smoothstep(110.0, 0.0));
                        let sample = text.with_weight(weight).sample_at(text.pixel);
                        let shadow = sample.coverage * sample.distance.smoothstep(SHADOW_REACH, 0.0).powi(2);
                        let color = if lyric_layer == 0 { LYRIC_COLOR } else { BACKGROUND_LYRIC_COLOR };
                        color
                            .opacity(sample.fill())
                            .over(Vec3::splat(0.2).extend(shadow))
                            .opacity((ahead.smoothstep(-10.0, 5.0) + 0.5).min(1.0))
                    })
            );
        }
    }
}
