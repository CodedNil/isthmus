use crate::scene::{FAR, Kind, Material, Sample, blend, box_sdf, capsule, cylinder, rounded_box, scaled_box};
use core::f32::consts::TAU;
use isthmus::prelude::*;

fn radiator(p: Vec3, size: Vec3) -> f32 {
    let panel = rounded_box(p, size * vec3(0.96, 0.64, 0.88), 0.012);
    // Bounded domain repetition: vertical pressed channels and real top vents.
    let pitch = 0.027;
    let count = (size.x * 0.45 / pitch).floor();
    let x = p.x - (p.x / pitch).round().max_num(-count).min_num(count) * pitch;
    let groove = rounded_box(vec3(x, p.y.abs() - size.y * 0.32, p.z), vec3(0.007, 0.008, size.z * 0.78), 0.003);
    let panel = panel.max_num(-groove);
    let cap = rounded_box(p - Vec3::Z * size.z * 0.45, vec3(size.x * 0.98, size.y * 0.92, 0.025), 0.008);
    let vent = rounded_box(vec3(x, p.y, p.z - size.z * 0.45), vec3(0.015, size.y * 0.57, 0.04), 0.004);
    let cap = cap.max_num(-vent);
    let endcap =
        rounded_box(vec3(p.x.abs() - size.x * 0.48, p.y, p.z), vec3(0.025, size.y * 0.9, size.z * 0.90), 0.008);
    let valve = capsule(p, vec3(size.x * 0.49, 0.0, -size.z * 0.35), vec3(size.x * 0.54, 0.0, -size.z * 0.35), 0.025);
    panel.min_num(cap).min_num(endcap).min_num(valve)
}

fn window(p: Vec3, size: Vec3) -> f32 {
    let sill = rounded_box(p - vec3(0.0, -0.015, -size.z * 0.5 + 0.012), vec3(size.x + 0.06, 0.35, 0.035), 0.008);
    let frame = rounded_box(p - Vec3::Y * 0.08, vec3(size.x, 0.055, size.z), 0.008);
    let opening = box_sdf(p - Vec3::Y * 0.08, vec3(size.x - 0.08, 0.14, size.z - 0.08));
    let frame = frame.max_num(-opening);
    let mullion = rounded_box(p - Vec3::Y * 0.08, vec3(0.035, 0.055, size.z - 0.05), 0.004);
    sill.min_num(frame).min_num(if size.x > 0.90 { mullion } else { FAR })
}

fn curtain<S: Sample>(p: Vec3, size: Vec3) -> S {
    // Broad vertical folds gather into a narrow waist, with a hem.
    let height = (p.z / size.z + 0.5).max_num(0.0).min_num(1.0);
    let t = ((height - 0.18) / 0.82).max_num(0.0).min_num(1.0);
    let spread = t * t * (3.0 - 2.0 * t);
    let width = 0.048 + 0.087 * spread + 0.020 * (1.0 - height / 0.18).max_num(0.0);
    let center = size.x * 0.5 - 0.095 - 0.05 * spread;
    let across = p.x.abs() - center;
    let depth = p.y + 0.025 - 0.035 * (1.0 - spread);
    let pleat = (across * TAU / 0.037).cos() * (0.010 + 0.007 * spread);
    let cloth = rounded_box(
        vec3(across, depth - pleat, p.z),
        vec3(width * 2.0, 0.024 + 0.028 * (1.0 - spread), size.z * 0.96),
        0.010,
    ) * 0.5;
    let rod = capsule(p, vec3(-size.x * 0.5, -0.025, size.z * 0.49), vec3(size.x * 0.5, -0.025, size.z * 0.49), 0.010);
    let tie = vec3(p.x.abs() - (size.x * 0.5 - 0.095), p.y - 0.01, p.z + size.z * 0.32);
    let hoop = vec2(tie.xy().length() - 0.067, tie.z).length() - 0.005;
    let mount = capsule(tie, vec3(0.0, 0.065, 0.0), vec3(0.0, 0.085, 0.0), 0.012);
    let finial = vec3(p.x.abs() - size.x * 0.49, p.y + 0.025, p.z - size.z * 0.49).length() - 0.020;
    S::new(cloth, || (Material::Fabric, vec3(0.68, 0.64, 0.57)))
        .union(S::new(rod.min_num(hoop).min_num(mount).min_num(finial), || (Material::Metal, vec3(0.65, 0.63, 0.58))))
}

