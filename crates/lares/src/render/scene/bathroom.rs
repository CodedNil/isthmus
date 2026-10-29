use super::{Kind, Material, Sample};
use isthmus::prelude::*;

#[inline(never)]
pub(super) fn model<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let distance = match kind {
        Kind::Bath => {
            let shell = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.05);
            let hollow = super::scaled_box(local, size, vec3(0.0, 0.0, 0.15), vec3(0.78, 0.86, 0.9), 0.08);
            let p = local + Vec3::Z * size.z * 0.5;
            let tap_point = vec3(p.x.abs() - 0.11, p.y + size.y * 0.39, p.z - size.z);
            let taps = super::cylinder(tap_point - Vec3::Z * 0.035, 0.017, 0.075).min_num(super::capsule(
                tap_point,
                vec3(0.0, 0.0, 0.04),
                vec3(0.0, 0.08, 0.04),
                0.012,
            ));
            let overflow = super::cylinder(vec3(p.x, p.z - size.z * 0.73, p.y + size.y * 0.42), 0.023, 0.008);
            return S::new(shell.max_num(-hollow), || (Material::Ceramic, Vec3::splat(0.96)))
                .union(S::new(taps.min_num(overflow), || (Material::Metal, Vec3::splat(0.78))));
        }
        Kind::Shower => {
            let p = local + Vec3::Z * size.z * 0.5;
            let tray = super::rounded_box(p - Vec3::Z * 0.065, vec3(size.x, size.y, 0.13), 0.025);
            let basin = super::rounded_box(p - Vec3::Z * 0.13, vec3(size.x - 0.065, size.y - 0.065, 0.12), 0.025);
            let posts = super::rounded_box(
                vec3(p.x.abs() - size.x * 0.48, p.y.abs() - size.y * 0.48, p.z - size.z * 0.52),
                vec3(0.023, 0.023, size.z * 0.94),
                0.004,
            );
            let tracks = super::rounded_box(
                vec3(p.x, p.y.abs() - size.y * 0.48, (p.z - size.z * 0.52).abs() - size.z * 0.46),
                vec3(size.x, 0.030, 0.032),
                0.005,
            );
            let stile = super::rounded_box(
                p - vec3(0.0, size.y * 0.48, size.z * 0.52),
                vec3(0.014, 0.018, size.z * 0.93),
                0.003,
            );
            let handle = super::capsule(
                p,
                vec3(0.028, size.y * 0.5 + 0.024, 0.88),
                vec3(0.028, size.y * 0.5 + 0.024, 1.10),
                0.009,
            );
            let rail = super::capsule(p, vec3(0.0, -size.y * 0.45, 1.05), vec3(0.0, -size.y * 0.45, 1.83), 0.012);
            let mixer = super::capsule(p, vec3(-0.115, -size.y * 0.43, 1.01), vec3(0.115, -size.y * 0.43, 1.01), 0.023);
            let head = super::capsule(p, vec3(0.025, -size.y * 0.40, 1.70), vec3(0.025, -size.y * 0.36, 1.83), 0.032);
            let hose_point = vec3(p.x - 0.04, p.y + size.y * 0.39, p.z - 1.10);
            let hose =
                (vec2(hose_point.x * 1.6, hose_point.z).length() - 0.23).abs().max_num(hose_point.y.abs()) - 0.006;
            return S::new(tray.max_num(-basin), || (Material::Ceramic, Vec3::splat(0.95))).union(S::new(
                posts
                    .min_num(tracks)
                    .min_num(stile)
                    .min_num(handle)
                    .min_num(rail)
                    .min_num(mixer)
                    .min_num(head)
                    .min_num(hose),
                || (Material::Metal, Vec3::splat(0.78)),
            ));
        }
        Kind::Sink => {
            let p = local + Vec3::Z * size.z * 0.5;
            let oval = |q: Vec3, r: Vec3| ((q / r).length() - 1.0) * r.min_element();
            let rim = oval(p - vec3(0.0, 0.0, 0.77), vec3(size.x * 0.53, size.y * 0.51, 0.135));
            let bowl = oval(p - vec3(0.0, 0.025, 0.835), vec3(size.x * 0.43, size.y * 0.39, 0.13));
            let pedestal = super::rounded_box(p - vec3(0.0, -0.055, 0.36), vec3(0.16, 0.20, 0.70), 0.065);
            let tap = super::capsule(p, vec3(0.0, -size.y * 0.35, 0.81), vec3(0.0, -size.y * 0.35, 0.96), 0.022);
            let spout = super::capsule(p, vec3(0.0, -size.y * 0.35, 0.94), vec3(0.0, 0.025, 0.94), 0.016);
            let drain = super::cylinder(p - vec3(0.0, 0.025, 0.715), 0.025, 0.006);
            return S::new(super::blend(rim.max_num(-bowl), pedestal, 0.02), || (Material::Ceramic, Vec3::splat(0.96)))
                .union(S::new(tap.min_num(spout).min_num(drain), || (Material::Metal, Vec3::splat(0.78))));
        }
        Kind::TowelRail => {
            let posts = super::capsule(
                local,
                vec3(size.x * 0.45, 0.0, -size.z * 0.5),
                vec3(size.x * 0.45, 0.0, size.z * 0.5),
                0.016,
            )
            .min_num(super::capsule(
                local,
                vec3(-size.x * 0.45, 0.0, -size.z * 0.5),
                vec3(-size.x * 0.45, 0.0, size.z * 0.5),
                0.016,
            ));
            let repeated = vec3(local.x, local.y, local.z - (local.z / 0.09).round().clamp_num(-3.0, 3.0) * 0.09);
            let bars = super::capsule(repeated, vec3(-size.x * 0.45, 0.0, 0.0), vec3(size.x * 0.45, 0.0, 0.0), 0.010);
            let cloth = super::rounded_box(
                local - vec3(0.0, 0.035 + 0.008 * (local.x * 95.0).sin(), -0.05),
                vec3(size.x * 0.70, 0.028, size.z * 0.60),
                0.010,
            ) * 0.55;
            return S::new(posts.min_num(bars), || (Material::Metal, Vec3::splat(0.8)))
                .union(S::new(cloth, || (Material::Knit, vec3(0.43, 0.42, 0.44))));
        }
        Kind::BathMat => {
            return S::new(super::rounded_box(local, size, 0.009), || (Material::Knit, vec3(0.53, 0.51, 0.53)));
        }
        Kind::ExtractorVent => {
            let frame = super::rounded_box(local, size, 0.004);
            let radial = vec2(local.x, local.z).length();
            let grille = ((radial / 0.009).fract() - 0.5).abs() < 0.18 && radial < size.x * 0.39;
            return S::new(frame, || (Material::Paint, Vec3::splat(if grille { 0.09 } else { 0.92 })));
        }
        Kind::WasteBin => {
            let body = super::cylinder(local, size.x * 0.48, size.z);
            let lid = super::cylinder(local - Vec3::Z * (size.z * 0.5), size.x * 0.49, 0.012);
            return S::new(body, || (Material::Paint, Vec3::splat(0.09)))
                .union(S::new(lid, || (Material::Metal, Vec3::splat(0.65))));
        }
        Kind::Toilet => {
            // Close-coupled ceramic toilet. Work in metres relative to floor;
            // the pan is an oval shell, with an open seat and recessed bowl.
            let p = local + Vec3::Z * size.z * 0.5;
            let oval = |q: Vec3, radii: Vec3| ((q / radii).length() - 1.0) * radii.min_element();
            let shell = oval(p - vec3(0.0, 0.075, 0.34), vec3(size.x * 0.48, size.y * 0.43, 0.19));
            let cavity = oval(p - vec3(0.0, 0.09, 0.43), vec3(size.x * 0.36, size.y * 0.33, 0.17));
            let pan = shell.max_num(-cavity).max_num(p.z - 0.425);
            let pedestal = super::rounded_box(p - vec3(0.0, 0.005, 0.135), vec3(0.21, 0.32, 0.27), 0.055);
            let tank = super::rounded_box(p - vec3(0.0, -0.245, 0.59), vec3(size.x * 0.95, 0.15, 0.38), 0.025);
            let lid = super::rounded_box(p - vec3(0.0, -0.245, 0.79), vec3(size.x, 0.17, 0.025), 0.009);
            let seat_outer = oval(p - vec3(0.0, 0.075, 0.433), vec3(size.x * 0.49, size.y * 0.435, 0.018));
            let seat_hole = oval(vec3(p.x, p.y - 0.09, 0.0), vec3(size.x * 0.35, size.y * 0.32, 1.0));
            let seat = seat_outer.max_num(-seat_hole);
            let upright_lid = oval(p - vec3(0.0, -0.16, 0.64), vec3(size.x * 0.48, 0.025, 0.215));
            let upright_seat = oval(p - vec3(0.0, -0.12, 0.64), vec3(size.x * 0.49, 0.018, 0.21))
                .max_num(-oval(vec3(p.x, 0.0, p.z - 0.64), vec3(size.x * 0.35, 1.0, 0.15)));
            super::blend(pan, pedestal, 0.025)
                .min_num(tank)
                .min_num(lid)
                .min_num(seat)
                .min_num(upright_lid)
                .min_num(upright_seat)
        }
        _ => super::FAR,
    };
    S::new(distance, || (Material::Ceramic, vec3(0.95, 0.95, 0.93)))
}
