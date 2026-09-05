use crate::render::{Globals, TextFragment};
use isthmus::{
    Float as _, Quad, Sdf,
    glam::{UVec2, Vec2, Vec3, Vec4, uvec2, vec2},
};

/// Where the drop shadow fades below the fragment kill threshold, plus an AA pixel.
const SHADOW_REACH: f32 = 18.0;
/// Smallest coverage worth shading; analytic shadows never reach exact zero.
pub const VISIBLE_ALPHA: f32 = 1.0 / 1024.0;

pub fn sample_pill(quad: Quad, pixel: Vec2, globals: Globals, time: f32) -> SurfaceSample {
    let size = quad.size;
    cantus_surface(quad, pixel, globals, time, |point| {
        Sdf::capsule(quad.local(point), (size.x - size.y) * 0.5, size.y * 0.5)
    })
}

/// Samples arbitrary Cantus SDF geometry with interaction, refraction and shadowing.
pub fn cantus_surface(
    quad: Quad,
    pixel: Vec2,
    globals: Globals,
    time: f32,
    shape: impl Fn(Vec2) -> Sdf,
) -> SurfaceSample {
    let distance = shape(pixel).distance;
    let mouse_distance = if globals.pressure > 0.0 { shape(globals.pointer).distance } else { 1.0 };
    let mouse_mask = mouse_distance.smoothstep(0.5, -0.5);
    let interaction = interaction(pixel, globals, time, mouse_mask);
    SurfaceSample::new(quad.local(pixel) + quad.size * 0.5, quad.size, Sdf::new(distance), interaction)
}

#[derive(Clone, Copy)]
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
    fn new(local: Vec2, size: Vec2, shape: Sdf, interaction: Interaction) -> Self {
        let mut surface = Self {
            bulge: interaction.bulge,
            refraction: interaction.refraction,
            local,
            refracted: local,
            size,
            distance: shape.distance,
            mask: 0.0,
            alpha: 0.0,
            ripple: interaction.ripple,
            flash: interaction.flash,
        };
        surface.resolve(shape);
        surface
    }

    fn resolve(&mut self, shape: Sdf) {
        self.distance = shape.distance - self.bulge * 0.5;
        self.mask = Sdf::new(self.distance).fill();
        let shadow = (-self.distance.max(0.0) * 0.3).exp() * 0.16;
        self.alpha = self.mask.max(shadow);
        let uv = self.local / self.size;
        let edge_lens =
            (uv.clamp(Vec2::ZERO, Vec2::ONE) - 0.5) * (1.0 + self.distance.min(0.0) / 120.0).clamp(0.0, 0.6) * 0.08;
        self.refracted = (uv - edge_lens) * self.size - self.refraction;
    }

    /// Resolves child geometry with this surface's existing interaction and optics.
    pub fn layer(mut self, shape: Sdf) -> Self {
        self.resolve(shape);
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

    pub fn text(self, text: &TextFragment) -> Vec4 {
        text.color(text.alpha_at(self.content_point(text.pixel)) * self.mask)
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

/// Maximum pixels a pill can cover beyond its bounds: shadow plus held-pointer and four ripples.
pub const PILL_MARGIN: f32 = SHADOW_REACH + (2.0 * 8.0 + 4.0 * 11.0) * 0.5;

/// 1.0 when positive, else 0.0; core lowers `f32::from(bool)` through `u8`, which costs an extra conversion.
pub fn presence(value: f32) -> f32 {
    if value > 0.0 { 1.0 } else { 0.0 }
}

#[derive(Clone, Copy)]
struct Interaction {
    bulge: f32,
    refraction: Vec2,
    ripple: Vec2,
    flash: f32,
}

fn interaction(pixel: Vec2, globals: Globals, time: f32, mouse_mask: f32) -> Interaction {
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
    let mouse_lift =
        if globals.pressure > 0.0 { pointer_offset.length().smoothstep(150.0, 0.0) * globals.pressure } else { 0.0 };
    Interaction {
        bulge: mouse_lift * mouse_mask * 8.0 + ripple.length() * 22.0,
        refraction: pointer_offset * mouse_lift * mouse_mask * 0.035 + ripple * 3.0,
        ripple,
        flash: ripple_flash,
    }
}
