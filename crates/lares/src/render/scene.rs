use super::{DOOR_HEIGHT, FLOOR_THICKNESS, Globals, WALL_HEIGHT, WALL_THICKNESS, WINDOW_HEAD, WINDOW_SILL};
use crate::home::{Action, Bounds, FurnitureKind, Material, OpeningKind, Room, Walls};
use isthmus::prelude::*;

mod bathroom;
mod bedroom;
mod decor;
mod kitchen;
mod living;
mod materials;
mod office;
mod utility;

#[derive(Clone, Copy)]
struct Hit {
    distance: f32,
    material: Material,
    tint: Vec3,
}

fn render_furniture(kind: FurnitureKind, local: Vec3, size: Vec3) -> Hit {
    match kind {
        FurnitureKind::Bed => bedroom::bed(local, size),
        FurnitureKind::Sofa => living::sofa(local, size),
        FurnitureKind::Ottoman => living::ottoman(local, size),
        FurnitureKind::Rug => living::rug(local, size),
        FurnitureKind::Chair => office::chair(local, size),
        FurnitureKind::Table => office::table(local, size),
        FurnitureKind::Workstation => office::workstation(local, size),
        FurnitureKind::Bath => bathroom::bath(local, size),
        FurnitureKind::Shower => bathroom::shower(local, size),
        FurnitureKind::Sink => bathroom::sink(local, size),
        FurnitureKind::Toilet => bathroom::toilet(local, size),
        FurnitureKind::Counter => kitchen::counter(local, size),
        FurnitureKind::Cupboard => kitchen::cupboard(local, size),
        FurnitureKind::Fridge => kitchen::fridge(local, size),
        FurnitureKind::Oven => kitchen::oven(local, size),
        FurnitureKind::Plant => decor::plant(local, size),
        FurnitureKind::WallArt => decor::wall_art(local, size),
        FurnitureKind::Appliance => utility::appliance(local, size),
        FurnitureKind::Radiator => utility::radiator(local, size),
    }
}

const WALL_COLOR: Vec3 = vec3(0.86, 0.84, 0.8);
const MAX_STEPS: u32 = 48;
const MIN_HIT_DISTANCE: f32 = 0.002;
const NORMAL_OFFSET: f32 = 0.004;
const CUT_REACH: f32 = WALL_THICKNESS * 2.0;
const FAR: f32 = 1000.0;
const SUN: Vec3 = vec3(0.45, 0.75, 0.5);
const FLOOR_SOURCE: u32 = 0;
const WALL_SOURCE: u32 = 1;
const FURNITURE_SOURCE: u32 = 2;

fn unrotate_z(point: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    vec3(point.x * cos + point.y * sin, -point.x * sin + point.y * cos, point.z)
}

fn box_sdf(point: Vec3, size: Vec3) -> f32 {
    let offset = point.abs() - size * 0.5;
    offset.max(Vec3::ZERO).length() + offset.max_element().min(0.0)
}

fn rect_sdf(point: Vec2, size: Vec2) -> f32 {
    let offset = point.abs() - size * 0.5;
    offset.max(Vec2::ZERO).length() + offset.max_element().min(0.0)
}

fn footprint(room: &Room, point: Vec2) -> f32 {
    let mut distance = rect_sdf(point, room.size);
    for index in 0..room.operations.len() {
        let operation = room.operations[index];
        let center = vec2(operation.pos.x, -operation.pos.y);
        let other = rect_sdf(point - center, operation.size);
        distance = if operation.action == Action::Subtract { distance.max(-other) } else { distance.min(other) };
    }
    distance
}

fn wall_ring(room: &Room, local: Vec3, plan: f32, height: f32) -> f32 {
    let outline = plan.abs() - WALL_THICKNESS * 0.5;
    let half = room.size * 0.5;
    let inset = WALL_THICKNESS * 0.5;
    let left = if room.walls.contains(Walls::LEFT) { outline.max(local.x + half.x - inset) } else { FAR };
    let right = if room.walls.contains(Walls::RIGHT) { outline.max(half.x - local.x - inset) } else { FAR };
    let top = if room.walls.contains(Walls::TOP) { outline.max(local.y + half.y - inset) } else { FAR };
    let bottom = if room.walls.contains(Walls::BOTTOM) { outline.max(half.y - local.y - inset) } else { FAR };
    let walls = left.min(right).min(top).min(bottom);
    walls.max(height)
}

fn rounded_box(point: Vec3, size: Vec3, radius: f32) -> f32 {
    let radius = radius.min(size.min_element() * 0.5);
    let offset = point.abs() - size * 0.5 + radius;
    offset.max(Vec3::ZERO).length() + offset.max_element().min(0.0) - radius
}

