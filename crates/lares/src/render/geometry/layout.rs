//! Compile orthogonal room outlines into exact floor and wall boxes.
use crate::{
    home::{Kind, Material, Object, Opening, Room, Walls},
    render::{CEILING_THICKNESS, EXTERIOR_WALL_THICKNESS, FLOOR_THICKNESS, WALL_HEIGHT, WALL_THICKNESS},
};
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
            pieces.push(Object { exterior: wall.exterior, ..slab(min, end) });
            min[axis] = lo[axis];
        }
        if max[axis] > hi[axis] {
            let mut start = min;
            start[axis] = hi[axis];
            pieces.push(Object { exterior: wall.exterior, ..slab(start, max) });
            max[axis] = hi[axis];
        }
    }
    pieces
}

/// An opening is cut through its own wall, so it must not bite the skirting of a wall that meets it.
/// Doorways sit near corners, and their cut reaches half a metre into the room.
fn skirting_clear_of(trim: &Object, cut: &Opening) -> bool {
    let along = usize::from(trim.size.y > trim.size.x);
    trim.size.z < 0.2 && cut.size[along] < cut.size[1 - along]
}

pub fn compile(rooms: &[Room]) -> Vec<Object> {
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
                    // Interior trim follows each authored wall, sharing its opening cuts.
                    let normal = match side {
                        0 => Vec3::X,
                        1 => Vec3::Y,
                        2 => -Vec3::X,
                        _ => -Vec3::Y,
                    };
                    let authored_center = vec2(center.x + origin.x, origin.y - center.y);
                    let authored_inward = vec2(normal.x, -normal.y);
                    let outside = authored_center - authored_inward * (WALL_THICKNESS + 0.01);
                    let exterior = !rooms.iter().any(|other| contains(other, outside));
                    let thickness = if exterior { EXTERIOR_WALL_THICKNESS } else { WALL_THICKNESS };
                    let extra = thickness - WALL_THICKNESS;
                    let mut wall_size = size;
                    if normal.x == 0.0 {
                        wall_size.y += extra;
                    } else {
                        wall_size.x += extra;
                    }
                    // Grow outward only: preserve authored room clearances and trim.
                    walls.push(Object {
                        exterior: if exterior { -normal } else { Vec3::ZERO },
                        ..solid(center - normal * extra * 0.5 + Vec3::Z * WALL_HEIGHT * 0.5, wall_size, Material::Paint)
                    });
                    for inward in [normal, -normal].into_iter().take(if exterior { 1 } else { 2 }) {
                        for (height, depth, base, material) in
                            [(0.10, 0.016, 0.022, Material::Paint), (0.022, 0.022, 0.0, Material::Wood)]
                        {
                            let mut trim_size = size;
                            trim_size.z = height;
                            // Wraps exposed corners too, so outward runs meet without a gap.
                            if normal.x == 0.0 {
                                trim_size.x += depth * 2.0;
                                trim_size.y = depth;
                            } else {
                                trim_size.y += depth * 2.0;
                                trim_size.x = depth;
                            }
                            let pos = center
                                + inward * (WALL_THICKNESS * 0.5 + depth * 0.5)
                                + Vec3::Z * (base + height * 0.5);
                            walls.push(solid(pos, trim_size, material));
                        }
                    }
                }
            } else {
                objects.push(solid(center - Vec3::Z * FLOOR_THICKNESS * 0.5, size.extend(FLOOR_THICKNESS), room.floor));
                // Hidden from overhead camera rays only; daylight and GI still see the closed room.
                objects.push(Object::new(
                    Kind::Ceiling,
                    center + Vec3::Z * (WALL_HEIGHT + CEILING_THICKNESS * 0.5),
                    size.extend(CEILING_THICKNESS),
                    Material::Paint,
                ));
            }
        });
        walls.extend(corner_blocks(room, rooms, origin));
        for cut in &cuts {
            walls = walls
                .into_iter()
                .flat_map(|wall| if skirting_clear_of(&wall, cut) { vec![wall] } else { subtract(wall, cut) })
                .collect();
        }
        // Low trim stays only where a surviving wall supports it, so openings leave no stray posts.
        let supported: Vec<_> = walls.iter().copied().filter(|wall| wall.size.z > 0.5).collect();
        walls.retain(|trim| {
            if trim.pos.z + trim.size.z * 0.5 > 0.123 {
                return true;
            }
            let min = trim.pos.truncate() - trim.size.truncate() * 0.5;
            let max = trim.pos.truncate() + trim.size.truncate() * 0.5;
            supported.iter().any(|wall| {
                let lo = wall.pos.truncate() - wall.size.truncate() * 0.5 - Vec2::splat(0.023);
                let hi = wall.pos.truncate() + wall.size.truncate() * 0.5 + Vec2::splat(0.023);
                min.cmplt(hi).all() && max.cmpgt(lo).all()
            })
        });
        objects.extend(walls);
        objects.extend(room.furniture.iter().chain(room.lights.iter().map(|light| &light.fixture)).map(|item| {
            Object {
                pos: point(room, item.pos + Vec3::Z * item.size.z * 0.5, origin),
                exterior: if item.kind == Kind::Window {
                    vec3(-item.rotation.x, item.rotation.y, 0.0)
                } else {
                    item.exterior
                },
                ..*item
            }
        }));
    }
    drop_duplicate_trim(&mut objects);
    objects
}