// The cupboard contains a hot-water cylinder, not a rectangular wall boiler.
fn cylinder_unit<S: Sample>(local: Vec3, size: Vec3) -> S {
    let p = local + Vec3::Z * size.z * 0.5;
    let tank = cylinder(p - Vec3::Z * (size.z * 0.48), size.x * 0.45, size.z * 0.88);
    let shoulder = ((p - Vec3::Z * (size.z * 0.88)) / vec3(size.x * 0.45, size.y * 0.45, 0.13)).length() - 1.0;
    let base = cylinder(p - Vec3::Z * 0.065, size.x * 0.46, 0.10);
    let mut pipes = FAR;
    let mut valves = FAR;
    for i in 0..3 {
        let x = (i as f32 - 1.0) * 0.14;
        let height = 1.12 + i as f32 * 0.12;
        pipes = pipes.min_num(capsule(p, vec3(x, 0.255, 0.20), vec3(x, 0.255, height), 0.012));
        pipes = pipes.min_num(capsule(p, vec3(x, 0.255, height), vec3(x, 0.10, height), 0.012));
        valves = valves.min_num(rounded_box(p - vec3(x, 0.28, height - 0.12), vec3(0.065, 0.015, 0.014), 0.006));
    }
    for i in 0..2 {
        let height = 1.61 + i as f32 * 0.12;
        let left = vec3(-0.23, 0.12, height);
        let right = vec3(0.23, 0.12, height);
        pipes = pipes.min_num(capsule(p, left, right, 0.012));
        pipes = pipes.min_num(capsule(p, right, right - Vec3::Z * 0.18, 0.012));
        pipes = pipes.min_num(capsule(p, left, left + Vec3::Z * 0.06, 0.012));
    }
    let insulation = capsule(p, vec3(-0.18, 0.12, 1.61), vec3(0.06, 0.12, 1.61), 0.027);
    S::new(tank.min_num(shoulder * 0.13), || (Material::Paint, vec3(0.52, 0.53, 0.54)))
        .union(S::new(base, || (Material::Paint, Vec3::splat(0.12))))
        .union(S::new(pipes, || (Material::Metal, vec3(0.68, 0.39, 0.20))))
        .union(S::new(valves, || (Material::Paint, vec3(0.65, 0.08, 0.06))))
        .union(S::new(insulation, || (Material::Metal, Vec3::splat(0.68))))
}

fn linen<S: Sample>(p: Vec3, size: Vec3) -> S {
    let pitch = size.z / 4.0;
    let row = (p.z / pitch + 1.5).round().clamp_num(0.0, 3.0);
    let folded = rounded_box(
        p - vec3(0.008 * row.sin(), 0.006 * row.cos(), (row - 1.5) * pitch),
        vec3(size.x, size.y, pitch * 0.92),
        pitch * 0.35,
    );
    S::new(folded, || (Material::Knit, Vec3::splat(if row < 1.0 { 0.20 } else { 0.88 - row * 0.025 })))
}

#[inline(never)]
pub fn utility_model<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let distance = match kind {
        Kind::Appliance => return cylinder_unit(local, size),
        Kind::LinenStack => return linen(local, size),
        Kind::Radiator => radiator(local, size),
        Kind::Window => window(local, size),
        Kind::Curtain => return curtain(local, size),
        _ => FAR,
    };
    let painted = kind != Kind::Appliance;
    S::new(distance, || {
        (
            if painted { Material::Paint } else { Material::Metal },
            if painted { vec3(0.93, 0.92, 0.89) } else { vec3(0.45, 0.45, 0.43) },
        )
    })
}

