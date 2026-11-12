use crate::scene::{
    FAR, Kind, Material, Sample, blend, capsule, cylinder, materials, models::decor, rounded_box, scaled_box, taper,
};
use isthmus::prelude::*;

const LINEN: Vec3 = vec3(0.79, 0.765, 0.70);
pub const BLUE: Vec3 = vec3(0.38, 0.49, 0.56);

#[inline(never)]
pub fn ottoman<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = scaled_box(p, size, vec3(0.0, 0.0, -0.14), vec3(0.96, 0.96, 0.65), 0.06);
    let cushion = scaled_box(p, size, vec3(0.0, 0.0, 0.29), vec3(1.0, 1.0, 0.35), 0.10);
    // Folded dusty blue throw, with a gently rolled edge.
    let cloth = p - vec3(-0.06, 0.08, 0.47) * size;
    let cloth = vec3(cloth.x, cloth.y, cloth.z - 0.003 * (cloth.x * 58.0 + cloth.y * 6.0).sin());
    let throw = rounded_box(cloth, size * vec3(0.68, 0.50, 0.035), 0.006) * 0.83;
    let index = (cloth.x / 0.009).round().max_num(-size.x * 0.30 / 0.009).min_num(size.x * 0.30 / 0.009);
    let fringe_point = vec3(cloth.x - index * 0.009, cloth.y, cloth.z);
    let fringe =
        capsule(fringe_point, vec3(0.0, size.y * 0.25, -0.001), vec3(0.003, size.y * 0.25 + 0.038, -0.006), 0.0015);
    let throw = throw.min_num(fringe);
    let body = base.min_num(cushion);
    S::new(body.min_num(throw), || {
        (
            if throw < body { Material::Knit } else { Material::Fabric },
            if throw < body { vec3(0.64, 0.70, 0.71) } else { LINEN },
        )
    })
}

#[inline(never)]
pub fn rug<S: Sample>(p: Vec3, size: Vec3) -> S {
    S::new(pile_distance::<true>(p, size), || (Material::Shag, Vec3::ONE))
}

#[inline(never)]
fn pile_distance<const CULL: bool>(p: Vec3, size: Vec3) -> f32 {
    // One root field covers the rug; the same yarn bundles form its top, sides and silhouette.
    let nap = vec2(0.32 + 0.25 * (p.y * 13.7 + p.x * 7.3).sin(), 0.22 * (p.x * 17.1 - p.y * 11.7).sin());
    let uv = p.xy() - nap * (p.z - size.z * 0.5).max_num(0.0);
    let shaping = |uv: Vec2| {
        let unit = uv / (size.xy() * 0.5);
        vec2(
            0.009 * (unit.y * unit.y - 0.4) + unit.y * 0.004 + 0.003 * (unit.y * 5.0).sin(),
            0.010 * (unit.x * unit.x - 0.3) - unit.x * 0.003 + 0.004 * (unit.x * 4.0).sin(),
        )
    };
    let outline = vec3(size.x, size.y, 0.20);
    let footprint = rounded_box((uv - shaping(uv)).extend(0.0), outline, 0.065);
    // A gently rippled backing and the yarns share one smooth volume.
    let elevation = |v: Vec2| 0.0025 * (v.x * 3.1 + v.y * 1.7).sin() + 0.0015 * (v.y * 5.3 - v.x * 2.0).sin();
    let backing = footprint.max_num((p.z - elevation(uv)).abs() - size.z * 0.5);
    let mut distance = FAR;
    let spacing = 0.016;
    let cell = (uv / spacing).floor();
    let point = uv.extend(p.z);
    for neighbour in 0..9 {
        // Visit the nearest root cell first, tightening the exclusion bound.
        let offset = match neighbour {
            0 => Vec2::ZERO,
            1 => -Vec2::X,
            2 => Vec2::X,
            3 => -Vec2::Y,
            4 => Vec2::Y,
            5 => vec2(-1.0, -1.0),
            6 => vec2(1.0, -1.0),
            7 => vec2(-1.0, 1.0),
            _ => Vec2::ONE,
        };
        let index = cell + offset;
        let seed = materials::hash(index);
        let variation = materials::hash(index + vec2(37.0, 81.0));
        let center = (index + vec2(0.05 + seed * 0.9, 0.05 + variation * 0.9)) * spacing;
        // Every yarn lies within 11.5 mm of its root, so a farther cell cannot beat a closer candidate.
        let lower = (uv - center).abs().max_element() - 0.0115;
        if CULL && lower > 0.0 && lower / 1.03 >= distance {
            continue;
        }
        let height = materials::hash(index + vec2(19.0, 43.0));
        let thickness = materials::hash(index + vec2(71.0, 29.0));
        let inside = rounded_box((center - shaping(center)).extend(0.0), outline, 0.065);
        if inside > 0.0 {
            continue;
        }
        let root = center.extend(size.z * 0.5 - 0.002 + elevation(center));
        let clump = (center.x * 9.0 + center.y * 3.0).sin() * (center.y * 12.0 - center.x * 2.0).sin();
        let tip = root + vec3((variation - 0.5) * 0.004, (seed - 0.5) * 0.005, 0.017 + height * 0.016 + clump * 0.005);
        let radius = 0.0050 + thickness * 0.0020;
        let yarn = taper(point, root, tip, radius * 1.2, radius * 0.75);
        distance = distance.min_num(yarn);
    }
    // Nap and the gently changing outline deform the whole volume together.
    blend(backing, distance, 0.009) / 1.75
}

