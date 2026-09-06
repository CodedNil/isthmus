use crate::{
    music::{Lyrics, Music, Track},
    render::{BarLayout, PANEL_START, TEXT_COLOR, TextFragment, UiContext},
};
use isthmus::{
    ColorExt as _, Float as _,
    glam::{Vec4, vec2},
    shader,
};

pub const EXTENSION: f32 = 10.0;

pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    const CLIP_PADDING: f32 = 4.0;

    let Some((index, progress_ms)) = music.timeline.span_at_playhead(&music.queue) else {
        return;
    };
    let screen_width = context.frame.screen_size.x;
    let prepare = |track: &mut Track| {
        if let Some(lyrics) = track.runtime.lyrics.ready_mut() {
            lyrics.prepare(track.duration_ms as f32, context.frame.text);
        }
    };
    let span = |track: &Track| {
        track
            .runtime
            .lyrics
            .ready()
            .filter(|lyrics| lyrics.span > 0.0)
            .map_or_else(|| track.queue_span_ms() * Lyrics::SILENCE_SPEED, |lyrics| lyrics.span)
    };

    prepare(&mut music.queue[index]);
    let current = &music.queue[index];
    let progress =
        current.runtime.lyrics.ready().map_or(progress_ms * Lyrics::SILENCE_SPEED, |lyrics| {
            lyrics.position(progress_ms, current.duration_ms as f32)
        });
    let current_x = layout.playhead_x - progress;
    let mut first = index;
    let mut x = current_x;
    while first > 0 && x >= -CLIP_PADDING {
        first -= 1;
        prepare(&mut music.queue[first]);
        x -= span(&music.queue[first]);
    }

    let y = PANEL_START + context.config.height + EXTENSION;
    let playhead_x = layout.playhead_x;
    for track in &mut music.queue[first..] {
        if x > screen_width + CLIP_PADDING {
            break;
        }
        if let Some(lyrics) = track.runtime.lyrics.ready_mut() {
            lyrics.prepare(track.duration_ms as f32, context.frame.text);
        }
        let track_x = x;
        x += span(track);
        let Some(lyrics) = track.runtime.lyrics.ready() else {
            continue;
        };
        for (background, color) in [(false, TEXT_COLOR.extend(1.0)), (true, Vec4::new(0.72, 0.86, 1.0, 1.0))] {
            let line = lyrics.visible(
                context.frame.text,
                -track_x - CLIP_PADDING..screen_width - track_x + CLIP_PADDING,
                background,
            );
            if line.width <= 0.0 {
                continue;
            }
            let placed = context.frame.text.visible(&line, vec2(track_x, y), 0.0..screen_width).with_color(color);
            let padding = placed.size * 0.2 + 1.0;
            context.frame.paint(
                placed.effects(1.5).displaced(padding),
                shader!(|text: TextFragment<'_>, playhead_x: f32, screen_width: f32| {
                    let edge_fade =
                        text.pixel.x.smoothstep(0.0, 32.0) * text.pixel.x.smoothstep(screen_width, screen_width - 32.0);
                    let emphasis = (text.pixel.x - playhead_x).abs().smoothstep(110.0, 0.0);
                    let weight = text.line.weight + emphasis * 45.0;
                    let sample = text.distance_with_weight(text.pixel, weight).sample();
                    let sung = text.pixel.x.smoothstep(playhead_x + 4.0, playhead_x - 4.0);
                    let fade = edge_fade * (1.0 - sung * 0.5);
                    sample.color(text.line.color.to_vec4(), Vec4::new(0.0, 0.0, 0.0, 0.4), 1.5).opacity(fade)
                }),
            );
        }
    }
}
