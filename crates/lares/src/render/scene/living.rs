use super::{Material, Sample};
use isthmus::prelude::*;
const LINEN: Vec3 = vec3(0.79, 0.765, 0.70);
const BLUE: Vec3 = vec3(0.38, 0.49, 0.56);

#[inline(never)]
pub(super) fn ottoman<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = super::scaled_box(p, size, vec3(0.0, 0.0, -0.14), vec3(0.96, 0.96, 0.65), 0.06);
    let cushion = super::scaled_box(p, size, vec3(0.0, 0.0, 0.29), vec3(1.0, 1.0, 0.35), 0.10);
    let piping = super::scaled_box(p, size, vec3(0.0, 0.0, 0.14), vec3(0.985, 0.985, 0.018), 0.04);
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
    let body = base.min_num(cushion).min_num(piping);
    S::new(body.min_num(throw), || {
        (
            if throw < body { Material::Knit } else { Material::Fabric },
            if throw < body { vec3(0.64, 0.70, 0.71) } else { LINEN },
        )
    })
}
#[inline(never)]
pub(super) fn rug<S: Sample>(p: Vec3, size: Vec3) -> S {
    // Dense, continuous pile avoids empty gaps between isolated fiber capsules.
    // These are centimeter-scale height changes in the traced surface, not bump.
    let rotated = vec2(p.x * 0.8 - p.y * 0.6, p.x * 0.6 + p.y * 0.8);
    let clumps = super::materials::noise(rotated * 38.0);
    let nap = super::materials::noise(p.xy() * 17.0 + 3.7);
    let height = size.z * 0.5 + 0.006 + clumps * 0.002 + nap * 0.001;
    let edge = super::rounded_box(vec3(p.x, p.y, 0.0), vec3(size.x, size.y, 0.04), 0.012);
    let layer = (p.z - height).max_num(-p.z - size.z * 0.5);
    // Quintic noise gradient <= 2.652 per octave. The combined height
    // slope <= 0.740 gives a tight, conservative extrusion bound of 1.25.
    let distance = edge.max_num(layer) / 1.25;
    S::new(distance, || (Material::Shag, Vec3::ONE))
}
/// Cushions bulge between their seams and compress toward their edges.
fn cushion(p: Vec3, size: Vec3, radius: f32) -> f32 {
    let uv = p.xy() / (size.xy() * 0.5);
    let dome = (1.0 - uv.x * uv.x).max_num(0.0) * (1.0 - uv.y * uv.y).max_num(0.0);
    let q = vec3(p.x, p.y, p.z - dome * size.z * 0.10);
    super::rounded_box(q, size, radius) * 0.75
}
#[inline(never)]
pub(super) fn sofa<S: Sample>(p: Vec3, size: Vec3) -> S {
    let base = super::scaled_box(p, size, vec3(0.0, 0.02, -0.34), vec3(0.98, 0.92, 0.30), 0.05);
    // Two separate seat and back cushions with defined seams.
    let mirrored = vec3(p.x.abs(), p.y, p.z);
    let seat_width = (size.x - 0.50) * 0.5;
    let center = vec3(seat_width * 0.5, size.y * 0.12, -size.z * 0.07);
    let seat = cushion(mirrored - center, vec3(seat_width - 0.008, size.y * 0.74, size.z * 0.23), 0.045);
    let back = mirrored - vec3(center.x, -size.y * 0.34, size.z * 0.20);
    let back = vec3(back.x, back.z * 0.98 - back.y * 0.20, back.y * 0.98 + back.z * 0.20);
    let back = cushion(back, vec3(seat_width - 0.008, size.z * 0.57, size.y * 0.23), 0.055);
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
    // Cushions lean against the back, at a real physical angle.
    let q = p - vec3(if p.x < 0.0 { -0.28 } else { 0.28 }, -0.20, 0.19) * size;
    let pillow = vec3(q.x, q.y * 0.91 + q.z * 0.414, -q.y * 0.414 + q.z * 0.91);
    let edge = (pillow.x.abs() / (size.x * 0.085)).max_num(0.0).min_num(1.0);
    let creases = 0.008 * (pillow.x * 53.0 + pillow.z * 12.0).sin() * edge * edge;
    let folded = vec3(pillow.x, pillow.y + creases, pillow.z);
    // Puckered seam and a softly inflated center rather than a rigid slab.
    let face = vec3(folded.x, folded.z, folded.y);
    let pillow_size = vec3(0.44, 0.43, 0.15);
    let pillow_body = cushion(face, pillow_size, 0.045);
    let piping = super::rounded_box(face, vec3(0.438, 0.428, 0.006), 0.003);
    let pillow = pillow_body.min_num(piping) * 0.70;
    let cushion = pillow < body;
    S::new(body.min_num(pillow), || {
        (
            if cushion && p.x > 0.0 { Material::Knit } else { Material::Fabric },
            if cushion { if p.x > 0.0 { vec3(0.34, 0.45, 0.48) } else { BLUE } } else { LINEN },
        )
    })
}

