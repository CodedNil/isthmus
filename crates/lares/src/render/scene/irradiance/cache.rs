//! Geometry transport and per-fixture bounce bases never change with the clock.
//! Sun visibility refreshes incrementally; eight diffuse bounces use no temporal feedback.
use super::{Grid, Irradiance, Probe, SPACING};
use crate::{
    home::{Kind, Material, Object},
    render::{
        Sun, WALL_HEIGHT,
        acceleration::{BRANCH, Node},
        scene::{self, FAR, fixtures},
    },
};
use core::{array::from_fn, f32::consts::PI};
use isthmus::glam::{IVec3, UVec3, Vec2, Vec3, uvec3, vec3};

const RAYS: usize = 128;
const UPDATE_BUDGET: usize = 64;
const BOUNCES: usize = 8;

impl Irradiance {
    fn project(&mut self, direction: Vec3, radiance: Vec3) {
        // Window the first SH band to a nonnegative reconstruction kernel.
        // This preserves mean energy and source linearity, while suppressing
        // negative colour lobes and square-looking bright/dark probe patches.
        self.constant += radiance / RAYS as f32;
        self.x += radiance * (direction.x / RAYS as f32);
        self.y += radiance * (direction.y / RAYS as f32);
        self.z += radiance * (direction.z / RAYS as f32);
    }

    fn add(&mut self, other: Self, weight: Vec3) {
        self.constant += other.constant * weight;
        self.x += other.x * weight;
        self.y += other.y * weight;
        self.z += other.z * weight;
    }
}
#[derive(Clone, Copy, Default)]
struct Bounce {
    point: Vec3,
    normal: Vec3,
    albedo: Vec3,
}

pub struct Cache {
    directions: [Vec3; RAYS],
    paths: Box<[Bounce]>,
    sky: Box<[Irradiance]>,
    ground: Box<[Irradiance]>,
    sun: Box<[Irradiance]>,
    fixtures: Box<[Irradiance]>,
    /// Fixed direct sources and their first bounce, shared by every spatial iteration.
    direct: Box<[Probe]>,
    raw: Box<[Probe]>,
    bounces: [Box<[Probe]>; BOUNCES - 2],
    active: Box<[usize]>,
    grid: Grid,
    lights: Box<[u32]>,
    cursor: usize,
    remaining: usize,
}
impl Cache {
    fn sun_at(&self, index: usize, sun: Sun, objects: &[Object], nodes: &[Node]) -> Irradiance {
        let mut result = Irradiance::default();
        if sun.radiance.max_element() <= 0.0 {
            return result;
        }
        for (sample, bounce) in self.paths[index * RAYS..(index + 1) * RAYS].iter().enumerate() {
            let cosine = bounce.normal.dot(sun.direction).max(0.0);
            if cosine <= 0.0 || bounce.albedo == Vec3::ZERO {
                continue;
            }
            let visibility = scene::cast::<true, false>(bounce.point, sun.direction, nodes, objects, 0.0, FAR).0;
            result.project(self.directions[sample], bounce.albedo * (cosine * visibility / PI));
        }
        result
    }

    fn compose_at(&mut self, index: usize, sun: Sun, objects: &[Object]) {
        let mut result = Irradiance::default();
        result.add(self.sky[index], sun.sky);
        // Analytic sunlit exterior ground seen through real apertures. No
        // outdoor probes: a neutral 20%-reflecting Lambertian ground assumption.
        let ground = (sun.radiance * sun.direction.z.max(0.0) + sun.sky * (0.85 * PI)) * (0.20 / PI);
        result.add(self.ground[index], ground);
        result.add(self.sun[index], sun.radiance);
        let count = self.lights.len();
        for (light, &object) in self.lights.iter().enumerate() {
            let fixture = objects[object as usize];
            result.add(self.fixtures[index * count + light], fixture.emission_color * fixture.emission);
        }
        self.raw[index].light = result;
    }

    /// Geometry-aware spatial denoising, before interpolation and further
    /// bounces. Fixed weights preserve source linearity and constant fields.
    fn filtered(&self, index: usize) -> Irradiance {
        let size = self.grid.size;
        let coordinate = uvec3(index as u32 % size.x, index as u32 / size.x % size.y, index as u32 / (size.x * size.y));
        let source = self.raw[index];
        let mut result = Irradiance::default();
        result.add(source.light, Vec3::splat(2.0));
        let mut total = 2.0;
        for axis in 0..3 {
            for sign in [-1, 1] {
                let mut neighbour = coordinate.as_ivec3();
                neighbour[axis] += sign;
                if neighbour.cmplt(IVec3::ZERO).any() || neighbour.cmpge(size.as_ivec3()).any() {
                    continue;
                }
                let neighbour = neighbour.as_uvec3();
                let probe = self.raw[(neighbour.x + size.x * (neighbour.y + size.y * neighbour.z)) as usize];
                let delta = probe.position - source.position;
                if delta.cmpgt(source.positive).any()
                    || (-delta).cmpgt(source.negative).any()
                    || (-delta).cmpgt(probe.positive).any()
                    || delta.cmpgt(probe.negative).any()
                {
                    continue;
                }
                let weight = 1.0 / (1.0 + delta.length_squared() / (SPACING * SPACING));
                result.add(probe.light, Vec3::splat(weight));
                total += weight;
            }
        }
        let mut normalized = Irradiance::default();
        normalized.add(result, Vec3::splat(1.0 / total));
        normalized
    }

