use super::{
    Globals,
    acceleration::{BRANCH, Node},
    photometry,
};
use crate::home::{Kind, Material, Object};
use core::f32::consts::PI;
use isthmus::{prelude::*, spirv_std::arch::kill};
#[cfg(not(target_arch = "spirv"))]
use std::sync::OnceLock;

mod bathroom;
mod bedroom;
pub(super) mod contacts;
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
        Kind::GlassPane => S::new(box_sdf(local, size), || (Material::Ceramic, Vec3::ONE)),
        Kind::Box => S::new(box_sdf(local, size), || (Material::Paint, WALL_COLOR)),
        Kind::Downlight => fixtures::downlight(local, size),
        Kind::Lamp => fixtures::lamp(local, size),
        Kind::Bed => bedroom::bed(local, size),
        Kind::TowerFan | Kind::RoundSpeaker => bedroom::accessory(kind, local, size),
        Kind::Dresser | Kind::Nightstand | Kind::Wardrobe | Kind::Vanity => bedroom::cabinet(kind, local, size),
        Kind::Sofa => living::sofa(local, size),
        Kind::Pillow => living::pillow(local, size),
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
        Kind::Bath
        | Kind::Shower
        | Kind::Sink
        | Kind::Toilet
        | Kind::TowelRail
        | Kind::BathMat
        | Kind::ExtractorVent
        | Kind::WasteBin => bathroom::model(kind, local, size),
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
        Kind::Ceiling => S::new(box_sdf(local, size), || (Material::Paint, vec3(0.92, 0.91, 0.89))),
        Kind::DrawerUnit => kitchen::drawers(local, size),
        Kind::AirFryer => kitchen::air_fryer(local, size),
        Kind::Cupboard | Kind::Fridge => kitchen::cabinet(kind, local, size),
        Kind::Oven => kitchen::oven(local, size),
        Kind::Plant | Kind::Blossom | Kind::Palm | Kind::SillPlant | Kind::Alocasia => decor::plant(kind, local, size),
        Kind::Leaf | Kind::ShieldLeaf | Kind::Petal | Kind::Stem => decor::blade(kind, local, size),
        Kind::SillArrangement => decor::sill_arrangement(local, size),
        Kind::SideTable => living::side_table(local, size),
        Kind::WallArt => decor::wall_art(local, size),
        Kind::Appliance | Kind::Radiator | Kind::Window | Kind::Curtain | Kind::LinenStack => {
            utility::model(kind, local, size)
        }
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

/// Half-width of ordinary lens leaves or asymmetric, notched shield leaves.
/// Shared by growth, the SDF, analytic intersections and contact projection.
pub(super) fn leaf_profile(x: f32, length: f32, width: f32, shield: bool) -> f32 {
    let t = (x / length).clamp_num(-1.0, 1.0);
    width
        * if shield { ((0.935 - 0.31 * t - 0.625 * t * t) / 0.973_44).max_num(0.0) } else { (1.0 - t * t).max_num(0.0) }
}

