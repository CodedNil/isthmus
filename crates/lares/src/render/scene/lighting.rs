//! Linear-light GGX / Smith / Schlick shading and an analytic room environment.
use super::materials::Surface;
use crate::render::Sun;
use core::f32::consts::PI;
use isthmus::prelude::*;
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
/// Anisotropic GGX for brushed metal and the longitudinal oak finish.
fn microfacet(s: Surface, n: Vec3, view: Vec3, light: Vec3, half: Vec3) -> f32 {
    let no_v = n.dot(view).max_num(0.001);
    let no_l = n.dot(light).max_num(0.0);
    let no_h = n.dot(half).max_num(0.0);
    if s.anisotropy <= 0.0 {
        return ggx(s.roughness, no_h, no_v, no_l);
    }
    let projected_tangent = s.tangent - n * s.tangent.dot(n);
    let tangent = if projected_tangent.length_squared() > 1e-6 {
        projected_tangent.normalize()
    } else if n.z.abs() < 0.9 {
        n.cross(Vec3::Z).normalize()
    } else {
        n.cross(Vec3::Y).normalize()
    };
    let bitangent = n.cross(tangent);
    let alpha = s.roughness * s.roughness;
    let aspect = (1.0 - s.anisotropy * 0.9).sqrt();
    let ax = alpha / aspect;
    let ay = alpha * aspect;
    let projected = vec3(tangent.dot(half) / ax, bitangent.dot(half) / ay, no_h);
    let denominator = projected.length_squared();
    let distribution = 1.0 / (PI * ax * ay * denominator * denominator).max_num(1e-6);
    let v = vec3(ax * tangent.dot(view), ay * bitangent.dot(view), no_v).length();
    let l = vec3(ax * tangent.dot(light), ay * bitangent.dot(light), no_l).length();
    distribution * 0.5 / (no_l * v + no_v * l).max_num(0.0001)
}
fn environment(direction: Vec3, roughness: f32) -> Vec3 {
    let sky = vec3(0.62, 0.71, 0.85);
    let ceiling = vec3(0.72, 0.73, 0.74);
    let base = vec3(0.18, 0.14, 0.10).lerp(ceiling, direction.z * 0.5 + 0.5);
    // Broad analytic windows: blur grows with roughness, with constant average energy.
    let window = direction.dot(vec3(-0.8, -0.2, 0.56).normalize()).max_num(0.0);
    let power = 2.0 + (1.0 - roughness) * 26.0;
    base + sky * window.powf(power) * ((power + 1.0) * 0.04)
}
#[inline(never)]
pub(super) fn direct(s: Surface, n: Vec3, view: Vec3, light: Vec3, radiance: Vec3) -> Vec3 {
    let no_v = n.dot(view).max_num(0.001);
    let f0 = Vec3::splat(0.04).lerp(s.albedo, s.metallic);
    let half = (view + light).normalize();
    let no_l = n.dot(light).max_num(0.0);
    let no_h = n.dot(half).max_num(0.0);
    let lo_h = light.dot(half).max_num(0.0);
    let f = fresnel(f0, lo_h) * s.specular;
    let diffuse = (Vec3::ONE - f) * (1.0 - s.metallic) * s.albedo / PI;
    let specular = f * microfacet(s, n, view, light, half);
    let sheen = s.albedo * s.sheen * ((1.0 - lo_h) * (1.0 - lo_h) * (1.0 - lo_h) * (1.0 - lo_h) * (1.0 - lo_h)) / PI;
    let coat_f = fresnel(Vec3::splat(0.04), lo_h) * s.coat;
    let coat = if s.coat > 0.0 { coat_f * ggx(s.coat_roughness, no_h, no_v, no_l) } else { Vec3::ZERO };
    // Local yarn-horizon masking, reciprocal in view/light directions. Large
    // pile clumps still cast real SDF shadows; this covers sub-texel fibres.
    let fiber_visibility = if s.fiber_depth > 0.0 {
        let elevation = |direction: Vec3| {
            let cosine = s.tangent.dot(direction).abs().max_num(0.001);
            cosine / (cosine + s.fiber_depth * (1.0 - cosine) * 0.30)
        };
        elevation(light) * elevation(view)
    } else {
        1.0
    };
    ((diffuse + specular + sheen) * (Vec3::ONE - coat_f) + coat) * radiance * (no_l * fiber_visibility)
}
#[inline(never)]
pub(super) fn shade(
    s: Surface,
    n: Vec3,
    view: Vec3,
    visibility: f32,
    indirect: Vec3,
    sun: Sun,
    reflection: Vec4,
) -> Vec3 {
    let no_v = n.dot(view).max_num(0.001);
    let f0 = Vec3::splat(0.04).lerp(s.albedo, s.metallic);
    let mut color =
        if visibility > 0.0 { direct(s, n, view, sun.direction, sun.radiance) * visibility } else { Vec3::ZERO };
    let f = fresnel(f0, no_v) * s.specular;
    let reflected = 2.0 * n.dot(view) * n - view;
    let diffuse_env = indirect * s.albedo * (1.0 - s.metallic) * (Vec3::ONE - f);
    // Analytic prefiltered environment and integrated specular approximation.
    let r = vec4(-1.0, -0.0275, -0.572, 0.022) * s.roughness + vec4(1.0, 0.0425, 1.04, -0.04);
    let a004 = (r.x * r.x).min_num((-9.28 * no_v).exp2()) * r.x + r.y;
    let ab = vec2(-1.04, 1.04) * a004 + vec2(r.z, r.w);
    // Interior reflections must share the locally occluded transport field.
    // Global sky here bypassed the roof/walls and bleached dark worktops and
    // foliage when daylight strength increased.
    let environment_scale = indirect * 1.5;
    let filtered_environment = |roughness: f32| {
        let approximate = environment(reflected, roughness) * environment_scale;
        approximate.lerp(reflection.truncate(), reflection.w * (1.0 - roughness).powi(2))
    };
    let specular_env =
        filtered_environment(s.roughness) * (f0 * ab.x + Vec3::splat(ab.y)).max_num(Vec3::ZERO) * s.specular;
    // Environment highlights follow local visibility instead of shining equally
    // inside cupboards and beside windows. The bounce field is already cached.
    let specular_occlusion = (indirect.element_sum() * 0.9).clamp(0.18, 1.0);
    let edge = 1.0 - no_v;
    let sheen_env = indirect * s.albedo * (s.sheen * (edge * edge * edge) * 0.35);
    let coat_env = fresnel(Vec3::splat(0.04), no_v) * s.coat;
    let coat =
        if s.coat > 0.0 { filtered_environment(s.coat_roughness) * coat_env * specular_occlusion } else { Vec3::ZERO };
    color +=
        ((diffuse_env + sheen_env + specular_env * specular_occlusion) * (Vec3::ONE - coat_env) + coat) * s.occlusion;
    color
}
#[inline(never)]
pub(super) fn display(color: Vec3, exposure: f32) -> Vec3 {
    let x = color * exposure;
    let mapped = ((x * (x * 2.51 + 0.03)) / (x * (x * 2.43 + 0.59) + 0.14)).clamp(Vec3::ZERO, Vec3::ONE);
    // Output is display-referred; the canvas uses an unorm target.
    mapped.powf(1.0 / 2.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occluded_indoor_reflections_do_not_receive_unfiltered_global_sky() {
        let material =
            Surface { albedo: Vec3::splat(0.05), metallic: 1.0, roughness: 0.45, occlusion: 1.0, ..Surface::default() };
        let sun = Sun { direction: Vec3::Z, radiance: Vec3::ZERO, sky: Vec3::ONE };
        let local = Vec3::splat(0.2);
        let first = shade(material, Vec3::X, Vec3::X, 0.0, local, sun, Vec4::ZERO);
        let bright_sky = Sun { sky: Vec3::splat(1000.0), ..sun };
        let second = shade(material, Vec3::X, Vec3::X, 0.0, local, bright_sky, Vec4::ZERO);
        assert!(
            (first - second).abs().max_element() < 0.00001,
            "the roof's cached occlusion must also apply to reflections"
        );
    }
    #[test]
    fn microfacet_reflection_is_finite_and_reciprocal_across_finishes_and_tangent_frames() {
        for normal in [Vec3::Z, Vec3::X, vec3(1.0, 1.0, 0.0).normalize()] {
            let axis = if normal.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
            for tangent in [normal, axis, normal.cross(axis)] {
                for roughness in [0.15, 0.4, 0.7, 1.0] {
                    for anisotropy in [0.0, 0.38, 0.7] {
                        for fiber_depth in [0.0, 0.3, 0.8] {
                            let material = Surface {
                                albedo: Vec3::splat(0.4),
                                roughness,
                                anisotropy,
                                tangent,
                                fiber_depth,
                                coat: 0.2,
                                coat_roughness: 0.45,
                                occlusion: 1.0,
                                ..Surface::default()
                            };
                            let view = (normal + axis * 0.4).normalize();
                            let light = (normal - axis * 0.25).normalize();
                            let first = direct(material, normal, view, light, Vec3::ONE) / normal.dot(light);
                            let reversed = direct(material, normal, light, view, Vec3::ONE) / normal.dot(view);
                            assert!(first.is_finite() && first.min_element() >= 0.0);
                            assert!((first - reversed).abs().max_element() < 0.0001);
                        }
                    }
                }
            }
        }
    }
}