fn capsule(point: Vec3, start: Vec3, end: Vec3, radius: f32) -> f32 {
    let axis = end - start;
    let along = ((point - start).dot(axis) / axis.length_squared().max(1.0e-6)).clamp(0.0, 1.0);
    (point - start - axis * along).length() - radius
}

fn taper(point: Vec3, start: Vec3, end: Vec3, start_radius: f32, end_radius: f32) -> f32 {
    let axis = end - start;
    let along = ((point - start).dot(axis) / axis.length_squared().max(1.0e-6)).clamp(0.0, 1.0);
    (point - start - axis * along).length() - (start_radius + (end_radius - start_radius) * along)
}

fn blend(a: f32, b: f32, amount: f32) -> f32 {
    let k = amount.max(1.0e-5);
    let t = (1.0 - (a - b).abs() / k).saturate();
    a.min(b) - k * t * t * 0.25
}

fn scaled_box(local: Vec3, size: Vec3, center: Vec3, extent: Vec3, radius: f32) -> f32 {
    rounded_box(local - center * size, size * extent, radius * size.min_element())
}

fn frond_of(point: Vec3, crown: Vec3, direction: Vec2, reach: f32) -> f32 {
    let tip = crown + vec3(direction.x * reach, direction.y * reach, reach * 0.3);
    taper(point, crown, tip, 0.045, 0.004)
}

fn wall_distance(room: &Room, local: Vec3, plan: f32, point: Vec3) -> f32 {
    let height = (-local.z).max(local.z - WALL_HEIGHT);
    let mut wall = wall_ring(room, local, plan, height);
    if wall < CUT_REACH {
        for offset in 0..room.openings.len() {
            let opening = room.openings[offset];
            let (height, y) = if opening.kind == OpeningKind::Door {
                (DOOR_HEIGHT, DOOR_HEIGHT * 0.5)
            } else {
                (WINDOW_HEAD - WINDOW_SILL, f32::midpoint(WINDOW_SILL, WINDOW_HEAD))
            };
            let center = vec3(room.position.x + opening.pos.x, room.position.y - opening.pos.y, y);
            let size = vec3(opening.width, WALL_THICKNESS * 4.0, height);
            let delta = point - center;
            let reach = wall.max(0.0) + size.max_element() * 0.866_025_4;
            if delta.length_squared() < reach * reach {
                wall = wall.max(-box_sdf(unrotate_z(delta, opening.rotation), size));
            }
        }
    }
    wall
}

fn room_hit(room: &Room, point: Vec3, furniture_mask: u32, source: &mut u32) -> Hit {
    let room_origin = vec3(room.position.x, room.position.y, 0.0);
    let local = point - room_origin;
    let plan = footprint(room, local.xy());
    let slab = (local.z + FLOOR_THICKNESS * 0.5).abs() - FLOOR_THICKNESS * 0.5;
    let mut best = Hit { distance: plan.max(slab), material: room.floor, tint: Vec3::ONE };
    *source = FLOOR_SOURCE;
    let wall = wall_distance(room, local, plan, point);
    if wall < best.distance {
        best = Hit { distance: wall, material: Material::Paint, tint: WALL_COLOR };
        *source = WALL_SOURCE;
    }

    for offset in 0..room.furniture.len() {
        if furniture_mask & (1 << offset) == 0 {
            continue;
        }
        let item = room.furniture[offset];
        let center = room_origin + vec3(item.pos.x, -item.pos.y, item.pos.z + item.size.z * 0.5);
        let delta = point - center;
        let reach = best.distance.max(0.0) + item.size.max_element() * 0.866_025_4 + 0.05;
        if delta.length_squared() > reach * reach {
            continue;
        }
        let local = unrotate_z(delta, item.rotation);
        let outside = (local.abs() - item.size * 0.5).max(Vec3::ZERO);
        let limit = best.distance.max(0.0);
        if outside.length_squared() > limit * limit {
            continue;
        }
        let model = render_furniture(item.kind, local, item.size);
        if model.distance < best.distance {
            best = model;
            *source = FURNITURE_SOURCE + offset as u32;
        }
    }
    best
}

fn surface_distance(room: &Room, point: Vec3, source: u32) -> f32 {
    let origin = vec3(room.position.x, room.position.y, 0.0);
    let local = point - origin;
    if source == FLOOR_SOURCE {
        let slab = (local.z + FLOOR_THICKNESS * 0.5).abs() - FLOOR_THICKNESS * 0.5;
        footprint(room, local.xy()).max(slab)
    } else if source == WALL_SOURCE {
        wall_distance(room, local, footprint(room, local.xy()), point)
    } else {
        let item = room.furniture[(source - FURNITURE_SOURCE) as usize];
        let center = origin + vec3(item.pos.x, -item.pos.y, item.pos.z + item.size.z * 0.5);
        render_furniture(item.kind, unrotate_z(point - center, item.rotation), item.size).distance
    }
}

