use crate::render::{Fragment, Globals, HELD_PRESSURE, RIPPLE_COUNT};
use isthmus::{
    Float as _, Quad, Text,
    geometry::{
        FragmentGeometry,
        effect::Effect,
        sdf::{self, SdfShape, Shape},
    },
    glam::{UVec2, Vec2, Vec3, Vec4, uvec2, vec2},
};

const SHADOW_OPACITY: f32 = 0.16;
const SHADOW_DECAY: f32 = 0.3;
const POINTER_REACH: f32 = 150.0;
const POINTER_REFRACTION: f32 = 0.035;
const RIPPLE_REFRACTION: f32 = 3.0;
const POINTER_BULGE: f32 = 8.0;
const RIPPLE_BULGE: f32 = 22.0;
/// Smallest coverage worth shading; analytic shadows never reach exact zero.
pub const VISIBLE_ALPHA: f32 = 1.0 / 1024.0;

#[derive(Clone, Copy)]
pub struct Refraction;

impl Effect for Refraction {
    fn displacement(self) -> f32 {
        // The radial falloff bounds u * (1 - smoothstep(0, 1, u)) below 0.26.
        POINTER_REACH * 0.26 * HELD_PRESSURE * POINTER_REFRACTION + RIPPLE_COUNT as f32 * 0.5 * RIPPLE_REFRACTION
    }
}

#[derive(Clone, Copy)]
pub struct Glass;

impl Effect for Glass {
    fn outset(self) -> f32 {
        let shadow = (SHADOW_OPACITY / VISIBLE_ALPHA).ln() / SHADOW_DECAY;
        shadow + (HELD_PRESSURE * POINTER_BULGE + RIPPLE_COUNT as f32 * 0.5 * RIPPLE_BULGE) * 0.5
    }
}

pub fn sample_pill<G: for<'a> FragmentGeometry<'a>>(quad: Quad, fragment: &Fragment<G>) -> SurfaceSample {
    Glass::sample(quad, Shape::pill(quad).shape, fragment)
}

impl Glass {
    /// Samples arbitrary Cantus SDF geometry with interaction, refraction and shadowing.
    pub fn sample<G: for<'a> FragmentGeometry<'a>>(
        quad: Quad,
        shape: impl SdfShape,
        fragment: &Fragment<G>,
    ) -> SurfaceSample {
        let globals = fragment.globals;
        let mouse_distance = if globals.pressure > 0.0 { shape.distance_at(globals.pointer) } else { 1.0 };
        let mouse_mask = mouse_distance.smoothstep(0.5, -0.5);
        SurfaceSample {
            local: quad.local(fragment.pixel) + quad.size * 0.5,
            size: quad.size,
            ..interaction(fragment.pixel, globals, fragment.time, mouse_mask)
        }
        .layer(shape.distance_at(fragment.pixel))
    }
}

#[derive(Clone, Copy, Default)]
pub struct SurfaceSample {
    bulge: f32,
    refraction: Vec2,
    /// Unmodified top-left-relative surface coordinates.
    pub local: Vec2,
    /// Ripple- and edge-refracted top-left-relative coordinates.
    pub refracted: Vec2,
    pub size: Vec2,
    pub distance: f32,
    pub mask: f32,
    pub alpha: f32,
    pub ripple: Vec2,
    flash: f32,
}

impl SurfaceSample {
    /// Resolves a contour with this surface's existing interaction and material coordinates.
    pub fn layer(mut self, distance: f32) -> Self {
        self.distance = distance - self.bulge * 0.5;
        self.mask = sdf::fill(self.distance);
        let shadow = (-self.distance.max(0.0) * SHADOW_DECAY).exp() * SHADOW_OPACITY;
        self.alpha = self.mask.max(shadow);
        let uv = self.local / self.size;
        let edge_lens =
            (uv.clamp(Vec2::ZERO, Vec2::ONE) - 0.5) * (1.0 + self.distance.min(0.0) / 120.0).clamp(0.0, 0.6) * 0.08;
        self.refracted = (uv - edge_lens) * self.size - self.refraction;
        self
    }

