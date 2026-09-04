use isthmus::glam::{FloatExt, Vec4, vec2};

use crate::render::{TEXT_COLOR, TextFragment};

pub const EXTENSION: f32 = 10.0;

#[cfg(not(target_arch = "spirv"))]
use crate::{
    music::{Lyrics, Music, Track},
    render::{BarLayout, PANEL_START, UiContext},
};
#[isthmus::paint]
pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    const CLIP_PADDING: f32 = 4.0;

    let Some((index, progress_ms)) = music.timeline.span_at_playhead(&music.queue) else {
        return;
    };
    let shaper = context.frame.text().shaper();
    let prepare = |track: &mut Track| {
        if let Some(lyrics) = track.runtime.lyrics.ready_mut() {
            lyrics.prepare(track.duration_ms as f32, &shaper);
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
    let progress = current
        .runtime
        .lyrics
        .ready()
        .map_or(progress_ms * Lyrics::SILENCE_SPEED, |lyrics| {
            lyrics.position(progress_ms, current.duration_ms as f32)
        });
    let current_x = layout.playhead_x - progress;
    let screen_width = context.frame.screen_size.x;
    let mut visible = vec![(index, current_x)];

    let mut x = current_x;
    for item in (0..index).rev() {
        prepare(&mut music.queue[item]);
        x -= span(&music.queue[item]);
        if x + span(&music.queue[item]) < -CLIP_PADDING {
            break;
        }
        visible.push((item, x));
    }
    visible.reverse();

    x = current_x + span(&music.queue[index]);
    for item in index + 1..music.queue.len() {
        if x > screen_width + CLIP_PADDING {
            break;
        }
        prepare(&mut music.queue[item]);
        visible.push((item, x));
        x += span(&music.queue[item]);
    }

    let y = PANEL_START + context.config.height + EXTENSION;
    let playhead_x = layout.playhead_x;
    for (item, x) in visible {
        let track = &music.queue[item];
        let Some(lyrics) = track.runtime.lyrics.ready() else {
            continue;
        };
        for (background, color) in [(false, TEXT_COLOR.extend(1.0)), (true, Vec4::new(0.72, 0.86, 1.0, 1.0))] {
            let line = lyrics.visible(&shaper, -x - CLIP_PADDING..screen_width - x + CLIP_PADDING, background);
            if line.width <= 0.0 {
                continue;
            }
            let placed = context
                .frame
                .text()
                .visible(&line, vec2(x, y), 0.0..screen_width)
                .with_color(color);
            let padding = placed.size * 0.2 + 1.0;
            context.frame.paint_text(
                placed.expanded(padding),
                |text: TextFragment, playhead_x: f32, screen_width: f32| {
                    let edge_fade =
                        text.pixel.x.smoothstep(0.0, 32.0) * text.pixel.x.smoothstep(screen_width, screen_width - 32.0);
                    let emphasis = (text.pixel.x - playhead_x).abs().smoothstep(110.0, 0.0);
                    let weight = (text.line.weight + emphasis * 0.15).min(1.0);
                    let sample = text.sample_with_weight(text.pixel, weight);
                    let sung = text.pixel.x.smoothstep(playhead_x + 4.0, playhead_x - 4.0);
                    let fade = edge_fade * (1.0 - sung * 0.5);
                    let mut color = sample.color(text.line.color.to_vec4(), Vec4::new(0.0, 0.0, 0.0, 0.4), 1.5);
                    color.w *= fade;
                    color
                },
            );
        }
    }
}