/// Cushions bulge between their seams and compress toward their edges.
fn seat_cushion(p: Vec3, size: Vec3, radius: f32) -> f32 {
    let uv = p.xy() / (size.xy() * 0.5);
    let dome = (1.0 - uv.x * uv.x).max_num(0.0) * (1.0 - uv.y * uv.y).max_num(0.0);
    let q = vec3(p.x, p.y, p.z - dome * size.z * 0.10);
    rounded_box(q, size, radius) * 0.75
}

/// Stuffing bulges between a pinched cloth seam; broad folds reach the edges.
fn throw_pillow(p: Vec3, size: Vec3) -> f32 {
    let uv = p.xy() / (size.xy() * 0.5);
    let dome = (1.0 - uv.x * uv.x).max_num(0.0) * (1.0 - uv.y * uv.y).max_num(0.0);
    let edge = uv.abs().max_element().min_num(1.0);
    let warp = vec2(0.003 * (uv.y * 4.0).sin(), 0.004 * (uv.x * 3.0).sin());
    let outline = rounded_box((p.xy() + warp).extend(0.0), size.xy().extend(1.0), 0.009);
    let diagonal = (uv.x + uv.y * 0.45 - 0.45) / 0.13;
    let opposite = (uv.x - uv.y * 0.6 + 0.45) / 0.17;
    let folds = 0.018 / (1.0 + diagonal * diagonal) - 0.012 / (1.0 + opposite * opposite);
    let fold = 0.012 * (uv.x * 7.0 + uv.y * 2.5).sin() * edge * edge;
    let depth = p.z + (fold + folds) * dome;
    let thickness = 0.007 + size.z * 0.5 * dome;
    -blend(-outline, -(depth.abs() - thickness), 0.012) / 2.2
}

#[inline(never)]
pub fn sofa<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = scaled_box(p, size, vec3(0.0, 0.02, -0.34), vec3(0.98, 0.92, 0.30), 0.05);
    // Two separate seat and back cushions with defined seams.
    let mirrored = vec3(p.x.abs(), p.y, p.z);
    let seat_width = (size.x - 0.50) * 0.5;
    let center = vec3(seat_width * 0.5, size.y * 0.12, -size.z * 0.07);
    let seat = seat_cushion(mirrored - center, vec3(seat_width - 0.008, size.y * 0.74, size.z * 0.23), 0.045);
    let back = mirrored - vec3(center.x, -size.y * 0.34, size.z * 0.20);
    let back = vec3(back.x, back.z * 0.98 - back.y * 0.20, back.y * 0.98 + back.z * 0.20);
    let back = seat_cushion(back, vec3(seat_width - 0.008, size.z * 0.57, size.y * 0.23), 0.055);
    let arm = rounded_box(
        mirrored - vec3(size.x * 0.5 - 0.125, size.y * 0.025, -size.z * 0.19),
        vec3(0.25, size.y * 0.96, size.z * 0.68),
        0.045,
    );
    let welt = rounded_box(
        mirrored - vec3(center.x, center.y, -size.z * 0.18),
        vec3(seat_width - 0.003, size.y * 0.747, size.z * 0.010),
        0.008,
    );
    let body = base.min_num(seat).min_num(back).min_num(arm).min_num(welt);
    S::new(body, || (Material::Fabric, LINEN))
}

