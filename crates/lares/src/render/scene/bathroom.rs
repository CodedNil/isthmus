use super::{Kind, Material, Sample};
use isthmus::prelude::*;

#[inline(never)]
pub(super) fn model<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let distance = match kind {
        Kind::Bath => {
            let shell = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.05);
            let hollow = super::scaled_box(local, size, vec3(0.0, 0.0, 0.15), vec3(0.78, 0.86, 0.9), 0.08);
            shell.max_num(-hollow)
        }
        Kind::Shower => {
            let tray = super::scaled_box(local, size, vec3(0.0, 0.0, -0.44), vec3(1.0, 1.0, 0.12), 0.03);
            let glass = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.02);
            let inside = super::scaled_box(local, size, Vec3::ZERO, vec3(0.93, 0.93, 1.2), 0.02);
            tray.min_num(glass.max_num(-inside))
        }
        Kind::Sink => {
            let rim = super::scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.03);
            let bowl = super::scaled_box(local, size, vec3(0.0, 0.0, 0.05), vec3(0.7, 0.7, 0.8), 0.2);
            rim.max_num(-bowl)
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
            super::blend(pan, pedestal, 0.025).min_num(tank).min_num(lid).min_num(seat)
        }
        _ => super::FAR,
    };
    S::new(distance, || (Material::Ceramic, vec3(0.8, 0.8, 0.78)))
}