#[inline(never)]
pub fn bath_model<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let distance = match kind {
        Kind::Bath => {
            let shell = scaled_box(local, size, Vec3::ZERO, Vec3::ONE, 0.05);
            let hollow = scaled_box(local, size, vec3(0.0, 0.0, 0.15), vec3(0.78, 0.86, 0.9), 0.08);
            let p = local + Vec3::Z * size.z * 0.5;
            let tap_point = vec3(p.x.abs() - 0.11, p.y + size.y * 0.39, p.z - size.z);
            let taps = cylinder(tap_point - Vec3::Z * 0.035, 0.017, 0.075).min_num(capsule(
                tap_point,
                vec3(0.0, 0.0, 0.04),
                vec3(0.0, 0.08, 0.04),
                0.012,
            ));
            let overflow = cylinder(vec3(p.x, p.z - size.z * 0.73, p.y + size.y * 0.42), 0.023, 0.008);
            return S::new(shell.max_num(-hollow), || (Material::Ceramic, Vec3::splat(0.96)))
                .union(S::new(taps.min_num(overflow), || (Material::Metal, Vec3::splat(0.78))));
        }
        Kind::Shower => {
            let p = local + Vec3::Z * size.z * 0.5;
            let tray = rounded_box(p - Vec3::Z * 0.065, vec3(size.x, size.y, 0.13), 0.025);
            let basin = rounded_box(p - Vec3::Z * 0.13, vec3(size.x - 0.065, size.y - 0.065, 0.12), 0.025);
            let posts = rounded_box(
                vec3(p.x.abs() - size.x * 0.48, p.y.abs() - size.y * 0.48, p.z - size.z * 0.52),
                vec3(0.023, 0.023, size.z * 0.94),
                0.004,
            );
            let tracks = rounded_box(
                vec3(p.x, p.y.abs() - size.y * 0.48, (p.z - size.z * 0.52).abs() - size.z * 0.46),
                vec3(size.x, 0.030, 0.032),
                0.005,
            );
            let stile =
                rounded_box(p - vec3(0.0, size.y * 0.48, size.z * 0.52), vec3(0.014, 0.018, size.z * 0.93), 0.003);
            let handle =
                capsule(p, vec3(0.028, size.y * 0.5 + 0.024, 0.88), vec3(0.028, size.y * 0.5 + 0.024, 1.10), 0.009);
            let rail = capsule(p, vec3(0.0, -size.y * 0.45, 1.05), vec3(0.0, -size.y * 0.45, 1.83), 0.012);
            let mixer = capsule(p, vec3(-0.115, -size.y * 0.43, 1.01), vec3(0.115, -size.y * 0.43, 1.01), 0.023);
            let head = capsule(p, vec3(0.025, -size.y * 0.40, 1.70), vec3(0.025, -size.y * 0.36, 1.83), 0.032);
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
            let pedestal = rounded_box(p - vec3(0.0, -0.055, 0.36), vec3(0.16, 0.20, 0.70), 0.065);
            let tap = capsule(p, vec3(0.0, -size.y * 0.35, 0.81), vec3(0.0, -size.y * 0.35, 0.96), 0.022);
            let spout = capsule(p, vec3(0.0, -size.y * 0.35, 0.94), vec3(0.0, 0.025, 0.94), 0.016);
            let drain = cylinder(p - vec3(0.0, 0.025, 0.715), 0.025, 0.006);
            return S::new(blend(rim.max_num(-bowl), pedestal, 0.02), || (Material::Ceramic, Vec3::splat(0.96)))
                .union(S::new(tap.min_num(spout).min_num(drain), || (Material::Metal, Vec3::splat(0.78))));
        }
        Kind::TowelRail => {
            let posts =
                capsule(local, vec3(size.x * 0.45, 0.0, -size.z * 0.5), vec3(size.x * 0.45, 0.0, size.z * 0.5), 0.016)
                    .min_num(capsule(
                        local,
                        vec3(-size.x * 0.45, 0.0, -size.z * 0.5),
                        vec3(-size.x * 0.45, 0.0, size.z * 0.5),
                        0.016,
                    ));
            let repeated = vec3(local.x, local.y, local.z - (local.z / 0.09).round().clamp_num(-3.0, 3.0) * 0.09);
            let bars = capsule(repeated, vec3(-size.x * 0.45, 0.0, 0.0), vec3(size.x * 0.45, 0.0, 0.0), 0.010);
            let cloth = rounded_box(
                local - vec3(0.0, 0.035 + 0.008 * (local.x * 95.0).sin(), -0.05),
                vec3(size.x * 0.70, 0.028, size.z * 0.60),
                0.010,
            ) * 0.55;
            return S::new(posts.min_num(bars), || (Material::Metal, Vec3::splat(0.8)))
                .union(S::new(cloth, || (Material::Knit, vec3(0.43, 0.42, 0.44))));
        }
        Kind::BathMat => {
            return S::new(rounded_box(local, size, 0.009), || (Material::Knit, vec3(0.53, 0.51, 0.53)));
        }
        Kind::ExtractorVent => {
            let frame = rounded_box(local, size, 0.004);
            let radial = vec2(local.x, local.z).length();
            let grille = ((radial / 0.009).fract() - 0.5).abs() < 0.18 && radial < size.x * 0.39;
            return S::new(frame, || (Material::Paint, Vec3::splat(if grille { 0.09 } else { 0.92 })));
        }
        Kind::WasteBin => {
            let body = cylinder(local, size.x * 0.48, size.z);
            let lid = cylinder(local - Vec3::Z * (size.z * 0.5), size.x * 0.49, 0.012);
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
            let pedestal = rounded_box(p - vec3(0.0, 0.005, 0.135), vec3(0.21, 0.32, 0.27), 0.055);
            let tank = rounded_box(p - vec3(0.0, -0.245, 0.59), vec3(size.x * 0.95, 0.15, 0.38), 0.025);
            let lid = rounded_box(p - vec3(0.0, -0.245, 0.79), vec3(size.x, 0.17, 0.025), 0.009);
            let seat_outer = oval(p - vec3(0.0, 0.075, 0.433), vec3(size.x * 0.49, size.y * 0.435, 0.018));
            let seat_hole = oval(vec3(p.x, p.y - 0.09, 0.0), vec3(size.x * 0.35, size.y * 0.32, 1.0));
            let seat = seat_outer.max_num(-seat_hole);
            let upright_lid = oval(p - vec3(0.0, -0.16, 0.64), vec3(size.x * 0.48, 0.025, 0.215));
            let upright_seat = oval(p - vec3(0.0, -0.12, 0.64), vec3(size.x * 0.49, 0.018, 0.21))
                .max_num(-oval(vec3(p.x, 0.0, p.z - 0.64), vec3(size.x * 0.35, 1.0, 0.15)));
            blend(pan, pedestal, 0.025)
                .min_num(tank)
                .min_num(lid)
                .min_num(seat)
                .min_num(upright_lid)
                .min_num(upright_seat)
        }
        _ => FAR,
    };
    S::new(distance, || (Material::Ceramic, vec3(0.95, 0.95, 0.93)))
}
