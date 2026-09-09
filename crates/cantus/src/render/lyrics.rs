use crate::{
    music::{Music, lyrics::Lyrics},
    render::{BarLayout, PANEL_START, Program, UiContext},
};
use isthmus::prelude::*;
use isthmus_sdf::prelude::*;

pub const EXTENSION: f32 = 10.0;
const SHADOW_REACH: f32 = 3.0;
const LYRIC_COLOR: Vec4 = Vec4::new(0.94, 0.94, 0.94, 1.0);
const BACKGROUND_LYRIC_COLOR: Vec4 = Vec4::new(0.72, 0.86, 1.0, 1.0);

pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    let Some((index, progress_ms)) = music.timeline.span_at_playhead(&music.queue) else {
        return;
    };
    let silence = Lyrics::default();
    let current = music.resources.lyrics(&music.queue[index], context.frame.resources).unwrap_or(&silence);
    let mut first = index;
    let mut x = layout.playhead_x - current.position(progress_ms);
    while first > 0 && x >= -SHADOW_REACH {
        first -= 1;
        let track = &music.queue[first];
        x -= music.resources.lyrics(track, context.frame.resources).unwrap_or(&silence).span(track.duration_ms as f32);
    }

    let y = PANEL_START + context.config.height + EXTENSION;
    for track in &music.queue[first..] {
        if x > context.frame.screen_size.x + SHADOW_REACH {
            break;
        }
        let lyrics = music.resources.lyrics(track, context.frame.resources).unwrap_or(&silence);
        let track_x = x;
        x += lyrics.span(track.duration_ms as f32);
        for (index, line) in lyrics.lines.iter().enumerate() {
            if line.text.count == 0 {
                continue;
            }
            shader!(
                context
                    .frame
                    .upload({
                        let playhead_x: f32 = layout.playhead_x;
                        let background: bool = index != 0;
                        let line: Text = context.frame.resources.place(line, vec2(track_x, y)).outlined(SHADOW_REACH);
                    })
                    .primitive(raster(line.bounds(0.0)))
                    .fragment(|_, fragment| {
                        let ahead = fragment.pixel.x - playhead_x;
                        let surface = line
                            .with_weight(700.0.lerp(745.0, ahead.abs().smoothstep(110.0, 0.0)))
                            .sample_at(fragment.pixel);
                        let shadow = surface.distance.smoothstep(SHADOW_REACH, 0.0).powi(2);
                        let color = if background { BACKGROUND_LYRIC_COLOR } else { LYRIC_COLOR };
                        surface.paint(color, Vec3::splat(0.2).extend(shadow))
                            * vec4(1.0, 1.0, 1.0, (ahead.smoothstep(-10.0, 5.0) + 0.5).min(1.0))
                    })
            );
        }
    }
}
