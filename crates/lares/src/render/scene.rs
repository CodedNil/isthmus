use super::{
    Globals,
    acceleration::{BRANCH, Node},
};
use crate::home::{Kind, Material, Object};
use isthmus::prelude::*;

mod bathroom;
mod bedroom;
mod decor;
mod entry;
pub(super) mod irradiance;
mod kitchen;
mod lighting;
mod living;
mod materials;
mod office;
mod trace;
mod utility;

#[derive(Clone, Copy)]
struct Hit {
    distance: f32,
    material: Material,
    tint: Vec3,
}

// Models share geometry; only the final surface query evaluates appearance.
trait Sample: Copy {
    fn new(distance: f32, surface: impl FnOnce() -> (Material, Vec3)) -> Self;
    fn distance(self) -> f32;
    fn subtract(self, distance: f32) -> Self;
    fn union(self, other: Self) -> Self {
        if other.distance() < self.distance() { other } else { self }
    }
}

#[derive(Clone, Copy)]
struct Distance(f32);

impl Sample for Distance {
    fn new(distance: f32, _: impl FnOnce() -> (Material, Vec3)) -> Self {
        Self(distance)
    }

    fn distance(self) -> f32 {
        self.0
    }

    fn subtract(self, distance: f32) -> Self {
        Self(self.0.max_num(-distance))
    }
}

impl Sample for Hit {
    fn new(distance: f32, surface: impl FnOnce() -> (Material, Vec3)) -> Self {
        let (material, tint) = surface();
        Self { distance, material, tint }
    }

    fn distance(self) -> f32 {
        self.distance
    }

    fn subtract(self, distance: f32) -> Self {
        Self { distance: self.distance.max_num(-distance), ..self }
    }
}
#[inline(never)]
fn cylinder(p: Vec3, radius: f32, height: f32) -> f32 {
    let d = vec2(p.xy().length() - radius, p.z.abs() - height * 0.5);
    d.max_num(Vec2::ZERO).length() + d.max_element().min_num(0.0)
}

#[inline(never)]
fn render_furniture<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    match kind {
        Kind::Box => S::new(box_sdf(local, size), || (Material::Paint, WALL_COLOR)),
        Kind::Bed => bedroom::bed(local, size),
        Kind::Sofa => living::sofa(local, size),
        Kind::Ottoman => living::ottoman(local, size),
        Kind::Rug => living::rug(local, size),
        Kind::Chair => office::chair(local, size),
        Kind::DiningChair => office::dining_chair(local, size),
        Kind::ShoeStorage => entry::shoe_storage(local, size),
        Kind::DisplayShelf => entry::shelf(local, size),
        Kind::WallSwords => entry::swords(local, size),
        Kind::Mirror => entry::mirror(local, size),
        Kind::PosterBlue => office::poster(local, size, false),
        Kind::Pegboard => office::pegboard(local, size),
        Kind::Poster | Kind::PosterNight => office::poster(local, size, kind == Kind::PosterNight),
        Kind::Table => office::table(local, size),
        Kind::Workstation => office::workstation(local, size),
        Kind::Bath | Kind::Shower | Kind::Sink | Kind::Toilet => bathroom::model(kind, local, size),
        Kind::Kettle => kitchen::kettle(local, size),
        Kind::EntryDoor => kitchen::entry_door(local, size),
        Kind::KitchenSink => kitchen::sink(local, size),
        Kind::KitchenPanel => {
            S::new(rounded_box(local, size, 0.003), || (Material::MapleHorizontal, vec3(0.98, 0.98, 0.96)))
        }
        Kind::KnifeBlock => kitchen::knife_block(local, size),
        Kind::Microwave => kitchen::microwave(local, size),
        Kind::CookerHood => kitchen::hood(local, size),
        Kind::Splashback => kitchen::splashback(local, size),
        Kind::Blind => kitchen::blind(local, size),
        Kind::TrailingPlant => decor::hanging_planter(local, size),
        Kind::Counter => kitchen::counter(local, size),
        Kind::Ceiling => S::new(box_sdf(local, size), || (Material::Paint, vec3(0.78, 0.76, 0.72))),
        Kind::DrawerUnit => kitchen::drawers(local, size),
        Kind::AirFryer => kitchen::air_fryer(local, size),
        Kind::Cupboard | Kind::Fridge => kitchen::cabinet(kind, local, size),
        Kind::Oven => kitchen::oven(local, size),
        Kind::Plant | Kind::Blossom | Kind::Palm | Kind::SillPlant => decor::plant(kind, local, size),
        Kind::Leaf | Kind::Petal | Kind::Stem => decor::blade(kind, local, size),
        Kind::SillArrangement => decor::sill_arrangement(local, size),
        Kind::SideTable => living::side_table(local, size),
        Kind::WallArt => decor::wall_art(local, size),
        Kind::Appliance | Kind::Radiator | Kind::Window | Kind::Curtain => utility::model(kind, local, size),
    }
}