#[inline(never)]
pub fn pillow<S: Sample>(p: Vec3, size: Vec3) -> S {
    // The upper cloth bends back while the base compresses onto the seat.
    let height = (p.z / (size.z * 0.5)).max_num(0.0).min_num(1.0);
    let q = vec3(p.x, p.y + height * height * 0.025, p.z);
    let leaned = vec3(q.x, q.z * 0.866 - q.y * 0.5, q.y * 0.866 + q.z * 0.5);
    let face = vec3(leaned.x * 0.978 - leaned.y * 0.208, leaned.x * 0.208 + leaned.y * 0.978, leaned.z);
    let body = throw_pillow(face, vec3(size.x, size.z, size.y));
    let seat = (-p.z - size.z * 0.485) * 0.45;
    S::new(-blend(-body, -seat, 0.003), || (Material::Fabric, BLUE))
}

#[inline(never)]
pub fn side_table<S: Sample>(p: Vec3, size: Vec3) -> S {
    // STARKVIND: 54 cm veneer top, hanging cylindrical filter and four legs; size.z is the table alone.
    let deck = size.z * 0.5 - 0.008;
    let top = cylinder(p - Vec3::Z * deck, size.x * 0.5, 0.016);
    let folded = vec3(p.x.abs(), p.y.abs(), p.z);
    let legs = taper(folded, vec3(0.17, 0.17, -size.z * 0.5), vec3(0.135, 0.135, deck - 0.025), 0.016, 0.022);
    let drum = p - Vec3::Z * (deck - 0.112);
    // The perforated housing narrows toward its bottom, with a rounded lip.
    let t = ((drum.z + 0.09) / 0.18).max_num(0.0).min_num(1.0);
    let radius = size.x * (0.405 + 0.070 * t);
    let radial = drum.xy().length() - radius;
    let cap = drum.z.abs() - 0.09;
    let barrel = vec2(radial + 0.006, cap + 0.006).max_num(Vec2::ZERO).length()
        + (radial + 0.006).max_num(cap + 0.006).min_num(0.0)
        - 0.006;
    let shell = barrel.max_num(-(radial + 0.006));
    let arc = drum.y.atan2(drum.x) * size.x * 0.45;
    let row = (drum.z / 0.009).round();
    let stagger = (row * 0.5).fract().abs() * 0.009;
    let hole = vec2(arc - ((arc - stagger) / 0.009).round() * 0.009 - stagger, drum.z - row * 0.009).length() - 0.0025;
    let grille = shell.max_num(-hole) / 1.04;
    let filter = barrel.max_num(radial + 0.010) / 1.04;
    let knob = cylinder(vec3(p.x, p.z - deck + 0.09, p.y - size.x * 0.45), 0.019, 0.022);
    let hardware = grille.min_num(filter).min_num(knob);
    let book = rounded_box(p - vec3(0.025, -0.065, deck + 0.026), vec3(0.19, 0.115, 0.023), 0.003);
    let coaster = cylinder(p - vec3(0.16, 0.025, deck + 0.017), 0.028, 0.005);
    let mut hit = S::new(top.min_num(legs), || (Material::Wood, vec3(0.19, 0.16, 0.13)))
        .union(S::new(hardware, || (Material::Metal, vec3(0.15, 0.16, 0.16))))
        .union(S::new(book, || (Material::Fabric, vec3(0.17, 0.19, 0.15))))
        .union(S::new(coaster, || (Material::Ceramic, vec3(0.92, 0.90, 0.84))));
    // Skip tabletop dressing outside conservative bounds of each component.
    let flowers = p - vec3(0.015, -0.085, deck + 0.112);
    if (flowers.length() - 0.10).max_num(0.0) * 0.4 <= hit.distance() {
        hit = hit.union(decor::table_flowers(flowers + Vec3::Z * 0.07));
    }
    hit
}