fn room_normal(room: &Room, point: Vec3, epsilon: f32, source: u32) -> Vec3 {
    if source == FLOOR_SOURCE && point.z >= -epsilon {
        return Vec3::Z;
    }
    let directions = [vec3(1.0, -1.0, -1.0), vec3(-1.0, -1.0, 1.0), vec3(-1.0, 1.0, -1.0), Vec3::ONE];
    let mut normal = Vec3::ZERO;
    let mut index = 0;
    while index < directions.len() {
        let direction = directions[index];
        normal += direction * surface_distance(room, point + direction * epsilon, source);
        index += 1;
    }
    let length = normal.length();
    if length > 0.0 { normal / length } else { Vec3::Z }
}

fn background(origin: Vec3, direction: Vec3) -> Vec3 {
    let sky = vec3(0.62, 0.79, 0.96).lerp(vec3(0.12, 0.39, 0.81), direction.z.max(0.0).sqrt());
    if direction.z >= 0.0 {
        return sky;
    }
    let distance = (-origin.z - 0.18) / direction.z;
    if distance <= 0.0 {
        return sky;
    }
    let point = origin + direction * distance;
    let grid = ((point.x.floor() as i32 + point.y.floor() as i32) & 1) as f32;
    let ground = vec3(0.45, 0.48, 0.51) + Vec3::splat(grid * 0.025);
    ground.lerp(sky, (distance * 0.003).clamp(0.0, 0.6))
}

fn clip_to_room(bound: Bounds, origin: Vec3, direction: Vec3) -> (f32, f32) {
    let slab = |point: f32, ray: f32, low: f32, high: f32| {
        if ray.abs() < 1.0e-6 {
            if point < low || point > high { (1.0, 0.0) } else { (-FAR, FAR) }
        } else {
            let a = (low - point) / ray;
            let b = (high - point) / ray;
            (a.min(b), a.max(b))
        }
    };
    let (x0, x1) = slab(origin.x, direction.x, bound.min.x, bound.max.x);
    let (y0, y1) = slab(origin.y, direction.y, bound.min.y, bound.max.y);
    let (z0, z1) = slab(origin.z, direction.z, -FLOOR_THICKNESS, WALL_HEIGHT);
    (x0.max(y0).max(z0).max(0.0), x1.min(y1).min(z1))
}

fn trace_room(room: &Room, origin: Vec3, direction: Vec3, enter: f32, exit: f32, pixel_angle: f32) -> f32 {
    let mut furniture_mask = 0;
    let room_origin = vec3(room.position.x, room.position.y, 0.0);
    for offset in 0..room.furniture.len() {
        let item = room.furniture[offset];
        let center = room_origin + vec3(item.pos.x, -item.pos.y, item.pos.z + item.size.z * 0.5);
        let along = (center - origin).dot(direction).clamp(enter, exit);
        let radius = item.size.length() * 0.5 + 0.1;
        if (origin + direction * along - center).length_squared() < radius * radius {
            furniture_mask |= 1 << offset;
        }
    }
    let mut travelled = enter;
    for _ in 0..MAX_STEPS {
        let point = origin + direction * travelled;
        let mut ignored = FLOOR_SOURCE;
        let current = room_hit(room, point, furniture_mask, &mut ignored);
        let epsilon = (travelled * pixel_angle).max(MIN_HIT_DISTANCE);
        if current.distance < epsilon {
            return travelled;
        }
        travelled += current.distance;
        if travelled > exit {
            break;
        }
    }
    FAR
}

pub fn shade(globals: Globals, rooms: &[Room], bounds: &[Bounds], origin: Vec3, direction: Vec3) -> Vec4 {
    let background = background(origin, direction);
    let mut nearest = FAR;
    let mut nearest_room = 0;
    for index in 0..bounds.len() {
        let (enter, exit) = clip_to_room(bounds[index], origin, direction);
        if enter >= exit || enter >= nearest {
            continue;
        }
        let room = rooms[index];
        let travelled = trace_room(&room, origin, direction, enter, exit.min(nearest), globals.pixel_angle);
        if travelled < nearest {
            nearest = travelled;
            nearest_room = index;
        }
    }
    if nearest >= FAR {
        return background.extend(1.0);
    }

    let point = origin + direction * nearest;
    let room = rooms[nearest_room];
    let mut source = FLOOR_SOURCE;
    let surface = room_hit(&room, point, u32::MAX, &mut source);
    let normal = room_normal(&room, point, NORMAL_OFFSET, source);
    let diffuse = normal.dot(SUN).max(0.0);
    let ambient = 0.32 + 0.18 * normal.z.max(0.0);
    let base = materials::color(surface.material, surface.tint, point, nearest * globals.pixel_angle);
    let color = base * (ambient + diffuse * 0.7);
    color.lerp(background, 1.0 - 1.0 / (1.0 + nearest * 0.012)).extend(1.0)
}
