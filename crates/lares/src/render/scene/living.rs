use super::{Material, Sample};
use isthmus::prelude::*;
const LINEN: Vec3 = vec3(0.79, 0.765, 0.70);
pub(super) const BLUE: Vec3 = vec3(0.38, 0.49, 0.56);

#[inline(never)]
pub(super) fn ottoman<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = super::scaled_box(p, size, vec3(0.0, 0.0, -0.14), vec3(0.96, 0.96, 0.65), 0.06);
    let cushion = super::scaled_box(p, size, vec3(0.0, 0.0, 0.29), vec3(1.0, 1.0, 0.35), 0.10);
    // Folded dusty blue throw, with a gently rolled edge.
    let cloth = p - vec3(-0.06, 0.08, 0.47) * size;
    let cloth = vec3(cloth.x, cloth.y, cloth.z - 0.003 * (cloth.x * 58.0 + cloth.y * 6.0).sin());
    let throw = super::rounded_box(cloth, size * vec3(0.68, 0.50, 0.035), 0.006) * 0.83;
    let index = (cloth.x / 0.009).round().max_num(-size.x * 0.30 / 0.009).min_num(size.x * 0.30 / 0.009);
    let fringe_point = vec3(cloth.x - index * 0.009, cloth.y, cloth.z);
    let fringe = super::capsule(
        fringe_point,
        vec3(0.0, size.y * 0.25, -0.001),
        vec3(0.003, size.y * 0.25 + 0.038, -0.006),
        0.0015,
    );
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
pub(super) fn rug<S: Sample>(p: Vec3, size: Vec3) -> S {
    S::new(pile_distance::<true>(p, size), || (Material::Shag, Vec3::ONE))
}

#[inline(never)]
fn pile_distance<const CULL: bool>(p: Vec3, size: Vec3) -> f32 {
    // One root field covers the entire rug. The very same leaning yarn
    // bundles form its top, sides and silhouette, without an edge treatment.
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
    let footprint = super::rounded_box((uv - shaping(uv)).extend(0.0), outline, 0.065);
    // A gently rippled backing and the yarns share one smooth volume.
    let elevation = |v: Vec2| 0.0025 * (v.x * 3.1 + v.y * 1.7).sin() + 0.0015 * (v.y * 5.3 - v.x * 2.0).sin();
    let backing = footprint.max_num((p.z - elevation(uv)).abs() - size.z * 0.5);
    let mut distance = super::FAR;
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
        let seed = super::materials::hash(index);
        let variation = super::materials::hash(index + vec2(37.0, 81.0));
        let center = (index + vec2(0.05 + seed * 0.9, 0.05 + variation * 0.9)) * spacing;
        // Every yarn lies within 11.5 mm of its root in XY. A positive
        // L-infinity bound cannot beat an already closer candidate.
        let lower = (uv - center).abs().max_element() - 0.0115;
        if CULL && lower > 0.0 && lower / 1.03 >= distance {
            continue;
        }
        let height = super::materials::hash(index + vec2(19.0, 43.0));
        let thickness = super::materials::hash(index + vec2(71.0, 29.0));
        let inside = super::rounded_box((center - shaping(center)).extend(0.0), outline, 0.065);
        if inside > 0.0 {
            continue;
        }
        let root = center.extend(size.z * 0.5 - 0.002 + elevation(center));
        let clump = (center.x * 9.0 + center.y * 3.0).sin() * (center.y * 12.0 - center.x * 2.0).sin();
        let tip = root + vec3((variation - 0.5) * 0.004, (seed - 0.5) * 0.005, 0.017 + height * 0.016 + clump * 0.005);
        let radius = 0.0050 + thickness * 0.0020;
        let yarn = super::taper(point, root, tip, radius * 1.2, radius * 0.75);
        distance = distance.min_num(yarn);
    }
    // Nap and the gently changing outline deform the whole volume together.
    super::blend(backing, distance, 0.009) / 1.75
}
/// Cushions bulge between their seams and compress toward their edges.
fn seat_cushion(p: Vec3, size: Vec3, radius: f32) -> f32 {
    let uv = p.xy() / (size.xy() * 0.5);
    let dome = (1.0 - uv.x * uv.x).max_num(0.0) * (1.0 - uv.y * uv.y).max_num(0.0);
    let q = vec3(p.x, p.y, p.z - dome * size.z * 0.10);
    super::rounded_box(q, size, radius) * 0.75
}
/// Stuffing bulges between a pinched cloth seam; broad folds reach the edges.
fn throw_pillow(p: Vec3, size: Vec3) -> f32 {
    let uv = p.xy() / (size.xy() * 0.5);
    let dome = (1.0 - uv.x * uv.x).max_num(0.0) * (1.0 - uv.y * uv.y).max_num(0.0);
    let edge = uv.abs().max_element().min_num(1.0);
    let warp = vec2(0.003 * (uv.y * 4.0).sin(), 0.004 * (uv.x * 3.0).sin());
    let outline = super::rounded_box((p.xy() + warp).extend(0.0), size.xy().extend(1.0), 0.009);
    let diagonal = (uv.x + uv.y * 0.45 - 0.45) / 0.13;
    let opposite = (uv.x - uv.y * 0.6 + 0.45) / 0.17;
    let folds = 0.018 / (1.0 + diagonal * diagonal) - 0.012 / (1.0 + opposite * opposite);
    let fold = 0.012 * (uv.x * 7.0 + uv.y * 2.5).sin() * edge * edge;
    let depth = p.z + (fold + folds) * dome;
    let thickness = 0.007 + size.z * 0.5 * dome;
    -super::blend(-outline, -(depth.abs() - thickness), 0.012) / 2.2
}
#[inline(never)]
pub(super) fn sofa<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = super::scaled_box(p, size, vec3(0.0, 0.02, -0.34), vec3(0.98, 0.92, 0.30), 0.05);
    // Two separate seat and back cushions with defined seams.
    let mirrored = vec3(p.x.abs(), p.y, p.z);
    let seat_width = (size.x - 0.50) * 0.5;
    let center = vec3(seat_width * 0.5, size.y * 0.12, -size.z * 0.07);
    let seat = seat_cushion(mirrored - center, vec3(seat_width - 0.008, size.y * 0.74, size.z * 0.23), 0.045);
    let back = mirrored - vec3(center.x, -size.y * 0.34, size.z * 0.20);
    let back = vec3(back.x, back.z * 0.98 - back.y * 0.20, back.y * 0.98 + back.z * 0.20);
    let back = seat_cushion(back, vec3(seat_width - 0.008, size.z * 0.57, size.y * 0.23), 0.055);
    let arm = super::rounded_box(
        mirrored - vec3(size.x * 0.5 - 0.125, size.y * 0.025, -size.z * 0.19),
        vec3(0.25, size.y * 0.96, size.z * 0.68),
        0.045,
    );
    let welt = super::rounded_box(
        mirrored - vec3(center.x, center.y, -size.z * 0.18),
        vec3(seat_width - 0.003, size.y * 0.747, size.z * 0.010),
        0.008,
    );
    let body = base.min_num(seat).min_num(back).min_num(arm).min_num(welt);
    S::new(body, || (Material::Fabric, LINEN))
}