const WALL_COLOR: Vec3 = vec3(0.86, 0.84, 0.8);
const MIN_HIT_DISTANCE: f32 = 0.0002;
const NORMAL_OFFSET: f32 = 0.0002;
const FAR: f32 = 1000.0;

fn unrotate_z(point: Vec3, rotation: Vec2) -> Vec3 {
    let (sin, cos) = (rotation.x, rotation.y);
    vec3(point.x * cos + point.y * sin, -point.x * sin + point.y * cos, point.z)
}

fn box_sdf(point: Vec3, size: Vec3) -> f32 {
    let offset = point.abs() - size * 0.5;
    offset.max_num(Vec3::ZERO).length() + offset.max_element().min_num(0.0)
}

#[inline(never)]
fn rounded_box(point: Vec3, size: Vec3, radius: f32) -> f32 {
    let radius = radius.min_num(size.min_element() * 0.5);
    let offset = point.abs() - size * 0.5 + radius;
    offset.max_num(Vec3::ZERO).length() + offset.max_element().min_num(0.0) - radius
}

#[inline(never)]
fn capsule(point: Vec3, start: Vec3, end: Vec3, radius: f32) -> f32 {
    let axis = end - start;
    let along = ((point - start).dot(axis) / axis.length_squared().max_num(1.0e-6)).max_num(0.0).min_num(1.0);
    (point - start - axis * along).length() - radius
}

fn stem_radius(size: Vec3) -> f32 {
    (size.length() * 0.018).max_num(0.001).min_num(0.004)
}

#[inline(never)]
fn taper(point: Vec3, start: Vec3, end: Vec3, start_radius: f32, end_radius: f32) -> f32 {
    let axis = end - start;
    let along = ((point - start).dot(axis) / axis.length_squared().max_num(1.0e-6)).max_num(0.0).min_num(1.0);
    let radius_delta = end_radius - start_radius;
    let distance = (point - start - axis * along).length() - (start_radius + radius_delta * along);
    // The varying radius raises the Lipschitz constant above one. Normalize
    // the bound without changing the zero surface, so relaxed steps remain safe.
    distance / (1.0 + radius_delta * radius_delta / axis.length_squared().max_num(1.0e-6)).sqrt()
}

fn blend(a: f32, b: f32, amount: f32) -> f32 {
    let k = amount.max_num(1.0e-5);
    let t = (1.0 - (a - b).abs() / k).max_num(0.0).min_num(1.0);
    a.min_num(b) - k * t * t * 0.25
}

fn scaled_box(local: Vec3, size: Vec3, center: Vec3, extent: Vec3, radius: f32) -> f32 {
    rounded_box(local - center * size, size * extent, radius * size.min_element())
}

fn background(origin: Vec3, direction: Vec3) -> Vec3 {
    if origin.z < super::WALL_HEIGHT {
        return vec3(0.84, 0.86, 0.88);
    }
    let sky = vec3(0.62, 0.79, 0.96).lerp(vec3(0.12, 0.39, 0.81), direction.z.max_num(0.0).sqrt());
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
    ground.lerp(sky, (distance * 0.003).max_num(0.0).min_num(0.6))
}

