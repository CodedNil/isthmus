//! Compile orthogonal room outlines into exact floor and wall boxes.
use super::{CEILING_THICKNESS, FLOOR_THICKNESS, WALL_HEIGHT, WALL_THICKNESS};
use crate::home::{Kind, Material, Object, Opening, Room, Walls};
use isthmus::glam::{Vec2, Vec3, vec2, vec3};

fn point(room: &Room, local: Vec3, origin: Vec2) -> Vec3 {
    let p = room.position.extend(0.0) + local - origin.extend(0.0);
    vec3(p.x, -p.y, p.z)
}
const fn solid(center: Vec3, size: Vec3, material: Material) -> Object {
    Object::new(Kind::Box, center, size, material)
}

/// Cut a box into at most six disjoint slabs; no GPU CSG or opening lists.
fn subtract(wall: Object, cut: &Opening) -> Vec<Object> {
    let (mut min, mut max) = (wall.pos - wall.size * 0.5, wall.pos + wall.size * 0.5);
    let lo = min.max(cut.pos - cut.size * 0.5);
    let hi = max.min(cut.pos + cut.size * 0.5);
    if !lo.cmplt(hi).all() {
        return vec![wall];
    }
    let slab = |min: Vec3, max: Vec3| solid((min + max) * 0.5, max - min, wall.material);
    let mut pieces = Vec::new();
    for axis in 0..3 {
        if min[axis] < lo[axis] {
            let mut end = max;
            end[axis] = lo[axis];
            pieces.push(slab(min, end));
            min[axis] = lo[axis];
        }
        if max[axis] > hi[axis] {
            let mut start = min;
            start[axis] = hi[axis];
            pieces.push(slab(start, max));
            max[axis] = hi[axis];
        }
    }
    pieces
}

pub(super) fn compile(rooms: &[Room]) -> Vec<Object> {
    let vertices = rooms.iter().flat_map(|r| r.outline.iter().map(|p| r.position + *p));
    let min = vertices.clone().fold(Vec2::splat(f32::MAX), Vec2::min);
    let max = vertices.fold(Vec2::splat(f32::MIN), Vec2::max);
    let origin = if rooms.is_empty() { Vec2::ZERO } else { (min + max) * 0.5 };
    let cuts: Vec<_> = rooms
        .iter()
        .flat_map(|room| {
            room.openings.iter().map(move |opening| Opening { pos: point(room, opening.pos, origin), ..*opening })
        })
        .collect();
    let mut objects = Vec::new();
    for room in rooms {
        let mut walls = Vec::new();
        footprint(room, |min, max, side| {
            let center = point(room, ((min + max) * 0.5).extend(0.0), origin);
            let size = (max - min).abs();
            if let Some(side) = side {
                if room.walls.contains(Walls::from_bits_retain(1 << side)) {
                    let size = (size + Vec2::splat(WALL_THICKNESS)).extend(WALL_HEIGHT);
                    walls.push(solid(center + Vec3::Z * WALL_HEIGHT * 0.5, size, Material::Paint));
                    // Interior trim follows each authored wall, sharing its opening cuts.
                    let normal = match side {
                        0 => Vec3::X,
                        1 => Vec3::Y,
                        2 => -Vec3::X,
                        _ => -Vec3::Y,
                    };
                    for (height, depth, base, material) in
                        [(0.10, 0.016, 0.022, Material::Paint), (0.022, 0.022, 0.0, Material::Wood)]
                    {
                        let mut trim_size = size;
                        trim_size.z = height;
                        if normal.x == 0.0 {
                            trim_size.y = depth;
                        } else {
                            trim_size.x = depth;
                        }
                        let pos =
                            center + normal * (WALL_THICKNESS * 0.5 + depth * 0.5) + Vec3::Z * (base + height * 0.5);
                        walls.push(solid(pos, trim_size, material));
                    }
                }
            } else {
                objects.push(solid(center - Vec3::Z * FLOOR_THICKNESS * 0.5, size.extend(FLOOR_THICKNESS), room.floor));
                // The cutaway hides these from overhead camera rays only.
                // Daylight and GI still see the real, closed room envelope.
                objects.push(Object::new(
                    Kind::Ceiling,
                    center + Vec3::Z * (WALL_HEIGHT + CEILING_THICKNESS * 0.5),
                    size.extend(CEILING_THICKNESS),
                    Material::Paint,
                ));
            }
        });
        for cut in &cuts {
            walls = walls.into_iter().flat_map(|wall| subtract(wall, cut)).collect();
        }
        objects.extend(walls);
        objects.extend(
            room.furniture
                .iter()
                .chain(room.lights.iter().map(|light| &light.fixture))
                .map(|item| Object { pos: point(room, item.pos + Vec3::Z * item.size.z * 0.5, origin), ..*item }),
        );
    }
    objects
}

/// Floors are horizontal strips of the polygon; walls follow its edges.
/// This is only for orthogonal outlines, so every emitted solid is exact.
#[expect(clippy::float_cmp, reason = "Axis alignment is an exact authoring invariant")]
fn footprint(room: &Room, mut emit: impl FnMut(Vec2, Vec2, Option<u32>)) {
    let edges: Vec<_> = room.outline.iter().copied().zip(room.outline.iter().copied().cycle().skip(1)).collect();
    for &(a, b) in &edges {
        assert!((a.x == b.x) != (a.y == b.y), "room edges must be horizontal or vertical and nonzero");
        let side = if a.x == b.x {
            if b.y > a.y { 2 } else { 0 }
        } else if b.x > a.x {
            3
        } else {
            1
        };
        emit(a, b, Some(side));
    }
    let mut ys: Vec<_> = room.outline.iter().map(|p| p.y).collect();
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    for band in ys.windows(2) {
        let y = f32::midpoint(band[0], band[1]);
        let mut xs: Vec<_> = edges
            .iter()
            .filter(|(a, b)| a.x == b.x && a.y.min(b.y) < y && a.y.max(b.y) > y)
            .map(|(a, _)| a.x)
            .collect();
        xs.sort_by(f32::total_cmp);
        for span in xs.as_chunks::<2>().0 {
            emit(vec2(span[0], band[0]), vec2(span[1], band[1]), None);
        }
    }
}