#[inline(never)]
pub(super) fn pillow<S: Sample>(p: Vec3, size: Vec3) -> S {
    // The upper cloth bends back while the base compresses onto the seat.
    let height = (p.z / (size.z * 0.5)).max_num(0.0).min_num(1.0);
    let q = vec3(p.x, p.y + height * height * 0.025, p.z);
    let leaned = vec3(q.x, q.z * 0.866 - q.y * 0.5, q.y * 0.866 + q.z * 0.5);
    let face = vec3(leaned.x * 0.978 - leaned.y * 0.208, leaned.x * 0.208 + leaned.y * 0.978, leaned.z);
    let body = throw_pillow(face, vec3(size.x, size.z, size.y));
    let seat = (-p.z - size.z * 0.485) * 0.45;
    S::new(-super::blend(-body, -seat, 0.003), || (Material::Fabric, BLUE))
}

#[inline(never)]
pub(super) fn side_table<S: Sample>(p: Vec3, size: Vec3) -> S {
    // STARKVIND: 54 cm veneer top, suspended cylindrical filter and four legs.
    // size.z describes the table itself; tabletop dressing shares its BVH leaf.
    let deck = size.z * 0.5 - 0.008;
    let top = super::cylinder(p - Vec3::Z * deck, size.x * 0.5, 0.016);
    let folded = vec3(p.x.abs(), p.y.abs(), p.z);
    let legs = super::taper(folded, vec3(0.17, 0.17, -size.z * 0.5), vec3(0.135, 0.135, deck - 0.025), 0.016, 0.022);
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
    let knob = super::cylinder(vec3(p.x, p.z - deck + 0.09, p.y - size.x * 0.45), 0.019, 0.022);
    let hardware = grille.min_num(filter).min_num(knob);
    let book = super::rounded_box(p - vec3(0.025, -0.065, deck + 0.026), vec3(0.19, 0.115, 0.023), 0.003);
    let coaster = super::cylinder(p - vec3(0.16, 0.025, deck + 0.017), 0.028, 0.005);
    let mut hit = S::new(top.min_num(legs), || (Material::Wood, vec3(0.19, 0.16, 0.13)))
        .union(S::new(hardware, || (Material::Metal, vec3(0.15, 0.16, 0.16))))
        .union(S::new(book, || (Material::Fabric, vec3(0.17, 0.19, 0.15))))
        .union(S::new(coaster, || (Material::Ceramic, vec3(0.92, 0.90, 0.84))));
    // Skip tabletop dressing outside conservative bounds of each component.
    let flowers = p - vec3(0.015, -0.085, deck + 0.112);
    if (flowers.length() - 0.10).max_num(0.0) * 0.4 <= hit.distance() {
        hit = hit.union(super::decor::table_flowers(flowers + Vec3::Z * 0.07));
    }
    hit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_pile_pruning_matches_exhaustive_union_across_scales_and_boundaries() {
        for size in [vec3(0.8, 1.2, 0.008), vec3(1.65, 2.94, 0.012), vec3(3.0, 4.0, 0.025)] {
            for sample in 0..3000 {
                let seed = vec2(sample as f32, 137.0);
                let point = vec3(
                    (super::super::materials::hash(seed) - 0.5) * (size.x + 0.12),
                    (super::super::materials::hash(seed + 37.0) - 0.5) * (size.y + 0.12),
                    super::super::materials::hash(seed + 83.0) * 0.09 - 0.02,
                );
                let accelerated = pile_distance::<true>(point, size);
                let exhaustive = pile_distance::<false>(point, size);
                assert!((accelerated - exhaustive).abs() < 1.0e-7, "pile bound removed visible geometry: {point:?}");
            }
        }
    }
}