#[inline(never)]
pub fn bed<S: Sample>(local: Vec3, size: Vec3) -> S {
    let p = local + Vec3::Z * size.z * 0.5;
    let base = rounded_box(p - vec3(0.0, 0.0, 0.20), vec3(size.x, size.y, 0.36), 0.035);
    let mattress = rounded_box(p - vec3(0.0, 0.0, 0.45), vec3(size.x * 0.98, size.y * 0.97, 0.23), 0.075);
    // Centimetre-scale cloth folds, with a conservative distance correction.
    let fold = 0.009 * (p.x * 21.0 + (p.y * 9.0).sin()).sin() + 0.006 * (p.y * 25.0 + p.x * 6.0).sin();
    let duvet = rounded_box(p - vec3(0.0, 0.10, 0.50 + fold), vec3(size.x + 0.06, size.y - 0.08, 0.25), 0.095) * 0.7;
    let throw = rounded_box(p - vec3(0.0, 0.02, 0.636 + fold), vec3(size.x + 0.07, size.y * 0.64, 0.032), 0.013) * 0.7;
    let sheet = rounded_box(p - vec3(0.0, -size.y * 0.30, 0.627), vec3(size.x, 0.34, 0.043), 0.019);
    let pillow_point = vec3(p.x.abs() - size.x * 0.245, p.y + size.y * 0.34, p.z - 0.704 - 0.055 * p.y);
    let pillows = rounded_box(pillow_point, vec3(size.x * 0.47, 0.43, 0.17), 0.070);
    let head_point = p - vec3(0.0, -size.y * 0.48, 0.69);
    let head = rounded_box(head_point, vec3(size.x + 0.06, 0.11, 1.18), 0.035);
    let panel_point = vec3(
        head_point.x,
        head_point.y - 0.045,
        head_point.z - (head_point.z / 0.255).round().clamp_num(-1.0, 1.0) * 0.255 - 0.21,
    );
    let panels = rounded_box(panel_point, vec3(size.x - 0.07, 0.065, 0.249), 0.023);
    S::new(base, || (Material::Fabric, vec3(0.66, 0.63, 0.58)))
        .union(S::new(mattress.min_num(sheet).min_num(pillows), || (Material::Fabric, vec3(0.96, 0.96, 0.94))))
        .union(S::new(duvet, || (Material::Fabric, vec3(0.20, 0.29, 0.48))))
        .union(S::new(throw, || (Material::Knit, vec3(0.65, 0.65, 0.67))))
        .union(S::new(head.min_num(panels), || {
            let warp = materials::noise(vec2(p.x * 9.0, p.z * 2.0));
            let vein = (p.z * 95.0 + warp * 12.0 + materials::noise(p.xz() * 27.0) * 2.0).sin();
            (Material::Fabric, Vec3::splat(0.57 + 0.065 * vein))
        }))
}