    pub fn uv(self) -> Vec2 {
        self.local / self.size
    }

    /// Maps a fragment coordinate through this surface's optical displacement.
    pub fn refract(self, pixel: Vec2) -> Vec2 {
        pixel + self.displacement()
    }

    /// Moves content with interaction while avoiding edge distortion of text baselines.
    pub fn content_point(self, pixel: Vec2) -> Vec2 {
        pixel - self.refraction
    }

    pub fn text(self, text: &Fragment<Text>) -> Vec4 {
        text.color(text.fill_at(self.content_point(text.pixel)) * self.mask)
    }

    pub fn displacement(self) -> Vec2 {
        self.refracted - self.local
    }

    pub const fn bulge(self) -> f32 {
        self.bulge
    }

    fn shade(self, mut color: Vec3) -> Vec3 {
        color += color.lerp(Vec3::ONE, 0.32) * self.distance.smoothstep(5.0, -3.0) * 0.14;
        color.lerp(color * 1.5 + 0.1, self.flash)
    }

    /// Straight-alpha surface color including its outer shadow.
    pub fn color(self, color: Vec3) -> Vec4 {
        // Preserve the outer shadow without applying straight-alpha coverage twice.
        (self.shade(color) * self.mask / self.alpha.max(0.0001)).extend(self.alpha)
    }

    /// Straight-alpha surface color clipped to the shape without a shadow.
    pub fn fill_color(self, color: Vec3) -> Vec4 {
        self.shade(color).extend(self.mask)
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

pub fn cloud_mass(p: Vec2, scale: f32, time: f32) -> f32 {
    fbm(p / scale * 0.14 + vec2(time * 0.012, 6.1))
}

/// 1.0 when positive, else 0.0; core lowers `f32::from(bool)` through `u8`, which costs an extra conversion.
pub fn presence(value: f32) -> f32 {
    if value > 0.0 { 1.0 } else { 0.0 }
}

fn interaction(pixel: Vec2, globals: Globals, time: f32, mouse_mask: f32) -> SurfaceSample {
    let mut ripple = Vec2::ZERO;
    let mut ripple_flash = 0.0;
    // Rust-GPU cannot lower this slice iterator without a pointer-to-integer conversion.
    for index in 0..globals.ripples.len() {
        let pulse = globals.ripples[index];
        let progress = ((time - pulse.start_time) * 1.2).saturate();
        // Uniform across the draw, so expired slots skip all per-pixel distance work.
        if pulse.start_time > 0.0 && progress < 1.0 {
            let offset = pixel - pulse.origin;
            let distance = offset.length();
            // Avoid normalize_or_zero's Naga-invalid infinity constant and duplicate square root.
            let direction = if distance > 0.0001 { offset / distance } else { Vec2::ZERO };
            let wave = (distance - progress * 600.0).abs().smoothstep(80.0, 0.0) * (1.0 - progress);
            ripple += direction * wave * (1.0 - progress) * 0.5;
            ripple_flash = (ripple_flash + wave * 0.5).min(1.0);
        }
    }

    let pointer_offset = pixel - globals.pointer;
    let mouse_lift = if globals.pressure > 0.0 {
        pointer_offset.length().smoothstep(POINTER_REACH, 0.0) * globals.pressure
    } else {
        0.0
    };
    SurfaceSample {
        bulge: mouse_lift * mouse_mask * POINTER_BULGE + ripple.length() * RIPPLE_BULGE,
        refraction: pointer_offset * mouse_lift * mouse_mask * POINTER_REFRACTION + ripple * RIPPLE_REFRACTION,
        ripple,
        flash: ripple_flash,
        ..Default::default()
    }
}