    fn bounce(&self, index: usize, previous: &[Probe]) -> Probe {
        let mut result = self.direct[index].light;
        for (sample, bounce) in self.paths[index * RAYS..(index + 1) * RAYS].iter().enumerate() {
            if bounce.albedo == Vec3::ZERO {
                continue;
            }
            let incident = self.grid.sample(previous, bounce.point, bounce.normal);
            // Finite spatial iterations, always adding the fixed source term.
            // No ad-hoc energy loss and no feedback from previous frames.
            result.project(self.directions[sample], incident * bounce.albedo);
        }
        Probe { light: result, ..self.direct[index] }
    }

    /// State changes blend precomputed light bases, without casting any rays.
    pub(crate) fn compose(&mut self, sun: Sun, objects: &[Object], probes: &mut [Probe]) {
        for slot in 0..self.active.len() {
            self.compose_at(self.active[slot], sun, objects);
        }
        for &index in &self.active {
            self.direct[index].light = self.filtered(index);
        }
        for pass in 0..BOUNCES - 1 {
            for slot in 0..self.active.len() {
                self.bounce_at(pass, self.active[slot], probes);
            }
        }
    }

    fn bounce_at(&mut self, pass: usize, index: usize, probes: &mut [Probe]) {
        let previous = if pass == 0 { &self.direct } else { &self.bounces[pass - 1] };
        let result = self.bounce(index, previous);
        if pass < BOUNCES - 2 {
            self.bounces[pass][index] = result;
        } else {
            probes[index] = result;
        }
    }

    pub(crate) const fn updating(&self) -> bool {
        self.remaining != 0
    }

    pub(crate) fn begin_update(&mut self) {
        self.cursor = 0;
        self.remaining = self.active.len() * (BOUNCES + 1);
    }

    /// At most 64 probes / 8192 cached paths per frame, independent of resolution.
    pub(crate) fn step(&mut self, sun: Sun, objects: &[Object], nodes: &[Node], probes: &mut [Probe]) {
        if self.remaining == 0 {
            return;
        }
        let count = self.active.len();
        let pass = self.cursor / count;
        let start = self.cursor % count;
        let end = (start + UPDATE_BUDGET).min(count);
        for slot in start..end {
            let index = self.active[slot];
            if pass == 0 {
                self.sun[index] = self.sun_at(index, sun, objects, nodes);
                self.compose_at(index, sun, objects);
            } else if pass == 1 {
                self.direct[index].light = self.filtered(index);
            } else {
                self.bounce_at(pass - 2, index, probes);
            }
        }
        self.remaining -= end - start;
        self.cursor += end - start;
    }

    pub(crate) fn relight(&mut self, sun: Sun, objects: &[Object], nodes: &[Node], probes: &mut [Probe]) {
        for &index in &self.active {
            self.sun[index] = self.sun_at(index, sun, objects, nodes);
        }
        self.compose(sun, objects, probes);
        self.remaining = 0;
        self.cursor = 0;
    }
}