/// Const specialization shares source without putting a mode branch in the GPU loop.
#[inline(never)]
fn cast<const SHADOW: bool>(
    origin: Vec3,
    direction: Vec3,
    nodes: &[Node],
    objects: &[Object],
    pixel_angle: f32,
) -> (f32, u32) {
    let bounds_ray = trace::Ray::new(origin, direction);
    let mut index = 0;
    let mut nearest = FAR;
    let mut object = 0;
    while index < nodes.len() {
        let node = nodes[index];
        let (enter, exit) = bounds_ray.bounds(node.min, node.max);
        if enter > exit || enter >= nearest {
            index = node.skip as usize;
            continue;
        }
        index += 1;
        if node.object == BRANCH {
            continue;
        }
        let item = objects[node.object as usize];
        if item.kind == Kind::Ceiling && (SHADOW || origin.z > super::WALL_HEIGHT) {
            continue;
        }
        // Architectural boxes are their BVH leaf bounds: no second slab test,
        // transform or SDF evaluation is needed, including for shadow rays.
        if matches!(item.kind, Kind::Box | Kind::Ceiling) && item.rotation == Vec2::Y {
            let hit = if enter > 0.0 { enter } else { exit };
            if hit < nearest {
                if SHADOW {
                    return (0.0, node.object);
                }
                nearest = hit;
                object = node.object;
            }
            continue;
        }
        let hit = intersect(item, origin, direction, enter, exit.min_num(nearest), pixel_angle);
        if hit < nearest {
            if SHADOW {
                return (0.0, node.object);
            }
            nearest = hit;
            object = node.object;
        }
    }
    (if SHADOW { 1.0 } else { nearest }, object)
}

// Keep per-object intersection separate from buffer traversal so each ray shares one body.
#[inline(never)]
fn intersect(item: Object, origin: Vec3, direction: Vec3, enter: f32, exit: f32, pixel_angle: f32) -> f32 {
    let local = unrotate_z(origin - item.pos, item.rotation);
    let ray = unrotate_z(direction, item.rotation);
    match item.kind {
        Kind::Leaf => trace::leaf(local, ray, item.size, enter, exit),
        Kind::Stem => trace::stem(local, ray, item.size),
        _ => trace::march(local, ray, enter, exit, pixel_angle, |p| {
            render_furniture::<Distance>(item.kind, p, item.size).0
        }),
    }
}

#[inline(never)]
fn object_normal(item: Object, local: Vec3) -> Vec3 {
    if matches!(item.kind, Kind::Box | Kind::Ceiling) {
        let face = (local.abs() - item.size * 0.5).abs();
        if face.z <= face.x && face.z <= face.y {
            vec3(0.0, 0.0, if local.z >= 0.0 { 1.0 } else { -1.0 })
        } else if face.x <= face.y {
            vec3(if local.x >= 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0)
        } else {
            vec3(0.0, if local.y >= 0.0 { 1.0 } else { -1.0 }, 0.0)
        }
    } else if item.kind == Kind::Leaf {
        decor::leaf_normal(local, item.size)
    } else if item.kind == Kind::Stem {
        let along = ((local + item.size * 0.5).dot(item.size) / item.size.length_squared().max_num(1.0e-12))
            .max_num(0.0)
            .min_num(1.0);
        (local + item.size * 0.5 - item.size * along).normalize()
    } else {
        trace::normal(local, |p| render_furniture::<Distance>(item.kind, p, item.size).0)
    }
}

