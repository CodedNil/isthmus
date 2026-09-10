use crate::{
    music::{Music, lyrics::Lyrics},
    render::{BarLayout, PANEL_START, Program, TEXT_COLOR, UiContext},
};
use isthmus::prelude::*;
use isthmus_sdf::prelude::*;

pub const EXTENSION: f32 = 17.0;
const SHADOW_REACH: f32 = 3.0;
const ROW_SPACING: f32 = 12.0;
const GROUP_COLOR: Vec3 = vec3(1.00, 0.72, 0.79); // rose, reserved for groups
const SECTION_SHADOW: f32 = 5.0;
const CHANNEL_COLORS: [Vec3; 3] = [
    TEXT_COLOR,
    vec3(0.66, 0.82, 1.00), // blue
    vec3(0.76, 0.94, 0.69), // sage
];

#[cfg(target_os = "linux")]
pub fn height(music: &Music) -> f32 {
    let rows = music
        .resources
        .lyrics
        .values()
        .filter_map(|state| state.ready())
        .map(|lyrics| lyrics.channels.len())
        .max()
        .unwrap_or(1);
    EXTENSION + rows.saturating_sub(1) as f32 * ROW_SPACING
}

// Each earlier voice frees one lane only inside its long silent gaps.
fn row_at(x: f32, mut row: f32, gaps: Buffer<'_, Vec4>) -> f32 {
    let ease = |start: f32, end: f32| {
        let t = ((x - start) / (end - start)).clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    };
    for index in 0..gaps.len() {
        let gap = gaps.load(index);
        row -= ease(gap.x, gap.y) * (1.0 - ease(gap.z, gap.w));
    }
    row.max(0.0)
}

pub fn show(context: &mut UiContext, music: &mut Music, layout: BarLayout) {
    let Some((index, mut time)) = music.timeline.span_at_playhead(&music.queue) else { return };
    let silence = Lyrics::default();
    let screen = -SHADOW_REACH * 1.2..context.frame.screen_size.x + SHADOW_REACH * 1.2;
    let current = &music.queue[index];
    let lyrics = music.resources.lyrics(current, context.frame.resources).unwrap_or(&silence);
    let mut x = layout.playhead_x - lyrics.position(time, current.duration_ms as f32);
    let mut first = index;
    while first > 0 && x >= screen.start {
        first -= 1;
        let track = &music.queue[first];
        let lyrics = music.resources.lyrics(track, context.frame.resources).unwrap_or(&silence);
        x -= lyrics.position(track.queue_span_ms(), track.duration_ms as f32);
        time += track.queue_span_ms();
    }
    for track in &music.queue[first..] {
        if x > screen.end {
            break;
        }
        let lyrics = music.resources.lyrics(track, context.frame.resources).unwrap_or(&silence);
        let baseline = PANEL_START + context.config.height + EXTENSION;
        for section in &lyrics.sections {
            let start = x + lyrics.position(section.time.start, track.duration_ms as f32);
            let label = context.frame.resources.shape(&section.text, 10.0, f32::MAX);
            let origin = vec2(start, baseline - 11.0);
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
        let mut bends = Vec::new();
        for (row, channel) in lyrics.channels.iter().enumerate() {
            let color = channel.singer.map_or(GROUP_COLOR, |singer| CHANNEL_COLORS[singer % CHANNEL_COLORS.len()]);
            for run in channel
                .runs
                .iter()
                .skip_while(|run| x + run.x.end < screen.start)
                .take_while(|run| x + run.x.start <= screen.end)
            {
                // Slide inside the shared cell so the sung fraction stays under the playhead.
                let sung = ((time - run.time.start) / (run.time.end - run.time.start)).clamp(0.0, 1.0) * run.width;
                let left = run.x.start
                    + (layout.playhead_x - x - run.x.start - sung)
                        .clamp(0.0, (run.x.end - run.x.start - run.width).max(0.0));
                if x + left + run.width < screen.start || x + left > screen.end {
                    continue;
                }
                let origin = vec2(x + left - run.line.text.min.x.min(0.0) * run.scale, baseline);
                let bounds = Rect::new(
                    origin + run.line.text.min * vec2(run.scale, 1.0),
                    origin + run.line.text.max * vec2(run.scale, 1.0) + vec2(0.0, row as f32 * ROW_SPACING),
                )
                .expanded(SHADOW_REACH * run.scale.max(1.0));
                shader!(
                    context
                        .frame
                        .upload({
                            let playhead_x: f32 = layout.playhead_x;
                            let track_x: f32 = x;
                            let row: f32 = row as f32;
                            let gaps: Buffer<'_, Vec4> = Buffer::new(&bends);
                            let color: Vec4 = color.extend(1.0);
                            let scale: f32 = run.scale;
                            let bounds: Rect;
                            let line: Text = context.frame.resources.place(&run.line, origin).outlined(SHADOW_REACH);
                        })
                        .primitive(raster(bounds))
                        .fragment(|_, fragment| {
                            let ahead = fragment.pixel.x - playhead_x;
                            let mut point = line.origin + (fragment.pixel - line.origin) / vec2(scale, 1.0);
                            point.y -= ROW_SPACING * row_at(fragment.pixel.x - track_x, row, gaps);
                            let surface = line
                                .with_weight(700.0.lerp(745.0, ahead.abs().smoothstep(110.0, 0.0)))
                                .sample_at(point);
                            let shadow = surface.distance.smoothstep(SHADOW_REACH, 0.0).powi(2);
                            surface.paint(color, Vec3::splat(0.2).extend(shadow))
                                * vec4(1.0, 1.0, 1.0, (ahead.smoothstep(-10.0, 5.0) + 0.5).min(1.0))
                        })
                );
            }
            bends.extend(
                channel.gaps.iter().copied().filter(|gap| x + gap.w >= screen.start && x + gap.x <= screen.end),
            );
        }
        x += lyrics.position(track.queue_span_ms(), track.duration_ms as f32);
        time -= track.queue_span_ms();
    }
}