#[inline(never)]
pub(super) fn side_table<S: Sample>(p: Vec3, size: Vec3) -> S {
    // STARKVIND: 54 cm veneer top, suspended cylindrical filter and four legs.
    // size.z describes the table itself; tabletop dressing shares its BVH leaf.
    let deck = size.z * 0.5 - 0.012;
    let top = super::cylinder(p - Vec3::Z * deck, size.x * 0.5, 0.024);
    let folded = vec3(p.x.abs(), p.y.abs(), p.z);
    let legs = super::taper(folded, vec3(0.17, 0.17, -size.z * 0.5), vec3(0.135, 0.135, deck - 0.025), 0.016, 0.022);
    let drum = p - Vec3::Z * (deck - 0.10);
    let barrel = super::cylinder(drum, size.x * 0.435, 0.15);
    let shell = barrel.max_num(-super::cylinder(drum, size.x * 0.435 - 0.006, 0.145));
    let arc = drum.y.atan2(drum.x) * size.x * 0.435;
    let hole = vec2(arc - (arc / 0.009).round() * 0.009, drum.z - (drum.z / 0.009).round() * 0.009).length() - 0.0025;
    let grille = shell.max_num(-hole);
    let filter = super::cylinder(drum, size.x * 0.41, 0.14);
    let knob = super::cylinder(vec3(p.x, p.z - deck + 0.09, p.y - size.x * 0.44), 0.019, 0.022);
    let hardware = grille.min_num(filter).min_num(knob);
    let book = super::rounded_box(p - vec3(0.025, -0.065, deck + 0.026), vec3(0.19, 0.115, 0.023), 0.003);
    let coaster = super::cylinder(p - vec3(0.16, 0.025, deck + 0.017), 0.028, 0.005);
    let mut hit = S::new(top.min_num(legs), || (Material::Wood, vec3(0.19, 0.16, 0.13)))
        .union(S::new(hardware, || (Material::Metal, vec3(0.15, 0.16, 0.16))))
        .union(S::new(book, || (Material::Fabric, vec3(0.17, 0.19, 0.15))))
        .union(S::new(coaster, || (Material::Ceramic, vec3(0.92, 0.90, 0.84))));
    // Skip tabletop dressing outside conservative bounds of each component.
    let lamp = p - vec3(-0.10, 0.085, deck + 0.193);
    if (lamp.length() - 0.18).max_num(0.0) * 0.4 <= hit.distance() {
        hit = hit.union(super::decor::table_lamp(lamp + Vec3::Z * 0.18));
    }
    let flowers = p - vec3(0.015, -0.085, deck + 0.112);
    if (flowers.length() - 0.10).max_num(0.0) * 0.4 <= hit.distance() {
        hit = hit.union(super::decor::table_flowers(flowers + Vec3::Z * 0.07));
    }
    hit
}
