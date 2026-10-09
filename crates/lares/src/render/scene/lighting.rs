//! Linear-light GGX / Smith / Schlick shading and an analytic room environment.
use super::materials::Surface;
use core::f32::consts::PI;
use isthmus::prelude::*;
pub(super) const RADIANCE: Vec3 = vec3(3.6, 3.35, 2.95);
pub(super) const KEY: Vec3 = vec3(-0.408_248, -0.408_248, 0.816_497);
fn fresnel(f0: Vec3, cosine: f32) -> Vec3 {
    let x = 1.0 - cosine;
    let x5 = x * x * x * x * x;
    f0 + (Vec3::ONE - f0) * x5
}
fn ggx(roughness: f32, no_h: f32, no_v: f32, no_l: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = no_h * no_h * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * d * d).max_num(0.00001);
    let lambda_v = no_l * (no_v * no_v * (1.0 - a2) + a2).sqrt();
    let lambda_l = no_v * (no_l * no_l * (1.0 - a2) + a2).sqrt();
    distribution * 0.5 / (lambda_v + lambda_l).max_num(0.0001)
}
fn environment(direction: Vec3, roughness: f32) -> Vec3 {
    let sky = vec3(0.62, 0.71, 0.85);
    let ceiling = vec3(0.78, 0.73, 0.64);
    let base = vec3(0.18, 0.14, 0.10).lerp(ceiling, direction.z * 0.5 + 0.5);
    // Broad analytic windows: blur grows with roughness, with constant average energy.
    let window = direction.dot(vec3(-0.8, -0.2, 0.56).normalize()).max_num(0.0);
    let power = 2.0 + (1.0 - roughness) * 26.0;
    base + sky * window.powf(power) * ((power + 1.0) * 0.04)
}
#[inline(never)]
pub(super) fn shade(s: Surface, n: Vec3, view: Vec3, visibility: f32, indirect: Vec3) -> Vec3 {
    let no_v = n.dot(view).max_num(0.001);
    let f0 = Vec3::splat(0.04).lerp(s.albedo, s.metallic);
    let mut color = Vec3::ZERO;
    {
        let (light, radiance, shadow) = (KEY, RADIANCE, visibility);
        let half = (view + light).normalize();
        let no_l = n.dot(light).max_num(0.0);
        let no_h = n.dot(half).max_num(0.0);
        let lo_h = light.dot(half).max_num(0.0);
        let f = fresnel(f0, lo_h);
        let diffuse = (Vec3::ONE - f) * (1.0 - s.metallic) * s.albedo / PI;
        let specular = f * ggx(s.roughness, no_h, no_v, no_l);
        let sheen = s.albedo * s.sheen * (1.0 - lo_h).powf(5.0) / PI;
        let coat_f = fresnel(Vec3::splat(0.04), lo_h) * s.coat;
        let coat = coat_f * ggx(0.22, no_h, no_v, no_l);
        color += ((diffuse + specular + sheen) * (Vec3::ONE - coat_f) + coat) * radiance * no_l * shadow;
    }
    let f = fresnel(f0, no_v);
    let reflected = 2.0 * n.dot(view) * n - view;
    let diffuse_env = indirect * s.albedo * (1.0 - s.metallic) * (Vec3::ONE - f);
    // Analytic prefiltered environment and integrated specular approximation.
    let r = vec4(-1.0, -0.0275, -0.572, 0.022) * s.roughness + vec4(1.0, 0.0425, 1.04, -0.04);
    let a004 = (r.x * r.x).min_num((-9.28 * no_v).exp2()) * r.x + r.y;
    let ab = vec2(-1.04, 1.04) * a004 + vec2(r.z, r.w);
    let specular_env = environment(reflected, s.roughness) * (f0 * ab.x + Vec3::splat(ab.y)).max_num(Vec3::ZERO);
    let coat_env = fresnel(Vec3::splat(0.04), no_v) * s.coat;
    color +=
        ((diffuse_env + specular_env) * (Vec3::ONE - coat_env) + environment(reflected, 0.22) * coat_env) * s.occlusion;
    color
}
#[inline(never)]
pub(super) fn display(color: Vec3) -> Vec3 {
    let x = color * 0.95;
    let mapped = ((x * (x * 2.51 + 0.03)) / (x * (x * 2.43 + 0.59) + 0.14)).clamp(Vec3::ZERO, Vec3::ONE);
    // Output is display-referred; the canvas uses an unorm target.
    mapped.powf(1.0 / 2.2)
}