#[inline(never)]
pub fn shade(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    direction: Vec3,
    time: f32,
    light_sample: u32,
) -> Vec4 {
    let mut origin = globals.eye;
    let mut direction = direction;
    let (mut nearest, mut object) = cast::<false>(origin, direction, nodes, objects, globals.pixel_angle);
    let mut reflected_distance = 0.0;
    if nearest < FAR && objects[object as usize].kind == Kind::Mirror {
        let item = objects[object as usize];
        let point = origin + direction * nearest;
        let local = unrotate_z(point - item.pos, item.rotation);
        if entry::mirror::<Hit>(local, item.size).material == Material::Mirror {
            let normal = unrotate_z(object_normal(item, local), vec2(-item.rotation.x, item.rotation.y));
            direction -= normal * (2.0 * direction.dot(normal));
            origin = point + normal * 0.002;
            reflected_distance = nearest;
            (nearest, object) = cast::<false>(origin, direction, nodes, objects, globals.pixel_angle);
        }
    }
    if nearest >= FAR {
        return background(origin, direction).extend(1.0);
    }
    let point = origin + direction * nearest;
    let item = objects[object as usize];
    let local = unrotate_z(point - item.pos, item.rotation);
    let normal = object_normal(item, local);
    let world_normal = unrotate_z(normal, vec2(-item.rotation.x, item.rotation.y));

    let (surface, texture_point) = if item.kind == Kind::Box {
        let tint = if item.material == Material::Paint { WALL_COLOR } else { Vec3::ONE };
        (Hit::new(0.0, || (item.material, tint)), point)
    } else {
        (render_furniture::<Hit>(item.kind, local, item.size), local)
    };
    let surface = if item.kind == Kind::PosterBlue {
        let tint = surface.tint;
        Hit { tint: vec3(tint.z * 0.72, tint.x * 0.72 + tint.y * 0.28, tint.x * 0.55 + tint.z * 0.60), ..surface }
    } else {
        surface
    };
    let surface = if item.kind == Kind::Stem && item.material == Material::Foliage {
        Hit::new(surface.distance, || (Material::Foliage, vec3(0.25, 0.38, 0.12)))
    } else {
        surface
    };
    if surface.material == Material::Emissive {
        return lighting::display(office::wallpaper(local, time) * 2.0).extend(1.0);
    }
    // The four pixel samples also cover the emitter, keeping four shadow rays per pixel.
    let visibility = if world_normal.dot(lighting::KEY) > 0.0 {
        let start = point + world_normal * 0.001;
        let offset = vec2((light_sample & 1) as f32 - 0.5, (light_sample >> 1) as f32 - 0.5) * 0.05;
        let light = (lighting::KEY + offset.extend(0.0)).normalize();
        cast::<true>(start, light, nodes, objects, 0.0).0
    } else {
        0.0
    };
    let indirect = grid.sample(probes, point + world_normal * 0.015, world_normal);
    let indirect = if item.kind == Kind::Ceiling { indirect.lerp(vec3(0.38, 0.39, 0.40), 0.80) } else { indirect };
    // Surface footprint grows at grazing incidence; the screen-space ray
    // cone alone seriously underfilters upholstery on the cushion sides.
    let incidence = normal.dot(unrotate_z(-direction, item.rotation)).abs().max_num(0.10);
    let pixel = (nearest + reflected_distance) * globals.pixel_angle / incidence;
    // Keep procedural material state out of the traversal loops' live registers.
    let material = materials::sample(surface.material, surface.tint, texture_point, normal, pixel);
    let bumped = materials::bump(normal, material.gradient);
    let bumped = unrotate_z(bumped, vec2(-item.rotation.x, item.rotation.y));
    let daylight = if globals.eye.z < super::WALL_HEIGHT { 0.35 } else { 1.0 };
    let color = lighting::shade(material, bumped, -direction, visibility * daylight, indirect);
    lighting::display(color).extend(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_does_not_evaluate_appearance() {
        let distance = Distance::new(0.5, || panic!("distance query evaluated appearance"));
        let other = Distance::new(0.25, || panic!("distance query evaluated appearance"));
        assert_eq!(distance.union(other).subtract(-0.4).0, 0.4);

        let first = Hit::new(0.25, || (Material::Metal, Vec3::X));
        let second = Hit::new(0.25, || (Material::Wood, Vec3::Y));
        let hit = first.union(second);
        assert!(hit.material == Material::Metal);
        assert_eq!(hit.tint, Vec3::X);
    }
}