/// Skirting has a lining on both sides of every wall, so rooms that share a wall each add the same boards.
/// Overlapping coplanar boards shade differently from each other and flicker, so each is kept once.
fn drop_duplicate_trim(objects: &mut Vec<Object>) {
    let is_trim =
        |object: &Object| object.kind == Kind::Box && object.pos.z > 0.0 && object.pos.z + object.size.z * 0.5 < 0.123;
    let covers = |outer: &Object, inner: &Object| {
        let (lo, hi) = (outer.pos - outer.size * 0.5, outer.pos + outer.size * 0.5);
        let (inner_lo, inner_hi) = (inner.pos - inner.size * 0.5, inner.pos + inner.size * 0.5);
        outer.material == inner.material
            && inner_lo.cmpge(lo - Vec3::splat(1.0e-4)).all()
            && inner_hi.cmple(hi + Vec3::splat(1.0e-4)).all()
    };
    let trim: Vec<usize> = (0..objects.len()).filter(|&index| is_trim(&objects[index])).collect();
    let mut dropped = vec![false; objects.len()];
    for &a in &trim {
        // A board goes if another contains it; equal boards go in favour of the earlier one.
        dropped[a] = trim.iter().any(|&b| {
            b != a && !dropped[b] && covers(&objects[b], &objects[a]) && (!covers(&objects[a], &objects[b]) || b < a)
        });
    }
    let mut index = 0;
    objects.retain(|_| {
        index += 1;
        !dropped[index - 1]
    });
}

/// Exterior walls grow outward only, which leaves a square missing at each outside corner; this fills it.
fn corner_blocks(room: &Room, rooms: &[Room], origin: Vec2) -> Vec<Object> {
    let outline = &room.outline;
    let count = outline.len();
    let area: f32 = (0..count).map(|i| outline[i].perp_dot(outline[(i + 1) % count])).sum();
    // The outward side of an edge, in authored axes.
    let outward =
        |direction: Vec2| if area > 0.0 { vec2(direction.y, -direction.x) } else { vec2(-direction.y, direction.x) };
    let walled = |a: Vec2, b: Vec2| room.walls.contains(Walls::from_bits_retain(1 << edge_side(a, b)));
    let open_air = |point: Vec2| !rooms.iter().any(|other| contains(other, room.position + point));
    let mut blocks = Vec::new();
    for index in 0..count {
        let (before, corner, after) =
            (outline[(index + count - 1) % count], outline[index], outline[(index + 1) % count]);
        let (arrive, leave) = ((corner - before).normalize(), (after - corner).normalize());
        // Only outside corners leave a notch.
        if arrive.perp_dot(leave) * area.signum() <= 0.0 || !walled(before, corner) || !walled(corner, after) {
            continue;
        }
        let (first, second) = (outward(arrive), outward(leave));
        let reach = WALL_THICKNESS + 0.01;
        // Probes sit just off each wall, away from the corner, so a neighbour sharing the wall is found.
        let (back, forward) = (corner - arrive * 0.05, corner + leave * 0.05);
        if !open_air(back + first * reach)
            || !open_air(forward + second * reach)
            || !open_air(corner + (first + second) * reach)
        {
            continue;
        }
        let extra = EXTERIOR_WALL_THICKNESS - WALL_THICKNESS;
        let center = corner + (first + second) * (EXTERIOR_WALL_THICKNESS * 0.5);
        let diagonal = first + second;
        blocks.push(Object {
            exterior: vec3(diagonal.x, -diagonal.y, 0.0).normalize(),
            ..solid(
                point(room, center.extend(0.0), origin) + Vec3::Z * WALL_HEIGHT * 0.5,
                vec3(extra, extra, WALL_HEIGHT),
                Material::Paint,
            )
        });
    }
    blocks
}

