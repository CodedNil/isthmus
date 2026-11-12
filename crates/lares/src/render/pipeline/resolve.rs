//! The final pass: unpacks the samples, averages them in linear light and composites the presence ghosts.
use crate::{
    presence::Ghost,
    render::{Frame, Program, pipeline::ghosts},
};
use isthmus::prelude::*;
use isthmus_2d::Rect;

/// Averages the valid samples in linear light; a camera change invalidates all of them.
pub fn resolve(frame: &mut Frame<'_>, images: &[Image; 4], mask: u32, interleaved: bool, people: &[Ghost]) {
    let first: &Image = &images[0];
    let second: &Image = &images[1];
    let third: &Image = &images[2];
    let fourth: &Image = &images[3];
    let sample_size = Vec2::from_array(images[0].size().map(|value| value as f32));
    shader!(
        "resolve",
        frame.blend(Blend::Replace),
        |frame,
         first: &Image,
         second: &Image,
         third: &Image,
         fourth: &Image,
         mask: u32,
         sample_size: Vec2,
         interleaved: bool,
         people: &[Ghost]| Rect::new(Vec2::ZERO, frame.screen_size),
        |frame, surface| {
            let size = sample_size;
            let p = surface.pixel * size / frame.screen_size;
            let color = resolved_sample(first, p, size, mask, 0, interleaved)
                + resolved_sample(second, p, size, mask, 1, interleaved)
                + resolved_sample(third, p, size, mask, 2, interleaved)
                + resolved_sample(fourth, p, size, mask, 3, interleaved);
            let ndc = surface.pixel / frame.screen_size * 2.0 - 1.0;
            let globals = frame.globals;
            let ray = (globals.forward + globals.right * ndc.x - globals.up * ndc.y).normalize();
            let height = resolved_height(first, p, size, mask, interleaved);
            let color = ghosts::composite(
                color.truncate() / color.w.max_num(1.0),
                people,
                globals.eye,
                ray,
                height,
                globals.pixel_angle,
            );
            color.powf(1.0 / 2.2).extend(1.0)
        },
    );
}

/// Where the interleaved image holds a native pixel: its own column or the neighbour's, and whether either is valid.
fn interleaved_position(pixel: Vec2, size: Vec2, mask: u32, sample: u32) -> (Vec2, bool) {
    let mut parity = pixel.x.floor() as u32 & 1;
    if mask & (1 << (sample * 2 + parity)) == 0 {
        parity ^= 1;
    }
    let valid = mask & (1 << (sample * 2 + parity)) != 0;
    let half = (size.x * 0.5).ceil();
    let start = if parity != 0 { half } else { 0.0 };
    let end = if parity != 0 { size.x } else { half };
    (vec2(((pixel.x + 0.5 - parity as f32) * 0.5 + start).clamp_num(start + 0.5, end - 0.5), pixel.y), valid)
}

fn resolved_height(image: &Image, pixel: Vec2, size: Vec2, mask: u32, interleaved: bool) -> f32 {
    let position = if interleaved { interleaved_position(pixel, size, mask, 0).0 } else { pixel };
    image.sample_raw(position / size).w * 3.0
}

fn resolved_sample(image: &Image, pixel: Vec2, size: Vec2, mask: u32, sample: u32, interleaved: bool) -> Vec4 {
    let (position, valid) =
        if interleaved { interleaved_position(pixel, size, mask, sample) } else { (pixel, mask & (1 << sample) != 0) };
    if !valid {
        return Vec4::ZERO;
    }
    let mut color = image.sample_raw(position / size).truncate().powf(2.2);
    if !interleaved && mask.count_ones() < 4 {
        // Softens high-contrast edges until all four samples are in.
        let a = image.sample_raw((position + vec2(0.5, 0.0)) / size).truncate().powf(2.2);
        let b = image.sample_raw((position - vec2(0.5, 0.0)) / size).truncate().powf(2.2);
        let c = image.sample_raw((position + vec2(0.0, 0.5)) / size).truncate().powf(2.2);
        let d = image.sample_raw((position - vec2(0.0, 0.5)) / size).truncate().powf(2.2);
        let high = a.max(b).max(c).max(d).max_element();
        let low = a.min(b).min(c).min(d).min_element();
        if high - low > (high + 0.01) * 0.15 {
            color = color.lerp((a + b + c + d) * 0.25, (4 - mask.count_ones()) as f32 / 6.0);
        }
    }
    color.extend(1.0)
}

#[cfg(test)]
mod tests {
    use crate::render::pipeline::resolve::*;

    #[test]
    fn interleaved_resolve_restores_every_native_pixel_for_even_and_odd_widths() {
        for width in 1u32..=33 {
            let half = width.div_ceil(2);
            let mut bytes = vec![0u8; width as usize * 4];
            for x in 0..width {
                let slot = x / 2 + if x & 1 != 0 { half } else { 0 };
                bytes[slot as usize * 4..slot as usize * 4 + 4].copy_from_slice(&[(x * 7) as u8, 64, 128, 255]);
            }
            let image = Image::rgba8([width, 1], bytes);
            for x in 0..width {
                let value = resolved_sample(&image, vec2(x as f32 + 0.5, 0.5), vec2(width as f32, 1.0), 3, 0, true);
                let expected = vec3((x * 7) as f32, 64.0, 128.0) / 255.0;
                assert!((value.truncate().powf(1.0 / 2.2) - expected).length() < 0.00001);
                assert_eq!(value.w, 1.0);
            }
            assert_eq!(resolved_sample(&image, vec2(0.5, 0.5), vec2(width as f32, 1.0), 0, 0, true), Vec4::ZERO);
        }
    }
}