pub(super) fn leaf_height(x: f32, y: f32, length: f32, width: f32, shield: bool) -> f32 {
    -0.18 * x * x / length + if shield { 0.32 * y * y / width } else { 0.32 * y.abs() }
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

fn background(origin: Vec3, sun: super::Sun, exposure: f32, white_balance: Vec3) -> Vec3 {
    if origin.z < super::WALL_HEIGHT {
        return lighting::display(sun.sky * white_balance * 1.5, exposure);
    }
    // The floor-plan surroundings are unlit context, not a rendered world.
    vec3(0.72, 0.77, 0.82)
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
            || (item.kind == Kind::GlassPane && (SHADOW || !CUTAWAY))
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

/// Nearest opaque surface from an emitter, excluding the fittings themselves.
/// Used only when baking camera-independent SDF shadow depth.
pub(super) fn light_depth(origin: Vec3, direction: Vec3, nodes: &[Node], objects: &[Object], limit: f32) -> f32 {
    let ray = trace::Ray::new(origin, direction);
    let mut index = 0;
    let mut nearest = limit;
    while index < nodes.len() {
        let node = nodes[index];
        let (enter, exit) = ray.bounds(node.min, node.max);
        if enter > exit || enter >= nearest {
            index = node.skip as usize;
            continue;
        }
        index += 1;
        if node.object == BRANCH {
            continue;
        }
        let item = objects[node.object as usize];
        if item.is_light() || item.kind == Kind::GlassPane {
            continue;
        }
        let hit = if matches!(item.kind, Kind::Box | Kind::Ceiling) && item.rotation == Vec2::Y {
            if enter > 0.0 { enter } else { exit }
        } else {
            intersect(item, origin, direction, enter, exit.min_num(nearest), 0.0)
        };
        nearest = nearest.min_num(hit);
    }
    nearest
}

/// Structural visibility bounds for the indoor probe field. Furniture must
/// not turn a cabinet's top edge into an infinitely tall occluding plane.
#[cfg(not(target_arch = "spirv"))]
pub(super) fn architectural_cast(origin: Vec3, direction: Vec3, nodes: &[Node], objects: &[Object]) -> (f32, u32) {
    let ray = trace::Ray::new(origin, direction);
    let mut index = 0;
    let mut nearest = FAR;
    let mut object = 0;
    while index < nodes.len() {
        let node = nodes[index];
        let (enter, exit) = ray.bounds(node.min, node.max);
        if enter > exit || enter >= nearest {
            index = node.skip as usize;
            continue;
        }
        index += 1;
        if node.object == BRANCH {
            continue;
        }
        let item = objects[node.object as usize];
        if matches!(item.kind, Kind::Box | Kind::Ceiling) && item.rotation == Vec2::Y {
            nearest = if enter > 0.0 { enter } else { exit };
            object = node.object;
        }
    }
    (nearest, object)
}

// Share per-object intersection across all BVH ray queries.
#[inline(never)]
fn intersect(item: Object, origin: Vec3, direction: Vec3, enter: f32, exit: f32, pixel_angle: f32) -> f32 {
    let local = unrotate_z(origin - item.pos, item.rotation);
    let ray = unrotate_z(direction, item.rotation);
    match item.kind {
        Kind::GlassPane => {
            let (near, far) = trace::Ray::new(local, ray).bounds(-item.size * 0.5, item.size * 0.5);
            if near <= far { near } else { FAR }
        }
        Kind::Leaf | Kind::ShieldLeaf => trace::leaf(local, ray, item.size, enter, exit, item.kind == Kind::ShieldLeaf),
        Kind::Stem => trace::stem(local, ray, item.size),
        _ => trace::march(local, ray, enter, exit, pixel_angle, |p| {
            render_furniture::<Distance>(item.kind, p, item.size).0
        }),
    }
}

#[inline(never)]
fn bounds_normal(local: Vec3, size: Vec3) -> Vec3 {
    let face = (local.abs() - size * 0.5).abs();
    if face.z <= face.x && face.z <= face.y {
        vec3(0.0, 0.0, if local.z >= 0.0 { 1.0 } else { -1.0 })
    } else if face.x <= face.y {
        vec3(if local.x >= 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0)
    } else {
        vec3(0.0, if local.y >= 0.0 { 1.0 } else { -1.0 }, 0.0)
    }
}

#[inline(never)]
fn object_normal(item: Object, local: Vec3) -> Vec3 {
    if matches!(item.kind, Kind::Box | Kind::Ceiling | Kind::GlassPane) {
        bounds_normal(local, item.size)
    } else if matches!(item.kind, Kind::Leaf | Kind::ShieldLeaf) {
        decor::leaf_normal(local, item.size, item.kind == Kind::ShieldLeaf)
    } else if item.kind == Kind::Stem {
        let along = ((local + item.size * 0.5).dot(item.size) / item.size.length_squared().max_num(1.0e-12))
            .max_num(0.0)
            .min_num(1.0);
        (local + item.size * 0.5 - item.size * along).normalize()
    } else {
        trace::normal(local, |p| render_furniture::<Distance>(item.kind, p, item.size).0)
    }
}

// Four half-float channels carry 59 payload bits without denormals or infinities.
// Each channel encodes 15 bits in sign, four exponent bits and ten mantissa bits.
pub(super) const VISIBILITY_OBJECTS: u32 = 8191;
const SUN_VISIBLE: u32 = 1 << 13;
const REFLECTED: u32 = 1 << 14;
const GLAZED: u32 = 1 << 15;
const fn half_payload(value: u32) -> f32 {
    f32::from_bits(((value & 16384) << 17) | ((((value >> 10) & 15) + 113) << 23) | ((value & 1023) << 13))
}
const fn read_half_payload(value: f32) -> u32 {
    let bits = value.to_bits();
    ((bits >> 17) & 16384) | ((((bits >> 23) & 255) - 113) << 10) | ((bits >> 13) & 1023)
}
fn octahedral(normal: Vec3) -> Vec2 {
    let normal = normal / normal.abs().element_sum().max_num(0.00001);
    let signs = vec2(if normal.x >= 0.0 { 1.0 } else { -1.0 }, if normal.y >= 0.0 { 1.0 } else { -1.0 });
    if normal.z < 0.0 { (Vec2::ONE - normal.yx().abs()) * signs } else { normal.xy() }
}
fn decode_normal(value: u32) -> Vec3 {
    let plane = uvec2(value & 511, value >> 9).as_vec2() * (2.0 / 511.0) - Vec2::ONE;
    let normal = plane.extend(1.0 - plane.abs().element_sum());
    let correction = (-normal.z).clamp_num(0.0, 1.0);
    vec3(
        normal.x + if normal.x >= 0.0 { -correction } else { correction },
        normal.y + if normal.y >= 0.0 { -correction } else { correction },
        normal.z,
    )
    .normalize()
}
fn encode_visibility(object: u32, flags: u32, height: f32, normal: Vec3, contact: f32) -> Vec4 {
    let height = ((height + 0.2) * (131_071.0 / 3.2)).round().clamp_num(0.0, 131_071.0) as u32;
    let normal = ((octahedral(normal) * 0.5 + Vec2::splat(0.5)) * 511.0).round().as_uvec2();
    let normal = normal.x | (normal.y << 9);
    let contact = (contact.clamp_num(0.0, 1.0) * 255.0).round() as u32;
    vec4(
        half_payload(object | (flags & (SUN_VISIBLE | REFLECTED))),
        half_payload(height & 32767),
        half_payload((height >> 15) | ((flags & GLAZED) >> 13) | (contact << 3) | ((normal >> 14) << 11)),
        half_payload((normal & 16383) | ((flags & (GLAZED << 1)) >> 2)),
    )
}
fn primary_direction(globals: Globals, pixel: Vec2, size: Vec2, pixel_size: f32) -> Vec3 {
    let ndc = (pixel + subpixel(globals.sample_index) * 0.5 * pixel_size) / size * 2.0 - 1.0;
    (globals.forward + globals.right * ndc.x - globals.up * ndc.y).normalize()
}

pub(super) fn visibility_pixel(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    contacts: &[u32],
    pixel: Vec2,
    size: Vec2,
    pixel_size: f32,
) -> Vec4 {
    let mut origin = globals.eye;
    let mut direction = primary_direction(globals, pixel, size, pixel_size);
    let mut flags = 0;
    let mut crossing = 0;
    while crossing < 6 {
        crossing += 1;
        let (distance, object) = cast::<false, true>(origin, direction, nodes, objects, globals.pixel_angle, FAR);
        if distance >= FAR {
            break;
        }
        let item = objects[object as usize];
        let point = origin + direction * distance;
        let local = unrotate_z(point - item.pos, item.rotation);
        if item.kind == Kind::GlassPane {
            let normal = rotate_z(bounds_normal(local, item.size), item.rotation).abs();
            let axis = if normal.x > 0.5 {
                1
            } else if normal.y > 0.5 {
                2
            } else {
                3
            };
            flags = (flags & !(GLAZED * 3)) | (axis << 15);
            let ray =
                trace::Ray::new(unrotate_z(origin - item.pos, item.rotation), unrotate_z(direction, item.rotation));
            let (_, exit) = ray.bounds(-item.size * 0.5, item.size * 0.5);
            origin += direction * (exit + 0.0005);
            continue;
        }
        if flags & REFLECTED == 0
            && item.kind == Kind::Mirror
            && entry::mirror::<Hit>(local, item.size).material == Material::Mirror
        {
            let normal = rotate_z(object_normal(item, local), item.rotation);
            direction -= normal * (2.0 * direction.dot(normal));
            origin = point + normal * 0.002;
            flags |= REFLECTED;
            continue;
        }
        let normal = rotate_z(object_normal(item, local), item.rotation);
        let cap = globals.eye.z > super::WALL_HEIGHT
            && point.z >= super::WALL_HEIGHT - 0.001
            && normal.z > 0.5
            && matches!(item.kind, Kind::Box | Kind::Downlight);
        if globals.sun.radiance.max_element() > 0.0 && normal.dot(globals.sun.direction) > 0.0 {
            let start = point + normal * 0.001 + Vec3::Z * if cap { super::CEILING_THICKNESS + 0.02 } else { 0.0 };
            let light = (globals.sun.direction + (subpixel(globals.sample_index) * 0.05).extend(0.0)).normalize();
            if cast::<true, false>(start, light, nodes, objects, 0.0, FAR).0 > 0.0 {
                flags |= SUN_VISIBLE;
            }
        }
        let contact = if cap || item.exterior.dot(normal) > 0.5 {
            1.0
        } else {
            contacts::visibility(
                contacts,
                objects,
                object,
                point,
                if item.kind == Kind::Rug { Vec3::Z } else { normal },
                globals.sample_index,
            )
        };
        return encode_visibility(object, flags, point.z, normal, contact);
    }
    encode_visibility(VISIBILITY_OBJECTS, 0, 0.0, Vec3::Z, 1.0)
}

#[derive(Clone, Copy)]
struct VisibleSurface {
    object: u32,
    flags: u32,
    normal: Vec3,
    contact: f32,
    origin: Vec3,
    direction: Vec3,
    nearest: f32,
    surface_height: f32,
    reflected_distance: f32,
}
fn visible_surface(
    globals: Globals,
    objects: &[Object],
    pixel: Vec2,
    size: Vec2,
    pixel_size: f32,
    record: Vec4,
    mirror: u32,
) -> VisibleSurface {
    let word = read_half_payload(record.x);
    let extra = read_half_payload(record.z);
    let object = word & VISIBILITY_OBJECTS;
    let height = (read_half_payload(record.y) | ((extra & 3) << 15)) as f32 * (3.2 / 131_071.0) - 0.2;
    let normal = decode_normal((read_half_payload(record.w) & 16383) | ((extra >> 11) << 14));
    let contact = ((extra >> 3) & 255) as f32 / 255.0;
    let mut origin = globals.eye;
    let mut direction = primary_direction(globals, pixel, size, pixel_size);
    let mut surface_height = (height / 3.0).clamp_num(0.0, 1.0);
    let reflected_distance = if word & REFLECTED != 0 {
        let item = objects[mirror as usize];
        let local = unrotate_z(origin - item.pos, item.rotation);
        let ray = unrotate_z(direction, item.rotation);
        let plane = if ray.y < 0.0 { 0.015 } else { 0.009 };
        let distance = (plane - local.y) / ray.y;
        let normal = rotate_z(vec3(0.0, if ray.y < 0.0 { 1.0 } else { -1.0 }, 0.0), item.rotation);
        let point = origin + direction * distance;
        surface_height = (point.z / 3.0).clamp_num(0.0, 1.0);
        origin = point + normal * 0.002;
        direction -= normal * (2.0 * direction.dot(normal));
        distance
    } else {
        0.0
    };
    let nearest = ((height - origin.z) / direction.z).max_num(0.0);
    VisibleSurface {
        object,
        flags: word | ((extra & 4) << 13) | ((read_half_payload(record.w) & 16384) << 2),
        normal,
        contact,
        origin,
        direction,
        nearest,
        surface_height,
        reflected_distance,
    }
}

pub(super) fn shade_visibility<const GROUP: u32>(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    size: Vec2,
    pixel_size: f32,
    time: f32,
    atlas: &Image,
    info: super::shadows::AtlasInfo,
    record: Vec4,
    mirror: u32,
    material: Vec4,
) -> Vec4 {
    let hit = visible_surface(globals, objects, pixel, size, pixel_size, record, mirror);
    let VisibleSurface {
        object,
        flags: word,
        normal,
        contact,
        origin,
        direction,
        nearest,
        surface_height,
        reflected_distance,
    } = hit;
    if object == VISIBILITY_OBJECTS {
        if GROUP > 1 {
            kill();
        }
        return background(globals.eye, globals.sun, globals.exposure, globals.white_balance).extend(0.0);
    }
    let (material, indirect) = decode_material(material);
    let family = materials::family(material.material);
    if GROUP != 0 && GROUP != family {
        kill();
    }
    let glass_axis = (word >> 15) & 3;
    let facing = match glass_axis {
        1 => direction.x.abs(),
        2 => direction.y.abs(),
        _ => direction.z.abs(),
    };
    let fresnel = if glass_axis != 0 { 0.078 + 0.922 * (1.0 - facing).powf(5.0) } else { 0.0 };
    let transmission =
        vec3(0.985, 0.995, 0.990).lerp(Vec3::ONE, if glass_axis == 0 { 1.0 } else { 0.0 }) * (1.0 - fresnel);
    let glass_light = indirect * (fresnel * 0.5 / PI);
    shade_surface::<true, true, GROUP>(
        globals,
        nodes,
        objects,
        lights,
        light_masks,
        contacts,
        grid,
        probes,
        origin,
        direction,
        nearest,
        object,
        surface_height,
        reflected_distance,
        glass_light,
        transmission,
        time,
        globals.sample_index,
        atlas,
        info,
        if word & SUN_VISIBLE != 0 { 1.0 } else { 0.0 },
        normal,
        contact,
        material,
        indirect,
    )
}

fn encode_material(surface: Hit, light: Vec3) -> Vec4 {
    let tint = (surface.tint.clamp_num(Vec3::ZERO, Vec3::ONE) * 255.0).round().as_uvec3();
    let material = tint.x | (tint.y << 8) | (tint.z << 16) | ((surface.material as u32) << 24);
    let exponent = light.max_element().max_num(1.0e-19).log2().ceil().clamp_num(-63.0, 63.0);
    let rgb = (light * (255.0 * (-exponent).exp2())).round().clamp_num(Vec3::ZERO, Vec3::splat(255.0)).as_uvec3();
    let illumination = rgb.x | (rgb.y << 8) | (rgb.z << 16) | (((exponent as i32 + 63) as u32) << 24);
    vec4(
        half_payload(material & 32767),
        half_payload((material >> 15) | ((illumination & 1) << 14)),
        half_payload((illumination >> 1) & 32767),
        half_payload(illumination >> 16),
    )
}
fn decode_material(record: Vec4) -> (Hit, Vec3) {
    let middle = read_half_payload(record.y);
    let material = read_half_payload(record.x) | ((middle & 16383) << 15);
    let light = (middle >> 14) | (read_half_payload(record.z) << 1) | (read_half_payload(record.w) << 16);
    let tint = uvec3(material & 255, (material >> 8) & 255, (material >> 16) & 255).as_vec3() / 255.0;
    let exponent = ((light >> 24) as i32 - 63) as f32;
    let rgb = uvec3(light & 255, (light >> 8) & 255, (light >> 16) & 255).as_vec3() * (exponent.exp2() / 255.0);
    (Hit::new(0.0, || (material_from_word(material >> 24), tint)), rgb)
}

fn visible_material(item: Object, local: Vec3) -> Hit {
    let mut surface = surface_at(item, local);
    if item.kind == Kind::PosterBlue {
        let tint = surface.tint;
        surface.tint = vec3(tint.z * 0.72, tint.x * 0.72 + tint.y * 0.28, tint.x * 0.55 + tint.z * 0.60);
    }
    if item.kind == Kind::Stem && item.material == Material::Foliage {
        (surface.material, surface.tint) = (Material::Foliage, vec3(0.25, 0.38, 0.12));
    }
    surface
}

pub(super) fn material_visibility(
    globals: Globals,
    objects: &[Object],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    size: Vec2,
    pixel_size: f32,
    record: Vec4,
    mirror: u32,
) -> Vec4 {
    let hit = visible_surface(globals, objects, pixel, size, pixel_size, record, mirror);
    if hit.object == VISIBILITY_OBJECTS {
        return encode_material(Hit::new(0.0, || (Material::Wood, Vec3::ZERO)), Vec3::ZERO);
    }
    let item = objects[hit.object as usize];
    let point = hit.origin + hit.direction * hit.nearest;
    let material = visible_material(item, unrotate_z(point - item.pos, item.rotation));
    let cap = globals.eye.z > super::WALL_HEIGHT
        && point.z >= super::WALL_HEIGHT - 0.001
        && hit.normal.z > 0.5
        && matches!(item.kind, Kind::Box | Kind::Downlight);
    let lighting_point = point + Vec3::Z * if cap { super::CEILING_THICKNESS + 0.02 } else { 0.0 };
    let cached = if item.surface_cache != u32::MAX && !cap {
        irradiance::Grid::surface(probes, item, unrotate_z(point - item.pos, item.rotation), hit.normal)
    } else {
        Vec4::ZERO
    };
    let light = if cached.w >= 0.95 {
        cached.truncate()
    } else {
        grid.sample(probes, lighting_point + hit.normal * 0.002, hit.normal)
            .lerp(cached.truncate(), (cached.w / 0.95).min_num(1.0))
    };
    encode_material(material, light)
}

const fn material_from_word(word: u32) -> Material {
    match word {
        1 => Material::Carpet,
        2 => Material::Tile,
        3 => Material::Fabric,
        4 => Material::Satin,
        5 => Material::Stone,
        6 => Material::Ceramic,
        7 => Material::Metal,
        8 => Material::Paint,
        9 => Material::Foliage,
        10 => Material::VariegatedFoliage,
        11 => Material::MarbledFoliage,
        12 => Material::Soil,
        13 => Material::Floorboards,
        14 => Material::Shag,
        15 => Material::Knit,
        16 => Material::Emissive,
        17 => Material::Mirror,
        18 => Material::Maple,
        19 => Material::MapleHorizontal,
        20 => Material::GlazedTile,
        21 => Material::BathroomTile,
        _ => Material::Wood,
    }
}

/// Averages four fixed subpixel rays in linear light, covering thin geometry without temporal jitter.
#[cfg(not(target_arch = "spirv"))]
pub fn pixel(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    screen_size: Vec2,
    pixel_size: f32,
    time: f32,
) -> Vec3 {
    static EMPTY: OnceLock<Image> = OnceLock::new();
    let image = EMPTY.get_or_init(|| Image::rgba8([1, 1], vec![0, 0, 0, 255]));
    pixel_impl::<false>(
        globals,
        nodes,
        objects,
        lights,
        light_masks,
        contacts,
        grid,
        probes,
        pixel,
        screen_size,
        pixel_size,
        time,
        image,
        super::shadows::AtlasInfo::new(0, 1),
    )
}

pub fn pixel_cached(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    screen_size: Vec2,
    pixel_size: f32,
    time: f32,
    atlas: &Image,
    info: super::shadows::AtlasInfo,
) -> Vec3 {
    pixel_impl::<true>(
        globals,
        nodes,
        objects,
        lights,
        light_masks,
        contacts,
        grid,
        probes,
        pixel,
        screen_size,
        pixel_size,
        time,
        atlas,
        info,
    )
}

#[cfg(test)]
pub fn pixel_single(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    screen_size: Vec2,
    pixel_size: f32,
    time: f32,
    atlas: &Image,
    info: super::shadows::AtlasInfo,
    sample: u32,
) -> Vec4 {
    let ndc = (pixel + subpixel(sample) * 0.5 * pixel_size) / screen_size * 2.0 - 1.0;
    let direction = (globals.forward + globals.right * ndc.x - globals.up * ndc.y).normalize();
    shade::<true>(
        globals,
        nodes,
        objects,
        lights,
        light_masks,
        contacts,
        grid,
        probes,
        direction,
        time,
        sample,
        atlas,
        info,
    )
}

/// One opaque reflection bounce for polished kitchen receivers. Roofs remain
/// solid and daylight can enter only through the authored window apertures.
#[inline(never)]
fn glossy_reflection<const CACHED: bool>(
    sun: super::Sun,
    point: Vec3,
    normal: Vec4,
    incoming: Vec4,
    nodes: &[Node],
    objects: &[Object],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
) -> Vec3 {
    let roughness = normal.w;
    let normal = normal.truncate();
    let sample = incoming.w as u32;
    let incoming = incoming.truncate();
    let reflected = incoming - normal * (2.0 * incoming.dot(normal));
    if CACHED {
        // The web viewer uses locally occluded, prefiltered transport instead
        // of embedding a second complete house tracer for glossy receivers.
        return grid.sample(probes, point + normal * 0.002, reflected) * (0.5 / PI);
    }
    let right = if reflected.z.abs() < 0.9 { Vec3::Z.cross(reflected).normalize() } else { Vec3::X };
    let up = reflected.cross(right);
    let spread = subpixel(sample) * (roughness * roughness * 2.0);
    let reflected = (reflected + right * spread.x + up * spread.y).normalize();
    let origin = point + normal * 0.002;
    let (distance, index) = cast::<false, false>(origin, reflected, nodes, objects, 0.003, FAR);
    if distance >= FAR {
        return if reflected.z >= 0.0 {
            sun.sky * (0.25 + reflected.z.sqrt() * 0.75)
        } else {
            (sun.radiance * sun.direction.z.max_num(0.0) + sun.sky * (0.85 * PI)) * (0.20 / PI)
        };
    }
    let item = objects[index as usize];
    let hit = origin + reflected * distance;
    let local = unrotate_z(hit - item.pos, item.rotation);
    if item.is_light() && item.emission > 0.0 {
        let luminance = if item.kind == Kind::Downlight {
            photometry::GU10_PEAK_CANDELA / (PI * (item.size.x * 0.225).powi(2) * photometry::UNITS)
        } else {
            470.0 / (4.0 * PI * PI * 0.065 * 0.065 * photometry::UNITS)
        };
        return item.emission_color * item.emission * luminance;
    }
    // Secondary rays retain the actual SDF silhouettes, but filter receiver
    // finishes to their mean. Re-evaluating full procedural furniture and six
    // normal samples here greatly increases shader register pressure, including
    // pixels which never need a glossy ray.
    let albedo = match item.kind {
        Kind::Box | Kind::Ceiling => match item.material {
            Material::Floorboards => vec3(0.15, 0.085, 0.045),
            Material::Paint => WALL_COLOR.powf(2.2),
            _ => Vec3::splat(0.50),
        },
        Kind::Leaf | Kind::ShieldLeaf | Kind::Stem => vec3(0.065, 0.15, 0.035),
        Kind::Petal => vec3(0.75, 0.35, 0.48),
        Kind::Sofa | Kind::Ottoman | Kind::Rug => vec3(0.65, 0.61, 0.52),
        Kind::Counter if local.z > item.size.z * 0.43 => Vec3::splat(0.025),
        Kind::Counter | Kind::Cupboard | Kind::Fridge | Kind::KitchenPanel | Kind::Workstation => {
            vec3(0.55, 0.43, 0.28)
        }
        Kind::EntryDoor | Kind::DisplayShelf | Kind::Table | Kind::DiningChair => vec3(0.22, 0.12, 0.055),
        Kind::Microwave | Kind::AirFryer | Kind::Chair | Kind::ShoeStorage => Vec3::splat(0.025),
        _ => Vec3::splat(0.50),
    };
    let local_normal = if matches!(item.kind, Kind::Leaf | Kind::ShieldLeaf) {
        decor::leaf_normal(local, item.size, item.kind == Kind::ShieldLeaf)
    } else {
        bounds_normal(local, item.size)
    };
    let normal = rotate_z(local_normal, item.rotation);
    albedo * grid.sample(probes, hit + normal * 0.002, normal)
}

fn pixel_impl<const CACHED: bool>(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    pixel: Vec2,
    screen_size: Vec2,
    pixel_size: f32,
    time: f32,
    atlas: &Image,
    info: super::shadows::AtlasInfo,
) -> Vec3 {
    let mut color = Vec3::ZERO;
    let mut sample = 0;
    while sample < 4 {
        let ndc = (pixel + subpixel(sample) * 0.5 * pixel_size) / screen_size * 2.0 - 1.0;
        let direction = (globals.forward + globals.right * ndc.x - globals.up * ndc.y).normalize();
        color += shade::<CACHED>(
            globals,
            nodes,
            objects,
            lights,
            light_masks,
            contacts,
            grid,
            probes,
            direction,
            time,
            sample,
            atlas,
            info,
        )
        .truncate()
        .powf(2.2);
        sample += 1;
    }
    (color * 0.25).powf(1.0 / 2.2)
}

/// Architectural boxes have a uniform finish; furniture evaluates its model.
#[inline(never)]
fn surface_at(item: Object, local: Vec3) -> Hit {
    if item.kind == Kind::Box {
        let tint = if item.material == Material::Paint { WALL_COLOR } else { Vec3::ONE };
        Hit::new(0.0, || (item.material, tint))
    } else {
        let mut hit = render_furniture::<Hit>(item.kind, local, item.size);
        if item.kind == Kind::Pillow {
            hit.material = item.material;
            hit.tint = if item.material == Material::Knit { vec3(0.34, 0.45, 0.48) } else { living::BLUE };
        }
        if item.kind == Kind::Blossom && item.material == Material::Ceramic && hit.material == Material::Ceramic {
            hit.tint = vec3(0.94, 0.93, 0.89);
        }
        if item.kind == Kind::Petal && item.material == Material::Ceramic {
            hit.tint = vec3(0.96, 0.95, 0.89);
        }
        if item.kind == Kind::Leaf && item.material == Material::VariegatedFoliage {
            let margin = (local.y.abs() / (item.size.y * 0.5)).clamp_num(0.0, 1.0);
            let patches = materials::noise(local.xy() * 65.0);
            let cream = ((margin - 0.47) * 3.5 + (patches - 0.5) * 1.1).clamp_num(0.0, 1.0);
            hit.tint = vec3(0.25, 0.37, 0.21).lerp(vec3(0.90, 0.89, 0.76), cream);
        }
        if item.kind == Kind::Stem && item.material == Material::Paint {
            hit.material = Material::Paint;
            hit.tint = vec3(0.34, 0.16, 0.14);
        }
        if item.material == Material::MarbledFoliage {
            if item.kind == Kind::Leaf {
                let q = decor::leaf_frame(local, item.size);
                let patches = materials::noise(q.xy() * 34.0 + vec2(q.x * 7.0, 0.0));
                let marble = ((patches - 0.48) * 3.0).clamp_num(0.0, 0.65);
                hit.tint = vec3(0.16, 0.32, 0.17).lerp(vec3(0.66, 0.76, 0.47), marble);
                hit.material = Material::MarbledFoliage;
            } else if item.kind == Kind::Stem {
                hit.tint = vec3(0.35, 0.17, 0.16);
            }
        }
        hit
    }
}

#[inline(never)]
fn shade<const CACHED: bool>(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    direction: Vec3,
    time: f32,
    light_sample: u32,
    atlas: &Image,
    info: super::shadows::AtlasInfo,
) -> Vec4 {
    let mut origin = globals.eye;
    let mut direction = direction;
    let mut surface_height = 0.0;
    let mut nearest = FAR;
    let mut object = 0;
    let mut reflected_distance = 0.0;
    // Parallel thin glazing transmits straight through; finite pane reflections
    // use the same one-bounce scene query as polished stone. No refractive tree.
    let mut glass_light = Vec3::ZERO;
    let mut transmission = Vec3::ONE;
    let mut reflected = false;
    for crossing in 0..6 {
        (nearest, object) = cast::<false, true>(origin, direction, nodes, objects, globals.pixel_angle, FAR);
        if crossing == 0 && nearest < FAR {
            surface_height = ((origin.z + direction.z * nearest) / 3.0).clamp_num(0.0, 1.0);
        }
        if crossing == 5 || nearest >= FAR {
            break;
        }
        if !reflected && objects[object as usize].kind == Kind::Mirror {
            let item = objects[object as usize];
            let point = origin + direction * nearest;
            let local = unrotate_z(point - item.pos, item.rotation);
            if entry::mirror::<Hit>(local, item.size).material == Material::Mirror {
                let normal = rotate_z(object_normal(item, local), item.rotation);
                direction -= normal * (2.0 * direction.dot(normal));
                origin = point + normal * 0.002;
                reflected_distance += nearest;
                reflected = true;
                continue;
            }
        }

        if objects[object as usize].kind != Kind::GlassPane {
            break;
        }
        let pane = objects[object as usize];
        let point = origin + direction * nearest;
        let local = unrotate_z(point - pane.pos, pane.rotation);
        let mut normal = rotate_z(bounds_normal(local, pane.size), pane.rotation);
        if normal.dot(direction) > 0.0 {
            normal = -normal;
        }
        let fresnel = 0.078 + 0.922 * (1.0 - normal.dot(-direction)).powf(5.0);
        glass_light += transmission
            * fresnel
            * glossy_reflection::<CACHED>(
                globals.sun,
                point + normal * 0.001,
                normal.extend(0.06),
                direction.extend(light_sample as f32),
                nodes,
                objects,
                grid,
                probes,
            );
        transmission *= vec3(0.985, 0.995, 0.990) * (1.0 - fresnel);
        let ray = trace::Ray::new(unrotate_z(origin - pane.pos, pane.rotation), unrotate_z(direction, pane.rotation));
        let (_, exit) = ray.bounds(-pane.size * 0.5, pane.size * 0.5);
        reflected_distance += exit + 0.0005;
        origin += direction * (exit + 0.0005);
    }
    shade_surface::<CACHED, false, 0>(
        globals,
        nodes,
        objects,
        lights,
        light_masks,
        contacts,
        grid,
        probes,
        origin,
        direction,
        nearest,
        object,
        surface_height,
        reflected_distance,
        glass_light,
        transmission,
        time,
        light_sample,
        atlas,
        info,
        0.0,
        Vec3::ZERO,
        1.0,
        Hit::new(0.0, || (Material::Wood, Vec3::ONE)),
        Vec3::ZERO,
    )
}

fn shade_surface<const CACHED: bool, const PRECOMPUTED: bool, const GROUP: u32>(
    globals: Globals,
    nodes: &[Node],
    objects: &[Object],
    lights: &[u32],
    light_masks: &[u32],
    contacts: &[u32],
    grid: irradiance::Grid,
    probes: &[irradiance::Probe],
    origin: Vec3,
    direction: Vec3,
    nearest: f32,
    object: u32,
    surface_height: f32,
    reflected_distance: f32,
    glass_light: Vec3,
    transmission: Vec3,
    time: f32,
    light_sample: u32,
    atlas: &Image,
    info: super::shadows::AtlasInfo,
    sun_visibility: f32,
    encoded_normal: Vec3,
    encoded_contact: f32,
    encoded_material: Hit,
    encoded_indirect: Vec3,
) -> Vec4 {
    if nearest >= FAR {
        return background(origin, globals.sun, globals.exposure, globals.white_balance).extend(surface_height);
    }
    let point = origin + direction * nearest;
    let item = objects[object as usize];
    let local = unrotate_z(point - item.pos, item.rotation);
    let normal = if PRECOMPUTED { unrotate_z(encoded_normal, item.rotation) } else { object_normal(item, local) };
    let world_normal = rotate_z(normal, item.rotation);
    let texture_point = if item.kind == Kind::Box { point } else { local };
    let mut surface = if PRECOMPUTED { encoded_material } else { visible_material(item, local) };
    // Section caps sit at the roof cut. Sample just above the hidden roof,
    // rather than treating the exposed wall cross-section as buried inside it.
    let section_cap = globals.eye.z > super::WALL_HEIGHT
        && point.z >= super::WALL_HEIGHT - 0.001
        && world_normal.z > 0.5
        && matches!(item.kind, Kind::Box | Kind::Downlight);
    let lighting_point = point + Vec3::Z * if section_cap { super::CEILING_THICKNESS + 0.02 } else { 0.0 };
    // The cutaway shows the downlight's luminous face from above as a state cue.
    if section_cap && item.kind == Kind::Downlight {
        let cue = if item.emission > 0.0 {
            lighting::display(item.emission_color * item.emission * 2.0, 0.95)
        } else {
            Vec3::splat(0.12)
        };
        return cue.extend(surface_height);
    }
    // Cutaway caps and outward wall faces are presentation context, not
    // lighting receivers. Do not sample probes, trace sun rays or visit lamps.
    if section_cap || item.exterior.dot(world_normal) > 0.5 {
        return lighting::display(surface.tint.powf(2.2) * 0.85, 0.95).extend(surface_height);
    }
    let fixture = item.is_light();
    if surface.material == Material::Emissive && (!fixture || item.emission > 0.0) {
        let emission = if fixture {
            let luminance = if item.kind == Kind::Downlight {
                // On-axis candela divided by the emitting lens area.
                photometry::GU10_PEAK_CANDELA / (PI * (item.size.x * 0.225).powi(2) * photometry::UNITS)
            } else {
                470.0 / (4.0 * PI * PI * 0.065 * 0.065 * photometry::UNITS)
            };
            item.emission_color * item.emission * luminance
        } else {
            office::wallpaper(local, time) * 2.0
        };
        return lighting::display((emission * transmission + glass_light) * globals.white_balance, globals.exposure)
            .extend(surface_height);
    }
    if fixture && surface.material == Material::Emissive {
        (surface.material, surface.tint) = (Material::Ceramic, vec3(0.85, 0.83, 0.78));
    }
    // The four pixel samples also cover the emitter, keeping four shadow rays per pixel.
    let visibility = if PRECOMPUTED {
        sun_visibility
    } else if globals.sun.radiance.max_element() > 0.0 && world_normal.dot(globals.sun.direction) > 0.0 {
        let start = lighting_point + world_normal * 0.001;
        let offset = subpixel(light_sample) * 0.05;
        let light = (globals.sun.direction + offset.extend(0.0)).normalize();
        cast::<true, false>(start, light, nodes, objects, 0.0, FAR).0
    } else {
        0.0
    };
    // Surface footprint grows at grazing incidence; the screen-space ray
    // cone alone seriously underfilters upholstery on the cushion sides.
    let local_view = unrotate_z(-direction, item.rotation);
    let pixel = (nearest + reflected_distance) * globals.pixel_angle;
    // Keep procedural material state out of the traversal loops' live registers.
    let mut material =
        materials::sample_group::<GROUP>(surface.material, surface.tint, texture_point, normal, local_view, pixel);
    if item.kind == Kind::ShieldLeaf {
        material.gradient += decor::leaf_veins(local, item.size, pixel);
        material.roughness = 0.33;
        material.coat = 0.12;
        material.coat_roughness = 0.28;
    }
    material.tangent = rotate_z(material.tangent, item.rotation);
    let bumped =
        materials::bump(normal, material.gradient, if surface.material == Material::Shag { 1.1 } else { 0.25 });
    // Unresolved fibres scatter around a shared nap normal. Retain the SDF
    // silhouette, while avoiding hard cylinder-like shading on every yarn.
    let bumped = if surface.material == Material::Shag { (bumped * 0.40 + Vec3::Z * 0.60).normalize() } else { bumped };
    let bumped = rotate_z(bumped, item.rotation);
    let indirect = if PRECOMPUTED {
        encoded_indirect
    } else {
        let cached = if item.surface_cache != u32::MAX && !section_cap {
            if surface.material == Material::Floorboards {
                irradiance::Grid::surface_normal(probes, item, local, world_normal, bumped)
            } else {
                irradiance::Grid::surface(probes, item, local, world_normal)
            }
        } else {
            Vec4::ZERO
        };
        if cached.w >= 0.95 {
            cached.truncate()
        } else {
            grid.sample(probes, lighting_point + world_normal * 0.002, world_normal)
                .lerp(cached.truncate(), (cached.w / 0.95).min_num(1.0))
        }
    };

    let contact_visibility =
        if matches!(surface.material, Material::Foliage | Material::VariegatedFoliage | Material::MarbledFoliage) {
            1.0
        } else {
            let receiver_normal = if surface.material == Material::Shag { Vec3::Z } else { world_normal };
            if PRECOMPUTED {
                encoded_contact
            } else {
                contacts::visibility(contacts, objects, object, point, receiver_normal, light_sample)
            }
        };
    let indirect = indirect * contact_visibility;

    let reflection =
        if !CACHED && matches!(surface.material, Material::Stone | Material::GlazedTile | Material::BathroomTile) {
            glossy_reflection::<CACHED>(
                globals.sun,
                point,
                bumped.extend(material.coat_roughness),
                direction.extend(light_sample as f32),
                nodes,
                objects,
                grid,
                probes,
            )
            .extend(1.0)
        } else {
            Vec4::ZERO
        };
    let mut color = lighting::shade(material, bumped, -direction, visibility, indirect, globals.sun, reflection);
    let mask = light_masks[object as usize];
    let mut slot = 0;
    while slot < lights.len() {
        let current = slot;
        slot += 1;
        // Slots beyond the mask remain conservatively visible.
        if current < 32 && mask & (1u32 << current) == 0 {
            continue;
        }
        let index = lights[current];
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
        let visible = if CACHED {
            super::shadows::visible(
                atlas,
                info,
                current as u32,
                light_sample,
                light,
                point + world_normal * 0.002,
                world_normal,
            )
        } else {
            cast::<true, false>(point + world_normal * 0.002, to_light, nodes, objects, 0.0, distance - 0.002).0
        };
        color += contribution * visible;
    }
    lighting::display((color * transmission + glass_light) * globals.white_balance, globals.exposure)
        .extend(surface_height)
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

/// Meter opaque scene surfaces in a small view grid. Windows/background are
/// excluded, so they cannot force the whole interior into silhouette.
#[cfg(not(target_arch = "spirv"))]
pub(super) fn camera_settings(scene: &super::Scene, eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) -> (f32, Vec3) {
    let mut illuminant = Vec3::ZERO;
    let mut log_sum = 0.0f32;
    let mut log_peak = -20.0f32;
    let mut samples = 0;
    for row in -1..=1 {
        for column in -1..=1 {
            let direction = (forward + right * (column as f32 * 0.65) + up * (row as f32 * 0.65)).normalize();
            let (distance, object) = cast::<false, true>(eye, direction, &scene.nodes, &scene.objects, 0.001, FAR);
            if distance >= FAR {
                continue;
            }
            let item = scene.objects[object as usize];
            let point = eye + direction * distance;
            let normal = rotate_z(object_normal(item, unrotate_z(point - item.pos, item.rotation)), item.rotation);
            let source = point + normal * 0.002;
            let cached = if item.surface_cache == u32::MAX {
                Vec4::ZERO
            } else {
                irradiance::Grid::surface(&scene.probes, item, unrotate_z(point - item.pos, item.rotation), normal)
            };
            let mut incident =
                scene.grid.sample(&scene.probes, source, normal).lerp(cached.truncate(), (cached.w / 0.95).min(1.0));
            let cosine = normal.dot(scene.sun.direction).max(0.0);
            if cosine > 0.0
                && cast::<true, false>(source, scene.sun.direction, &scene.nodes, &scene.objects, 0.0, FAR).0 > 0.0
            {
                incident += scene.sun.radiance * (cosine / PI);
            }
            for &index in &scene.lights {
                let light = scene.objects[index as usize];
                let (to_light, radiance, distance) = fixtures::incident(light, point, Vec2::ZERO);
                let cosine = normal.dot(to_light).max(0.0);
                if cosine > 0.0
                    && radiance.max_element() > 0.0
                    && cast::<true, false>(source, to_light, &scene.nodes, &scene.objects, 0.0, distance - 0.002).0
                        > 0.0
                {
                    incident += radiance * (cosine / PI);
                }
            }
            // Exposure of a conventional 18% grey receiver; do not compensate
            // for dark wood or white cabinets by baking albedo into the meter.
            let luminance = (incident.dot(vec3(0.2126, 0.7152, 0.0722)) * photometry::UNITS * 0.18).max(0.05);
            illuminant += incident / incident.dot(vec3(0.2126, 0.7152, 0.0722)).max(0.0001);
            let log = luminance.log2();
            log_sum += log;
            log_peak = log_peak.max(log);
            samples += 1;
        }
    }
    if samples == 0 {
        return (photometry::EXPOSURE, Vec3::ONE);
    }
    let average = log_sum / samples as f32;
    let ev = (3.0 + average + ((log_peak - average) * 0.25).min(1.5)).clamp_num(3.0, 11.0);
    let white_balance = (Vec3::splat(samples as f32) / illuminant.max(Vec3::splat(0.001)))
        .clamp_num(Vec3::splat(0.65), Vec3::splat(2.2));
    (photometry::UNITS / (1.2 * ev.exp2()), Vec3::ONE.lerp(white_balance, 0.85))
}
