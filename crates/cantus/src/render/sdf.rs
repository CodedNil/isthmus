use crate::render::Program;
use isthmus::{
    Float as _, Primitive, Quad, ShaderFrame,
    glam::{UVec2, Vec2, Vec3, Vec4, uvec2, vec2},
    surface,
};
use isthmus_sdf::{Outlined, Sample, Shape};

const SHADOW_OPACITY: f32 = 0.16;
const SHADOW_DECAY: f32 = 0.3;
const POINTER_REACH: f32 = 150.0;
const POINTER_REFRACTION: f32 = 0.035;
const RIPPLE_REFRACTION: f32 = 3.0;
const POINTER_BULGE: f32 = 8.0;
const RIPPLE_BULGE: f32 = 22.0;
const LENS_REACH: f32 = 3.0;
/// Smallest coverage worth shading; analytic shadows never reach exact zero.
pub const VISIBLE_ALPHA: f32 = 1.0 / 1024.0;

pub fn deform(
    shape: Shape<impl Fn(Vec2) -> f32 + Copy>,
    frame: ShaderFrame<Program>,
) -> impl Primitive<Program, Outputs = (), Sample = DeformedSample> {
    let (bulge_reach, _) = reach(shape, frame);
    let reach = (SHADOW_OPACITY / VISIBLE_ALPHA).ln() / SHADOW_DECAY;
    surface(shape.bounds(reach + bulge_reach), move |point| {
        let mut sample = sample_deformation(shape, frame, point);
        sample.sdf.coverage = sample.sdf.band(-f32::MAX..reach)
            * sample.sdf.fill().max((-sample.sdf.distance.max(0.0) * SHADOW_DECAY).exp() * SHADOW_OPACITY);
        (sample, sample.sdf.coverage)
    })
}

/// Supplies displaced coordinates without clipping content to the parent.
pub fn refract(
    parent: Shape<impl Fn(Vec2) -> f32 + Copy>,
    frame: ShaderFrame<Program>,
    bounds: impl Into<Option<Quad>>,
) -> impl Primitive<Program, Outputs = (), Sample = DeformedSample> {
    let (_, refraction_reach) = reach(parent, frame);
    surface(bounds.into().map(|bounds| bounds.expanded(refraction_reach)), move |point| {
        let sample = sample_deformation(parent, frame, point);
        (sample, 1.0)
    })
}

/// Applies the parent's edge lensing and bulge to an independently bounded layer.
pub fn lens<F: Fn(Vec2) -> f32 + Copy>(
    parent: Shape<impl Fn(Vec2) -> f32 + Copy>,
    frame: ShaderFrame<Program>,
    shape: impl Into<Outlined<F>>,
) -> impl Primitive<Program, Outputs = (), Sample = DeformedSample> {
    let outlined = shape.into();
    let (bulge_reach, refraction_reach) = reach(parent, frame);
    surface(outlined.bounds(refraction_reach + bulge_reach + LENS_REACH), move |point| {
        let mut parent = sample_deformation(parent, frame, point);
        parent.sdf = outlined.sample(outlined.shape.distance_at(parent.refracted) - parent.bulge * 0.5);
        (parent, parent.sdf.coverage)
    })
}

fn reach(shape: Shape<impl Fn(Vec2) -> f32 + Copy>, frame: ShaderFrame<Program>) -> (f32, f32) {
    let pressure = frame.globals.pressure * shape.distance_at(frame.globals.pointer).smoothstep(0.5, -0.5);
    let mut ripple = 0.0;
    for index in 0..frame.globals.ripples.len() {
        let pulse = frame.globals.ripples[index];
        if pulse.start_time > 0.0 {
            ripple += (1.0 - ((frame.time - pulse.start_time) * 1.2).saturate()).powi(2) * 0.5;
        }
    }
    let bulge_reach = (pressure * POINTER_BULGE + ripple * RIPPLE_BULGE) * 0.5;
    let refraction_reach = POINTER_REACH * pressure * POINTER_REFRACTION + ripple * RIPPLE_REFRACTION;
    (bulge_reach, refraction_reach)
}