/// Point-in-polygon classification in authored coordinates, including notches.
fn contains(room: &Room, point: Vec2) -> bool {
    let point = point - room.position;
    let mut inside = false;
    for (a, b) in room.outline.iter().zip(room.outline.iter().cycle().skip(1)) {
        if (a.y > point.y) != (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// Which side of its room an axis-aligned edge walls, numbered as in `Walls`.
#[expect(clippy::float_cmp, reason = "Axis alignment is an exact authoring invariant")]
fn edge_side(a: Vec2, b: Vec2) -> u32 {
    if a.x == b.x {
        if b.y > a.y { 2 } else { 0 }
    } else if b.x > a.x {
        3
    } else {
        1
    }
}

/// Floors are horizontal strips of the polygon and walls follow its edges; outlines must be orthogonal.
#[expect(clippy::float_cmp, reason = "Axis alignment is an exact authoring invariant")]
fn footprint(room: &Room, mut emit: impl FnMut(Vec2, Vec2, Option<u32>)) {
    let edges: Vec<_> = room.outline.iter().copied().zip(room.outline.iter().copied().cycle().skip(1)).collect();
    for &(a, b) in &edges {
        assert!((a.x == b.x) != (a.y == b.y), "room edges must be horizontal or vertical and nonzero");
        let side = edge_side(a, b);
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

#[cfg(test)]
mod tests {
    use crate::render::geometry::layout::*;

    #[test]
    fn outside_corners_are_filled_but_shared_walls_and_inside_corners_are_not() {
        // The blocks are the only objects that are exactly a wall's extra thickness square.
        let blocks = |rooms: &[Room]| {
            let extra = EXTERIOR_WALL_THICKNESS - WALL_THICKNESS;
            compile(rooms)
                .iter()
                .filter(|object| (object.size - vec3(extra, extra, WALL_HEIGHT)).abs().max_element() < 1.0e-5)
                .count()
        };
        let square = |x: f32| Room::new(vec2(x, 0.0), Vec2::splat(4.0), Material::Floorboards);
        assert_eq!(blocks(&[square(0.0)]), 4);
        // An L has five outside corners and one inside corner, which has nothing outside it to fill.
        let mut l_shape = square(0.0);
        l_shape.outline =
            [(-2.0, -2.0), (2.0, -2.0), (2.0, 0.0), (0.0, 0.0), (0.0, 2.0), (-2.0, 2.0)].map(Vec2::from).to_vec();
        assert_eq!(blocks(&[l_shape]), 5);
        // Rooms side by side share a wall, so only their four outer corners are exposed.
        assert_eq!(blocks(&[square(-2.0), square(2.0)]), 4);
    }

    #[test]
    fn trim_covers_return_corners_in_every_orientation() {
        for turn in 0..4 {
            let rotate = |mut point: Vec2| {
                for _ in 0..turn {
                    point = vec2(-point.y, point.x);
                }
                point
            };
            let outline: Vec<_> = [(-2.0, -2.0), (2.0, -2.0), (2.0, 0.0), (0.0, 0.0), (0.0, 2.0), (-2.0, 2.0)]
                .map(|point| rotate(Vec2::from(point)))
                .into_iter()
                .collect();
            let mut room = Room::new(Vec2::ZERO, Vec2::splat(4.0), Material::Floorboards);
            room.outline = outline;
            let objects = compile(&[room]);
            for (material, depth, height, z) in
                [(Material::Paint, 0.016, 0.10, 0.072), (Material::Wood, 0.022, 0.022, 0.011)]
            {
                let corner = rotate(Vec2::splat(-WALL_THICKNESS * 0.5 - depth * 0.5));
                let point = vec3(corner.x, -corner.y, z);
                assert!(
                    objects.iter().any(|object| object.material == material
                        && (object.size.z - height).abs() < 1e-6
                        && (point - object.pos).abs().cmple(object.size * 0.5).all()),
                    "trim leaves a gap at return corner, rotation {turn}"
                );
            }
        }
    }
}