#[inline(never)]
pub fn cabinet<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    let p = local + Vec3::Z * size.z * 0.5;
    let wardrobe = kind == Kind::Wardrobe;
    let columns = if matches!(kind, Kind::Nightstand | Kind::Vanity) { 1.0 } else { 2.0 };
    let rows = if wardrobe {
        1.0
    } else if kind == Kind::Dresser {
        3.0
    } else {
        2.0
    };
    let pitch = vec2((size.x - 0.032) / columns, (size.z - 0.065) / rows);
    let body = rounded_box(p - Vec3::Z * (size.z * 0.5 + 0.018), size - vec3(0.0, 0.025, 0.036), 0.003);
    let front_point = p - vec3(0.0, size.y * 0.5 + 0.006, size.z * 0.5 + 0.018);
    let repeated = vec3(
        front_point.x
            - ((front_point.x / pitch.x - 0.5).round() + 0.5).clamp_num(-(columns - 1.0) * 0.5, (columns - 1.0) * 0.5)
                * pitch.x,
        front_point.y,
        front_point.z
            - ((front_point.z / pitch.y - (rows - 1.0) * 0.5).round() + (rows - 1.0) * 0.5)
                .clamp_num(-(rows - 1.0) * 0.5, (rows - 1.0) * 0.5)
                * pitch.y,
    );
    let fronts = rounded_box(repeated, vec3(pitch.x - 0.004, 0.018, pitch.y - 0.006), 0.002);
    let color = if kind == Kind::Vanity {
        vec3(0.90, 0.89, 0.86)
    } else if wardrobe {
        Vec3::ONE
    } else {
        vec3(0.105, 0.095, 0.087)
    };
    let material = if kind == Kind::Vanity {
        Material::Paint
    } else if wardrobe {
        Material::Maple
    } else {
        Material::Wood
    };
    let handle_point = vec3(p.x.abs() - 0.035, p.y - size.y * 0.5 - 0.035, p.z - size.z * 0.48);
    let handles = capsule(handle_point, vec3(0.0, 0.0, -0.10), vec3(0.0, 0.0, 0.10), 0.007);
    S::new(body, || (material, color * 0.8))
        .union(S::new(fronts, || (material, color)))
        .union(S::new(if wardrobe { handles } else { FAR }, || (Material::Metal, Vec3::splat(0.75))))
}

#[inline(never)]
pub fn accessory<S: Sample>(kind: Kind, p: Vec3, size: Vec3) -> S {
    if kind == Kind::TowerFan {
        let base = cylinder(p + Vec3::Z * (size.z * 0.5 - 0.018), 0.125, 0.035);
        let body = rounded_box(p, vec3(0.13, 0.14, size.z - 0.04), 0.058);
        let vent = rounded_box(p - vec3(0.0, 0.067, -0.08), vec3(0.065, 0.012, size.z * 0.68), 0.017);
        let control = cylinder(vec3(p.x, p.z - size.z * 0.36, p.y - 0.070), 0.035, 0.009);
        return S::new(body.min_num(base), || (Material::Paint, Vec3::splat(0.07)))
            .union(S::new(vent, || (Material::Metal, Vec3::splat(0.12))))
            .union(S::new(control, || (Material::Metal, Vec3::splat(0.55))));
    }
    let disc = cylinder(vec3(p.x, p.z - 0.015, p.y), size.x * 0.48, size.y * 0.70);
    let legs = capsule(p, vec3(-0.04, 0.0, -0.08), vec3(-0.075, 0.015, -size.z * 0.5), 0.012).min_num(capsule(
        p,
        vec3(0.04, 0.0, -0.08),
        vec3(0.075, 0.015, -size.z * 0.5),
        0.012,
    ));
    S::new(disc, || (Material::Fabric, Vec3::splat(0.44))).union(S::new(legs, || (Material::Paint, Vec3::splat(0.90))))
}

#[cfg(test)]
mod tests {
    use crate::scene::models::domestic::*;

    #[test]
    fn procedural_pile_pruning_matches_exhaustive_union_across_scales_and_boundaries() {
        for size in [vec3(0.8, 1.2, 0.008), vec3(1.65, 2.94, 0.012), vec3(3.0, 4.0, 0.025)] {
            for sample in 0..3000 {
                let seed = vec2(sample as f32, 137.0);
                let point = vec3(
                    (materials::hash(seed) - 0.5) * (size.x + 0.12),
                    (materials::hash(seed + 37.0) - 0.5) * (size.y + 0.12),
                    materials::hash(seed + 83.0) * 0.09 - 0.02,
                );
                let accelerated = pile_distance::<true>(point, size);
                let exhaustive = pile_distance::<false>(point, size);
                assert!((accelerated - exhaustive).abs() < 1.0e-7, "pile bound removed visible geometry: {point:?}");
            }
        }
    }
}