pub fn sample_deformation(
    shape: Shape<impl Fn(Vec2) -> f32 + Copy>,
    frame: ShaderFrame<Program>,
    point: Vec2,
) -> DeformedSample {
    let pressure = frame.globals.pressure * shape.distance_at(frame.globals.pointer).smoothstep(0.5, -0.5);
    let mut ripple = Vec2::ZERO;
    let mut flash = 0.0;
    // Rust-GPU cannot lower this slice iterator without a pointer-to-integer conversion.
    for index in 0..frame.globals.ripples.len() {
        let pulse = frame.globals.ripples[index];
        let progress = ((frame.time - pulse.start_time) * 1.2).saturate();
        if pulse.start_time > 0.0 && progress < 1.0 {
            let offset = point - pulse.origin;
            let distance = offset.length();
            let direction = if distance > 0.0001 { offset / distance } else { Vec2::ZERO };
            let wave = (distance - progress * 600.0).abs().smoothstep(80.0, 0.0) * (1.0 - progress);
            ripple += direction * wave * (1.0 - progress) * 0.5;
            flash = (flash + wave * 0.5).min(1.0);
        }
    }
    let pointer_offset = point - frame.globals.pointer;
    let mouse_lift = pointer_offset.length().smoothstep(POINTER_REACH, 0.0) * pressure;
    let bulge = mouse_lift * POINTER_BULGE + ripple.length() * RIPPLE_BULGE;
    let content = point - pointer_offset * mouse_lift * POINTER_REFRACTION - ripple * RIPPLE_REFRACTION;
    let sdf = Sample::new(shape.distance_at(point) - bulge * 0.5, 0.0);
    let lens = (1.0 + sdf.distance.min(0.0) / 12.0).saturate() * LENS_REACH;
    let refracted = content - sdf.gradient / sdf.gradient.length().max(f32::MIN_POSITIVE) * lens;
    DeformedSample { content, refracted, sdf, bulge, ripple, flash }
}

#[derive(Clone, Copy)]
pub struct DeformedSample {
    /// Displaced screen position without edge lensing.
    pub content: Vec2,
    /// Displaced screen position including edge lensing.
    pub refracted: Vec2,
    pub sdf: Sample,
    pub bulge: f32,
    pub ripple: Vec2,
    flash: f32,
}

impl DeformedSample {
    /// Glass appearance; coverage is applied by the primitive after shading.
    pub fn glass(self, mut color: Vec3) -> Vec4 {
        let facing = (self.sdf.gradient / self.sdf.gradient.length().max(f32::MIN_POSITIVE)).dot(vec2(-0.6, -0.8));
        let inward = (-self.sdf.distance).max(0.0);
        let sheen = (1.0 - inward / 6.0).saturate().powi(2)
            * (0.10 + 0.30 * facing.max(0.0).powi(4) + 0.10 * (-facing).max(0.0).powi(4))
            + (1.0 - inward / 12.0).saturate().powi(3) * 0.02;
        color = color.lerp(Vec3::ONE, sheen);
        color = color.lerp(color * 1.5 + 0.1, self.flash);
        (color * (self.sdf.fill() / self.sdf.coverage.max(f32::MIN_POSITIVE))).extend(1.0)
    }
}

/// Core 2-lane avalanche mixer for hash functions
pub fn avalanche(mut value: UVec2) -> UVec2 {
    value = value.wrapping_mul(UVec2::splat(1_664_525)).wrapping_add(UVec2::splat(1_013_904_223));
    value.x = value.x.wrapping_add(value.y.wrapping_mul(1_664_525));
    value.y = value.y.wrapping_add(value.x.wrapping_mul(1_664_525));
    value ^= value >> 16;
    value.x = value.x.wrapping_add(value.y.wrapping_mul(1_664_525));
    value.y = value.y.wrapping_add(value.x.wrapping_mul(1_664_525));
    value ^= value >> 16;
    value
}

pub fn hash(p: Vec2) -> Vec2 {
    let value = avalanche(uvec2(p.x as i32 as u32, p.y as i32 as u32));
    vec2(value.x as f32, value.y as f32) * 2.328_306_4e-10
}

// Keep this shared to avoid duplicating its noise graph in every shader entry point.
#[inline(never)]
pub fn simplex_noise(p: Vec2) -> f32 {
    const K1: f32 = 0.366_025_42;
    const K2: f32 = 0.211_324_87;
    let cell = (p + (p.x + p.y) * K1).floor();
    let a = p - cell + (cell.x + cell.y) * K2;
    let corner = if a.x > a.y { vec2(1.0, 0.0) } else { vec2(0.0, 1.0) };
    let b = a - corner + K2;
    let c = a - 1.0 + 2.0 * K2;
    let contribution = |offset: Vec2, point: Vec2| {
        let falloff = (0.5 - point.length_squared()).max(0.0);
        falloff * falloff * falloff * falloff * point.dot(hash(cell + offset) * 2.0 - 1.0)
    };
    70.0 * (contribution(Vec2::ZERO, a) + contribution(corner, b) + contribution(Vec2::ONE, c))
}

pub fn fbm(mut p: Vec2) -> f32 {
    let mut density = 0.0;
    let mut amplitude = 0.5;
    for _ in 0..4 {
        density += simplex_noise(p) * amplitude;
        p = vec2(p.x * 1.6 + p.y * 1.2, p.y * 1.6 - p.x * 1.2);
        amplitude *= 0.5;
    }
    0.5 + density * 0.5
}

/// 1.0 when positive, else 0.0; core lowers `f32::from(bool)` through `u8`, which costs an extra conversion.
pub fn presence(value: f32) -> f32 {
    if value > 0.0 { 1.0 } else { 0.0 }
}