pub fn bake(
    objects: &[Object],
    nodes: &[Node],
    lights: &[u32],
    min: Vec3,
    max: Vec3,
    sun: Sun,
) -> (Grid, Box<[Probe]>, Cache) {
    let origin = vec3(min.x, min.y, 0.10);
    let size = ((vec3(max.x, max.y, 2.6) - origin) / SPACING).ceil().as_uvec3() + UVec3::ONE;
    let grid = Grid { origin, size };
    let count = (size.x * size.y * size.z) as usize;
    let directions = from_fn(|sample| {
        let h = 1.0 - 2.0 * (sample as f32 + 0.5) / RAYS as f32;
        let (sin, cos) = (sample as f32 * 2.399_963).sin_cos();
        let r = (1.0 - h * h).sqrt();
        vec3(r * cos, r * sin, h)
    });
    let mut cache = Cache {
        directions,
        paths: vec![Bounce::default(); count * RAYS].into_boxed_slice(),
        sky: vec![Irradiance::default(); count].into_boxed_slice(),
        ground: vec![Irradiance::default(); count].into_boxed_slice(),
        sun: vec![Irradiance::default(); count].into_boxed_slice(),
        fixtures: vec![Irradiance::default(); count * lights.len()].into_boxed_slice(),
        direct: vec![Probe::default(); count].into_boxed_slice(),
        raw: vec![Probe::default(); count].into_boxed_slice(),
        bounces: from_fn(|_| vec![Probe::default(); count].into_boxed_slice()),
        active: Box::default(),
        grid,
        lights: lights.into(),
        cursor: 0,
        remaining: 0,
    };
    let inside = |p: Vec3| {
        let mut index = 0;
        while index < nodes.len() {
            let node = nodes[index];
            if p.cmplt(node.min).any() || p.cmpgt(node.max).any() {
                index = node.skip as usize;
                continue;
            }
            index += 1;
            if node.object == BRANCH {
                continue;
            }
            let object = objects[node.object as usize];
            let local = scene::unrotate_z(p - object.pos, object.rotation);
            if scene::render_furniture::<scene::Distance>(object.kind, local, object.size).0 < -0.002 {
                return true;
            }
        }
        false
    };
    let trace = |from: Vec3, direction: Vec3| scene::cast::<false, false>(from, direction, nodes, objects, 0.0, FAR);
    let usable = |point: Vec3| {
        if point.z <= 0.025 || point.z >= WALL_HEIGHT - 0.01 || inside(point) {
            return false;
        }
        let (distance, object) = scene::architectural_cast(point, Vec3::Z, nodes, objects);
        distance < FAR && objects[object as usize].kind == Kind::Ceiling
    };
    for index in 0..count {
        let coordinate = uvec3(index as u32 % size.x, index as u32 / size.x % size.y, index as u32 / (size.x * size.y));
        let lattice = origin + coordinate.as_vec3() * SPACING;
        let mut point = lattice;
        // Keep interpolation sites near wall/ceiling intersections populated.
        // Bake at the nearest free indoor point instead of discarding sites
        // inside furniture or sampling exterior sky above the roof.
        if !usable(point) {
            let mut nearest = f32::INFINITY;
            for candidate in 0..27 {
                let offset =
                    vec3((candidate % 3) as f32 - 1.0, (candidate / 3 % 3) as f32 - 1.0, (candidate / 9) as f32 - 1.0)
                        * (SPACING * 0.5);
                let distance = offset.length_squared();
                if distance < nearest && usable(lattice + offset) {
                    nearest = distance;
                    point = lattice + offset;
                }
            }
            if !nearest.is_finite() {
                cache.direct[index].positive = Vec3::splat(-FAR);
                cache.direct[index].negative = Vec3::splat(-FAR);
                continue;
            }
        }
        cache.direct[index].position = point;
        let clearance = |axis: Vec3| (scene::architectural_cast(point, axis, nodes, objects).0 - 0.002).max(0.0);
        let axes = |sign: f32| vec3(clearance(Vec3::X * sign), clearance(Vec3::Y * sign), clearance(Vec3::Z * sign));
        (cache.direct[index].positive, cache.direct[index].negative) = (axes(1.0), axes(-1.0));
        for (sample, &direction) in directions.iter().enumerate() {
            let (distance, object) = trace(point, direction);
            if distance >= FAR {
                if direction.z >= 0.0 {
                    cache.sky[index].project(direction, Vec3::splat(0.25 + direction.z.sqrt() * 0.75));
                } else {
                    cache.ground[index].project(direction, Vec3::ONE);
                }
                continue;
            }
            let object = objects[object as usize];
            let hit = point + direction * distance;
            let local = scene::unrotate_z(hit - object.pos, object.rotation);
            let normal = scene::rotate_z(scene::object_normal(object, local), object.rotation);
            let albedo = average_albedo(object, local).min(Vec3::splat(0.85));
            let point = hit + normal * 0.002;
            cache.paths[index * RAYS + sample] = Bounce { point, normal, albedo };
            for (light, &fixture) in lights.iter().enumerate() {
                // Unit brightness: entity changes only scale the cached transport.
                let fixture = Object { emission: 1.0, emission_color: Vec3::ONE, ..objects[fixture as usize] };
                let (direction_to_light, radiance, distance_to_light) = fixtures::incident(fixture, point, Vec2::ZERO);
                let contribution = albedo * radiance * (normal.dot(direction_to_light).max(0.0) / PI);
                if contribution.max_element() < 0.0005 {
                    continue;
                }
                let visibility = scene::cast::<true, false>(
                    point,
                    direction_to_light,
                    nodes,
                    objects,
                    0.0,
                    distance_to_light - 0.002,
                )
                .0;
                cache.fixtures[index * lights.len() + light].project(direction, contribution * visibility);
            }
        }
    }
    cache.active = cache
        .direct
        .iter()
        .enumerate()
        .filter_map(|(index, probe)| (probe.positive.x >= 0.0).then_some(index))
        .collect();
    cache.raw = cache.direct.clone();
    cache.bounces = from_fn(|_| cache.direct.clone());
    let mut probes = cache.direct.clone();
    cache.relight(sun, objects, nodes, &mut probes);
    (grid, probes, cache)
}

fn average_albedo(object: Object, local: Vec3) -> Vec3 {
    let surface = scene::surface_at(object, local);
    let base = match surface.material {
        Material::Floorboards => vec3(0.15, 0.085, 0.045),
        Material::Wood => vec3(0.35, 0.18, 0.075),
        Material::Maple | Material::MapleHorizontal => vec3(0.68, 0.58, 0.42),
        Material::Tile => vec3(0.035, 0.04, 0.045),
        Material::Metal | Material::Mirror => Vec3::splat(0.08),
        Material::Emissive => Vec3::splat(0.04),
        _ => Vec3::ONE,
    };
    surface.tint.clamp(Vec3::ZERO, Vec3::ONE).powf(2.2) * base
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod transport_tests;
