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
mod fixtures;
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
    extrude(vec2(p.xy().length() - radius, p.z.abs() - height * 0.5))
}

/// Exact distance to the intersection of two independent slabs.
fn extrude(d: Vec2) -> f32 {
    d.max_num(Vec2::ZERO).length() + d.max_element().min_num(0.0)
}

#[inline(never)]
fn render_furniture<S: Sample>(kind: Kind, local: Vec3, size: Vec3) -> S {
    match kind {
        Kind::Box => S::new(box_sdf(local, size), || (Material::Paint, WALL_COLOR)),
        Kind::Downlight => fixtures::downlight(local, size),
        Kind::Lamp => fixtures::lamp(local, size),
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

fn rotate_z(point: Vec3, rotation: Vec2) -> Vec3 {
    unrotate_z(point, vec2(-rotation.x, rotation.y))
}

/// One of four fixed offsets in [-0.5, 0.5], shared by pixel, sun and emitter samples.
fn subpixel(sample: u32) -> Vec2 {
    vec2((sample & 1) as f32 - 0.5, (sample >> 1) as f32 - 0.5)
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

fn background(origin: Vec3, direction: Vec3, sun: super::Sun) -> Vec3 {
    if origin.z < super::WALL_HEIGHT {
        return lighting::display(sun.sky * 1.5);
    }
    let sky = lighting::display(sun.sky * (0.55 + direction.z.max_num(0.0).sqrt() * 0.7));
    if direction.z >= 0.0 {
        return sky;
    }
    let distance = (-origin.z - 0.18) / direction.z;
    if distance <= 0.0 {
        return sky;
    }
    let point = origin + direction * distance;
    let grid = ((point.x.floor() as i32 + point.y.floor() as i32) & 1) as f32;
    let ground = lighting::display((vec3(0.45, 0.48, 0.51) + Vec3::splat(grid * 0.025)) * sun.sky * 0.65);
    ground.lerp(sky, (distance * 0.003).max_num(0.0).min_num(0.6))
}

/// Const specialization shares source without putting a mode branch in the GPU loop.
#[inline(never)]
fn cast<const SHADOW: bool, const CUTAWAY: bool>(
    origin: Vec3,
    direction: Vec3,
    nodes: &[Node],
    objects: &[Object],
    pixel_angle: f32,
    limit: f32,
) -> (f32, u32) {
    let bounds_ray = trace::Ray::new(origin, direction);
    let mut index = 0;
    let mut nearest = limit;
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
        if (item.kind == Kind::Ceiling && CUTAWAY && !SHADOW && origin.z > super::WALL_HEIGHT)
            || (SHADOW && item.is_light())
        {
            continue;
        }
        // Architectural boxes are their BVH leaf bounds: no second slab test,
        // transform or SDF evaluation is needed, including for shadow rays.
        let hit = if matches!(item.kind, Kind::Box | Kind::Ceiling) && item.rotation == Vec2::Y {
            if enter > 0.0 { enter } else { exit }
        } else {
            intersect(item, origin, direction, enter, exit.min_num(nearest), pixel_angle)
        };
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
        if face.z <= face.x + MIN_HIT_DISTANCE && face.z <= face.y + MIN_HIT_DISTANCE {
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

/// Averages four fixed subpixel rays in linear light, covering thin geometry without temporal jitter.
pub fn pixel(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    screen_size: Vec2,
    pixel_size: f32,
    time: f32,
) -> Vec3 {
    let mut color = Vec3::ZERO;
    let mut sample = 0;
    while sample < 4 {
        let ndc = (pixel + subpixel(sample) * 0.5 * pixel_size) / screen_size * 2.0 - 1.0;
        let direction = (globals.forward + globals.right * ndc.x - globals.up * ndc.y).normalize();
        color += shade(globals, nodes, objects, lights, grid, probes, direction, time, sample).truncate().powf(2.2);
        sample += 1;
    }
    (color * 0.25).powf(1.0 / 2.2)
}

/// Architectural boxes have a uniform finish; furniture evaluates its model.
fn surface_at(item: Object, local: Vec3) -> Hit {
    if item.kind == Kind::Box {
        let tint = if item.material == Material::Paint { WALL_COLOR } else { Vec3::ONE };
        Hit::new(0.0, || (item.material, tint))
    } else {
        render_furniture::<Hit>(item.kind, local, item.size)
    }
}

#[inline(never)]
fn shade(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    direction: Vec3,
    time: f32,
    light_sample: u32,
) -> Vec4 {
    let mut origin = globals.eye;
    let mut direction = direction;
    let (mut nearest, mut object) = cast::<false, true>(origin, direction, nodes, objects, globals.pixel_angle, FAR);
    let mut reflected_distance = 0.0;
    if nearest < FAR && objects[object as usize].kind == Kind::Mirror {
        let item = objects[object as usize];
        let point = origin + direction * nearest;
        let local = unrotate_z(point - item.pos, item.rotation);
        if entry::mirror::<Hit>(local, item.size).material == Material::Mirror {
            let normal = rotate_z(object_normal(item, local), item.rotation);
            direction -= normal * (2.0 * direction.dot(normal));
            origin = point + normal * 0.002;
            reflected_distance = nearest;
            (nearest, object) = cast::<false, true>(origin, direction, nodes, objects, globals.pixel_angle, FAR);
        }
    }
    if nearest >= FAR {
        return background(origin, direction, globals.sun).extend(1.0);
    }
    let point = origin + direction * nearest;
    let item = objects[object as usize];
    let local = unrotate_z(point - item.pos, item.rotation);
    let normal = object_normal(item, local);
    let world_normal = rotate_z(normal, item.rotation);
    let texture_point = if item.kind == Kind::Box { point } else { local };
    let mut surface = surface_at(item, local);
    if item.kind == Kind::PosterBlue {
        let tint = surface.tint;
        surface.tint = vec3(tint.z * 0.72, tint.x * 0.72 + tint.y * 0.28, tint.x * 0.55 + tint.z * 0.60);
    }
    if item.kind == Kind::Stem && item.material == Material::Foliage {
        (surface.material, surface.tint) = (Material::Foliage, vec3(0.25, 0.38, 0.12));
    }
    // Section caps sit at the roof cut. Sample just above the hidden roof,
    // rather than treating the exposed wall cross-section as buried inside it.
    let section_cap = globals.eye.z > super::WALL_HEIGHT
        && point.z >= super::WALL_HEIGHT - 0.001
        && world_normal.z > 0.5
        && matches!(item.kind, Kind::Box | Kind::Downlight);
    let lighting_point = point + Vec3::Z * if section_cap { super::CEILING_THICKNESS + 0.02 } else { 0.0 };
    let fixture = item.is_light();
    // The cutaway shows the downlight's luminous face from above as a state cue.
    if section_cap && item.kind == Kind::Downlight && item.emission > 0.0 {
        return lighting::display(fixtures::WARM * item.emission * 2.0).extend(1.0);
    }
    if surface.material == Material::Emissive && (!fixture || item.emission > 0.0) {
        let emission = if fixture { surface.tint * item.emission * 4.0 } else { office::wallpaper(local, time) * 2.0 };
        return lighting::display(emission).extend(1.0);
    }
    if fixture && surface.material == Material::Emissive {
        (surface.material, surface.tint) = (Material::Ceramic, vec3(0.85, 0.83, 0.78));
    }
    // The four pixel samples also cover the emitter, keeping four shadow rays per pixel.
    let visibility = if globals.sun.radiance.max_element() > 0.0 && world_normal.dot(globals.sun.direction) > 0.0 {
        let start = lighting_point + world_normal * 0.001;
        let offset = subpixel(light_sample) * 0.05;
        let light = (globals.sun.direction + offset.extend(0.0)).normalize();
        cast::<true, false>(start, light, nodes, objects, 0.0, FAR).0
    } else {
        0.0
    };
    let indirect = grid.sample(probes, lighting_point + world_normal * 0.015, world_normal);

    // Surface footprint grows at grazing incidence; the screen-space ray
    // cone alone seriously underfilters upholstery on the cushion sides.
    let incidence = normal.dot(unrotate_z(-direction, item.rotation)).abs().max_num(0.10);
    let pixel = (nearest + reflected_distance) * globals.pixel_angle / incidence;
    // Keep procedural material state out of the traversal loops' live registers.
    let material = materials::sample(surface.material, surface.tint, texture_point, normal, pixel);
    let bumped = materials::bump(normal, material.gradient);
    let bumped = rotate_z(bumped, item.rotation);
    let mut color = lighting::shade(material, bumped, -direction, visibility, indirect, globals.sun);
    for &index in lights {
        let light = objects[index as usize];
        if light.emission <= 0.0 {
            continue;
        }
        // Reuse the four antialiasing samples across a finite emitter as well.
        let emitter = subpixel(light_sample) * if light.kind == Kind::Downlight { 0.055 } else { 0.035 };
        let (to_light, radiance, distance) = fixtures::incident(light, point, emitter);
        if radiance.max_element() <= 0.0 || bumped.dot(to_light) <= 0.0 {
            continue;
        }
        // Cull negligible/backfacing contributions before any expensive shadow traversal.
        let contribution = lighting::direct(material, bumped, -direction, to_light, radiance);
        if contribution.max_element() < 0.001 {
            continue;
        }
        let visible =
            cast::<true, false>(point + world_normal * 0.002, to_light, nodes, objects, 0.0, distance - 0.002).0;
        color += contribution * visible;
    }
    lighting::display(color).extend(1.0)
}
