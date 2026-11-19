//! Shader-side runtime: the vertex stage every shader shares, the reading and writing around a shader's
//! fragment closure, and the slice view of captured data. Generated entry points only bind resources and call
//! in here.
//!
//! Each draw is a record of `1 + D::WORDS` words: the offset of the frame state, then the shader's payload,
//! whose first field is the region to rasterize.
use crate::{
    Fragment, FrameState, Primitive, Program, ShaderData, ShaderFrame,
    glam::{Vec2, Vec4, vec2, vec4},
};
use core::marker::PhantomData;
use spirv_std::arch::kill;

/// Places one corner of the draw's region.
#[doc(hidden)]
pub fn vertex(
    words: &[u32],
    payload_words: usize,
    index: u32,
    draw: u32,
    position: &mut Vec4,
    uv: &mut Vec2,
    out_draw: &mut u32,
) {
    let record = draw as usize * (payload_words + 1);
    // SAFETY: The recorder aligns complete records to this stride; the region is the payload's first field,
    // and the screen size is the first field of the frame state it points to.
    let (screen_size, corner) = unsafe {
        let frame = <u32 as ShaderData>::read_unchecked(words, record) as usize;
        (
            <Vec2 as ShaderData>::read_unchecked(words, frame),
            <Vec2 as ShaderData>::read_unchecked(words, record + 1 + (index as usize & 3) * 2),
        )
    };
    let ndc = corner / screen_size * 2.0 - 1.0;
    *position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
    *uv = vec2((index & 1) as f32, (index >> 1) as f32);
    *out_draw = draw;
}

/// Reads a draw's frame state and payload, and the fragment's coordinates.
#[doc(hidden)]
pub fn fragment_input<P: Program, D: ShaderData>(
    words: &[u32],
    draw: u32,
    coord: Vec4,
    uv: Vec2,
) -> (ShaderFrame<P>, D, Fragment) {
    let record = draw as usize * (D::WORDS + 1);
    // SAFETY: The recorder aligns complete payloads and their frame offsets to this stride.
    let (frame, data) = unsafe {
        let frame = <u32 as ShaderData>::read_unchecked(words, record) as usize;
        (FrameState::read_unchecked(words, frame), D::read_unchecked(words, record + 1))
    };
    let pixel = vec2(coord.x, coord.y) * frame.pixel_scale;
    (frame, data, Fragment { pixel, uv, sample: (), coverage: 1.0 })
}

/// Runs a shader's closures for one pixel, then discards it if uncovered, and otherwise premultiplies by coverage
/// unless `raw` output was requested.
#[doc(hidden)]
pub fn fragment<P: Program, S: Primitive>(
    frame: ShaderFrame<P>,
    fragment: Fragment,
    stage: impl FnOnce(ShaderFrame<P>) -> S,
    shade: impl FnOnce(ShaderFrame<P>, Fragment<S::Sample>) -> Vec4,
    raw: bool,
    out: &mut Vec4,
) {
    let (sample, coverage) = stage(frame).sample(fragment);
    let color = shade(frame, fragment.sampled(sample, coverage));
    if coverage <= 0.0 {
        kill();
    }
    *out = if raw {
        color
    } else {
        let alpha = color.w * coverage;
        (color.truncate() * alpha).extend(alpha)
    };
}

/// A captured slice, read from the frame's words without a panic path.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ShaderSlice<'a, T: ShaderData> {
    words: &'a [u32],
    range: [u32; 2],
    marker: PhantomData<T>,
}

impl<'a, T: ShaderData> ShaderSlice<'a, T> {
    pub const fn from_words(words: &'a [u32], range: [u32; 2]) -> Self {
        Self { words, range, marker: PhantomData }
    }

    pub const fn len(self) -> usize {
        self.range[1] as usize
    }

    pub fn load(self, index: usize) -> T {
        if index >= self.len() { T::ZERO } else { T::read(self.words, self.range[0] as usize + index * T::WORDS) }
    }
}
