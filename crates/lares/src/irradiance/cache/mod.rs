//! Static geometry transport, solved fixture responses and budgeted daylight.
pub(super) use crate::irradiance::cache::daylight::Packed;
use crate::{
    home::{Kind, Material, Object},
    irradiance::{Controls, Grid, Irradiance, Probe, SPACING, surface},
    render::{
        WALL_HEIGHT,
        geometry::acceleration::{BRANCH, Node},
        light::sun::Sun,
    },
    scene,
    scene::{FAR, lighting},
};
use core::{array::from_fn, f32::consts::PI};
use isthmus::{
    ShaderData,
    glam::{IVec3, UVec3, Vec2, Vec3, uvec3, vec3},
};
use std::collections::BTreeMap;

mod daylight;
mod transport;

const RAYS: usize = 256;
const UPDATE_BUDGET: usize = 64;
const BOUNCES: usize = 8;

impl Irradiance {
    fn project(&mut self, direction: Vec3, radiance: Vec3) {
        // Windows the first SH band to a nonnegative kernel, keeping mean energy and linearity.
        self.constant += radiance / RAYS as f32;
        self.x += radiance * (direction.x / RAYS as f32);
        self.y += radiance * (direction.y / RAYS as f32);
        self.z += radiance * (direction.z / RAYS as f32);
    }

    fn project_receiver(&mut self, direction: Vec3, radiance: Vec3, normal: Vec3) {
        if normal.length_squared() > 0.5 {
            let cosine = normal.dot(direction).max(0.0);
            self.constant += radiance * (4.0 * cosine / RAYS as f32);
            if cosine > 0.0 {
                // First derivative of the cosine integral, in world space, so bumped normals respond to bounced light.
                let incoming = radiance * (4.0 / RAYS as f32);
                self.x += incoming * direction.x;
                self.y += incoming * direction.y;
                self.z += incoming * direction.z;
            }
        } else {
            self.project(direction, radiance);
        }
    }

    fn add(&mut self, other: Self, weight: Vec3) {
        self.constant += other.constant * weight;
        self.x += other.x * weight;
        self.y += other.y * weight;
        self.z += other.z * weight;
    }
}

#[derive(Clone, Copy, Default, ShaderData)]
struct Bounce {
    object: u32,
    direction: Vec3,
    lookup: surface::Lookup,
    point: Vec3,
    normal: Vec3,
    albedo: Vec3,
    volume_offset: u32,
    volume_length: u32,
}

pub struct Cache {
    pub(super) daylight: Option<daylight::Daylight>,
    surface_slots: Box<[u32]>,
    surface_layouts: Box<[[u32; 6]]>,
    paths: Vec<Bounce>,
    volume_weights: Vec<(u32, f32)>,
    /// Solved unit response, keyed by the fixtures controlled together.
    pub(super) responses: BTreeMap<Vec<u32>, Box<[Packed]>>,
    applied: Vec<Vec3>,
    groups: Vec<Vec<u32>>,
    path_offsets: Box<[usize]>,
    path_lengths: Box<[usize]>,
    sky: Box<[Irradiance]>,
    ground: Box<[Irradiance]>,
    sun: Box<[Irradiance]>,
    fixtures: Box<[Irradiance]>,
    /// Fixed direct sources and their first bounce.
    direct: Box<[Probe]>,
    raw: Box<[Probe]>,
    bounces: [Box<[Probe]>; BOUNCES - 2],
    pub(super) active: Box<[usize]>,
    grid: Grid,
    surface_neighbours: Box<[[u32; 9]]>,
    volume_neighbours: Box<[[u32; 6]]>,
    lights: Box<[u32]>,
    cursor: usize,
    remaining: usize,
}

impl Cache {
    pub(crate) fn light_count(&self) -> usize {
        self.lights.len()
    }

    pub(crate) fn probe_count(&self) -> usize {
        self.direct.len()
    }

    pub(crate) fn statistics(&self) -> (usize, usize, usize) {
        if let Some(daylight) = &self.daylight {
            return (0, 0, daylight.bytes());
        }
        (
            self.paths.len(),
            self.volume_weights.len(),
            self.paths.len() * size_of::<Bounce>() + self.volume_weights.len() * size_of::<(u32, f32)>(),
        )
    }

    fn sun_at(&self, index: usize, sun: Sun, objects: &[Object], nodes: &[Node]) -> Irradiance {
        let mut result = Irradiance::default();
        if sun.radiance.max_element() <= 0.0 {
            return result;
        }
        for bounce in &self.paths[self.path_offsets[index]..self.path_offsets[index] + self.path_lengths[index]] {
            let cosine = bounce.normal.dot(sun.direction).max(0.0);
            if cosine <= 0.0 || bounce.albedo == Vec3::ZERO {
                continue;
            }
            let visibility = scene::cast::<true, false>(bounce.point, sun.direction, nodes, objects, 0.0, FAR).0;
            result.project_receiver(
                bounce.direction,
                bounce.albedo * (cosine * visibility / PI),
                self.direct[index].normal,
            );
        }
        result
    }

    fn daylight_at(&mut self, index: usize, sun: Sun) {
        let mut result = Irradiance::default();
        result.add(self.sky[index], sun.sky);
        // Exterior ground seen through apertures is analytic: neutral, 20% reflective, Lambertian.
        let ground = (sun.radiance * sun.direction.z.max(0.0) + sun.sky * (0.85 * PI)) * (0.20 / PI);
        result.add(self.ground[index], ground);
        result.add(self.sun[index], sun.radiance);
        self.raw[index].light = result;
    }

    /// Geometry-aware denoising with fixed weights, which keeps sources linear.
    fn filtered(&self, index: usize) -> Irradiance {
        if self.raw[index].normal.length_squared() > 0.5 {
            let lattice = (self.grid.size.x * self.grid.size.y * self.grid.size.z) as usize;
            let source = self.raw[index];
            let mut result = Irradiance::default();
            let mut total = 0.0;
            for (tap, &neighbour) in self.surface_neighbours[index - lattice].iter().enumerate() {
                if neighbour == u32::MAX {
                    continue;
                }
                let probe = self.raw[neighbour as usize];
                let agreement = probe.normal.dot(source.normal);
                let separation = (probe.position - source.position).dot(source.normal).abs();
                if agreement < 0.85 || separation > 0.05 {
                    continue;
                }
                let weight = if tap == 4 {
                    4.0
                } else if tap % 2 == 1 {
                    2.0
                } else {
                    1.0
                };
                result.add(probe.light, Vec3::splat(weight));
                total += weight;
            }
            let mut normalized = Irradiance::default();
            normalized.add(result, Vec3::splat(1.0 / total.max(1.0)));
            return normalized;
        }
        let source = self.raw[index];
        let mut result = Irradiance::default();
        result.add(source.light, Vec3::splat(2.0));
        let mut total = 2.0;
        for &neighbour in &self.volume_neighbours[index] {
            if neighbour == u32::MAX {
                continue;
            }
            let probe = self.raw[neighbour as usize];
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
        let mut normalized = Irradiance::default();
        normalized.add(result, Vec3::splat(1.0 / total));
        normalized
    }

    fn bounce(&self, index: usize, previous: &[Probe]) -> Probe {
        let mut result = self.direct[index].light;
        for bounce in &self.paths[self.path_offsets[index]..self.path_offsets[index] + self.path_lengths[index]] {
            if bounce.albedo == Vec3::ZERO {
                continue;
            }
            let cached = bounce.lookup.sample(previous);
            let incident = if cached.w > 0.01 {
                cached.truncate()
            } else {
                self.volume_weights
                    [bounce.volume_offset as usize..(bounce.volume_offset + bounce.volume_length) as usize]
                    .iter()
                    .fold(Vec3::ZERO, |sum, &(index, weight)| {
                        let field = previous[index as usize].light;
                        sum + (field.constant
                            + field.x * bounce.normal.x
                            + field.y * bounce.normal.y
                            + field.z * bounce.normal.z)
                            * weight
                    })
                    .max(Vec3::ZERO)
                    .lerp(cached.truncate(), (cached.w * 100.0).min(1.0))
            };
            // Finite iterations that always add the fixed source term.
            result.project_receiver(bounce.direction, incident * bounce.albedo, self.direct[index].normal);
        }
        Probe { light: result, ..self.direct[index] }
    }

    /// Solves daylight transport, then adds the fixture field.
    pub(crate) fn compose(&mut self, sun: Sun, objects: &[Object], probes: &mut [Probe]) {
        if let Some(daylight) = &self.daylight {
            daylight.compose(sun, &self.active, &self.direct, probes);
        } else {
            for slot in 0..self.active.len() {
                self.daylight_at(self.active[slot], sun);
            }
            self.solve(probes);
        }
        self.add_fixtures(objects, probes);
        self.applied = self
            .lights
            .iter()
            .map(|&index| {
                let light = objects[index as usize];
                light.emission_color * light.emission
            })
            .collect();
    }

    fn solve(&mut self, probes: &mut [Probe]) {
        for &index in &self.active {
            self.direct[index].light = self.filtered(index);
        }
        for pass in 0..BOUNCES - 1 {
            for slot in 0..self.active.len() {
                self.bounce_at(pass, self.active[slot]);
            }
        }
        for &index in &self.active {
            probes[index] = Probe { light: self.filtered(index), ..self.raw[index] };
        }
        for &index in &self.active {
            self.raw[index].light = probes[index].light;
        }
        for &index in &self.active {
            probes[index] = Probe { light: self.filtered(index), ..self.raw[index] };
        }
    }

    pub(crate) fn bind_groups(&mut self, entities: &[(&str, u32)]) {
        let mut groups = BTreeMap::<&str, Vec<u32>>::new();
        for &(name, object) in entities {
            if let Some(slot) = self.lights.iter().position(|&light| light == object) {
                groups.entry(name).or_default().push(slot as u32);
            }
        }
        self.groups = groups.into_values().collect();
    }

    pub(crate) fn prepare_controls(&mut self, sun: Sun, objects: &[Object], probes: &mut [Probe]) {
        if self.fixtures.is_empty() {
            return;
        }
        for group in self.groups.clone() {
            for &index in &self.active {
                let mut value = Irradiance::default();
                for &slot in &group {
                    value.add(self.fixtures[index * self.lights.len() + slot as usize], Vec3::ONE);
                }
                self.raw[index].light = value;
            }
            let mut response = self.direct.clone();
            self.solve(&mut response);
            self.responses.insert(group, self.active.iter().map(|&index| Packed::new(response[index].light)).collect());
        }
        // Fixture transport is a solved field from here, so the first-bounce bank is freed.
        self.fixtures = Box::default();
        self.compose(sun, objects, probes);
    }

    fn add_fixtures(&self, objects: &[Object], probes: &mut [Probe]) {
        for group in &self.groups {
            let fixture = objects[self.lights[group[0] as usize] as usize];
            let weight = fixture.emission_color * fixture.emission;
            let response = &self.responses[group];
            for (slot, &index) in self.active.iter().enumerate() {
                probes[index].light.add(response[slot].unpack(), weight);
            }
        }
    }

    /// Blends solved group responses.
    pub(crate) fn fixture_changes(&mut self, objects: &[Object], probes: &mut [Probe]) {
        let current: Vec<_> = self
            .lights
            .iter()
            .map(|&index| {
                let light = objects[index as usize];
                light.emission_color * light.emission
            })
            .collect();
        for group in &self.groups {
            let delta = current[group[0] as usize] - self.applied[group[0] as usize];
            if delta == Vec3::ZERO {
                continue;
            }
            let response = &self.responses[group];
            for (slot, &index) in self.active.iter().enumerate() {
                probes[index].light.add(response[slot].unpack(), delta);
            }
        }
        self.applied = current;
    }

    fn bounce_at(&mut self, pass: usize, index: usize) {
        let previous = if pass == 0 { &self.direct } else { &self.bounces[pass - 1] };
        let result = self.bounce(index, previous);
        if pass < BOUNCES - 2 {
            self.bounces[pass][index] = result;
        } else {
            self.raw[index] = result;
        }
    }

    pub(crate) const fn updating(&self) -> bool {
        self.remaining != 0
    }

    pub(crate) fn begin_update(&mut self) {
        self.cursor = 0;
        self.remaining = self.active.len() * (BOUNCES + 4);
    }

    /// Advances at most 64 probes and 16384 paths per frame.
    pub(crate) fn step(&mut self, sun: Sun, objects: &[Object], nodes: &[Node], probes: &mut [Probe]) {
        if self.remaining == 0 {
            return;
        }
        if self.daylight.is_some() {
            self.compose(sun, objects, probes);
            self.remaining = 0;
            self.cursor = 0;
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
                self.daylight_at(index, sun);
            } else if pass == 1 {
                self.direct[index].light = self.filtered(index);
            } else if pass == BOUNCES + 2 {
                self.raw[index].light = self.bounces[0][index].light;
            } else if pass == BOUNCES + 1 || pass == BOUNCES + 3 {
                self.bounces[0][index] = Probe { light: self.filtered(index), ..self.raw[index] };
            } else {
                self.bounce_at(pass - 2, index);
            }
        }
        self.remaining -= end - start;
        self.cursor += end - start;
        if self.remaining == 0 {
            // Publishes a complete solution, never a half-updated one.
            for &index in &self.active {
                probes[index] = self.bounces[0][index];
            }
            self.add_fixtures(objects, probes);
        }
    }

    pub(crate) fn relight(&mut self, sun: Sun, objects: &[Object], nodes: &[Node], probes: &mut [Probe]) {
        if self.daylight.is_none() {
            for &index in &self.active {
                self.sun[index] = self.sun_at(index, sun, objects, nodes);
            }
        }
        self.compose(sun, objects, probes);
        self.remaining = 0;
        self.cursor = 0;
    }
}

#[cfg(test)]
pub fn bake(
    objects: &mut [Object],
    nodes: &[Node],
    lights: &[u32],
    min: Vec3,
    max: Vec3,
    sun: Sun,
) -> (Grid, Vec<Probe>, Cache) {
    let (grid, mut probes, mut cache) = bake_cached(objects, nodes, lights, min, max, sun, None);
    cache.groups = (0..lights.len() as u32).map(|slot| vec![slot]).collect();
    cache.prepare_controls(sun, objects, &mut probes);
    (grid, probes, cache)
}

pub fn bake_cached(
    objects: &mut [Object],
    nodes: &[Node],
    lights: &[u32],
    min: Vec3,
    max: Vec3,
    sun: Sun,
    controls: Option<Controls>,
) -> (Grid, Vec<Probe>, Cache) {
    if let Some(controls) = controls {
        let mut cache = controls.transport;
        assert_eq!(cache.surface_slots.len(), objects.len(), "baked object layout differs");
        for ((object, &slot), layout) in objects.iter_mut().zip(&cache.surface_slots).zip(&cache.surface_layouts) {
            object.surface_cache = slot;
            object.surface_origin =
                vec3(f32::from_bits(layout[0]), f32::from_bits(layout[1]), f32::from_bits(layout[2]));
            object.surface_grid = uvec3(layout[3], layout[4], layout[5]);
        }
        cache.active = controls.active;
        cache.groups = controls.responses.keys().cloned().collect();
        cache.responses = controls.responses;
        let mut probes = cache.direct.to_vec();
        cache.relight(sun, objects, nodes, &mut probes);
        return (cache.grid, probes, cache);
    }
    let origin = vec3(min.x, min.y, 0.10);
    let size = ((vec3(max.x, max.y, 2.6) - origin) / SPACING).ceil().as_uvec3() + UVec3::ONE;
    let grid = Grid { origin, size };
    let lattice_count = (size.x * size.y * size.z) as usize;
    let surfaces = surface::sites(objects, nodes, lattice_count as u32);
    let count = lattice_count + surfaces.probes.len();
    let directions: [Vec3; RAYS] = from_fn(|sample| {
        let h = 1.0 - 2.0 * (sample as f32 + 0.5) / RAYS as f32;
        let (sin, cos) = (sample as f32 * 2.399_963).sin_cos();
        let r = (1.0 - h * h).sqrt();
        vec3(r * cos, r * sin, h)
    });
    let mut cache = Cache {
        daylight: None,
        surface_slots: objects.iter().map(|object| object.surface_cache).collect(),
        surface_layouts: objects
            .iter()
            .map(|object| {
                [
                    object.surface_origin.x.to_bits(),
                    object.surface_origin.y.to_bits(),
                    object.surface_origin.z.to_bits(),
                    object.surface_grid.x,
                    object.surface_grid.y,
                    object.surface_grid.z,
                ]
            })
            .collect(),
        paths: Vec::new(),
        volume_weights: Vec::new(),
        responses: BTreeMap::new(),
        applied: Vec::new(),
        groups: Vec::new(),
        path_offsets: vec![0; count].into_boxed_slice(),
        path_lengths: vec![0; count].into_boxed_slice(),
        sky: vec![Irradiance::default(); count].into_boxed_slice(),
        ground: vec![Irradiance::default(); count].into_boxed_slice(),
        sun: vec![Irradiance::default(); count].into_boxed_slice(),
        fixtures: vec![Irradiance::default(); count * lights.len()].into_boxed_slice(),
        direct: vec![Probe::default(); count].into_boxed_slice(),
        raw: vec![Probe::default(); count].into_boxed_slice(),
        bounces: from_fn(|_| vec![Probe::default(); count].into_boxed_slice()),
        active: Box::default(),
        grid,
        surface_neighbours: surfaces.neighbours.into_boxed_slice(),
        volume_neighbours: vec![[u32::MAX; 6]; lattice_count].into_boxed_slice(),
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
            if scene::render_furniture::<scene::Distance>(object.kind, local, object.size).0 < -0.0001 {
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
    cache.direct[lattice_count..].copy_from_slice(&surfaces.probes);
    // Every receiver's architectural bounds are set before any stencil is baked.
    for probe in &mut cache.direct[lattice_count..] {
        if probe.normal == Vec3::ZERO {
            continue;
        }
        let clearance =
            |axis: Vec3| (scene::architectural_cast(probe.position, axis, nodes, objects).0 - 0.002).max(0.0);
        probe.positive = vec3(clearance(Vec3::X), clearance(Vec3::Y), clearance(Vec3::Z));
        probe.negative = vec3(clearance(-Vec3::X), clearance(-Vec3::Y), clearance(-Vec3::Z));
    }
    for index in 0..count {
        let surface = index >= lattice_count;
        if surface && cache.direct[index].normal == Vec3::ZERO {
            continue;
        }
        let coordinate = uvec3(index as u32 % size.x, index as u32 / size.x % size.y, index as u32 / (size.x * size.y));
        let lattice = if surface { cache.direct[index].position } else { origin + coordinate.as_vec3() * SPACING };
        let mut point = lattice;
        if surface && inside(point) {
            // A texel inside a thin mirror or frame moves to free air on the wall's side; thick furniture stays disabled.
            let normal = cache.direct[index].normal;
            let clearance = scene::architectural_cast(point, normal, nodes, objects).0;
            for step in 1..=6 {
                let offset = step as f32 * 0.01;
                let candidate = lattice + normal * offset;
                if offset < clearance && !inside(candidate) {
                    point = candidate;
                    break;
                }
            }
            if inside(point) {
                cache.direct[index] =
                    Probe { positive: Vec3::splat(-FAR), negative: Vec3::splat(-FAR), ..Probe::default() };
                continue;
            }
        }
        // Sites inside furniture or above the roof bake at the nearest free indoor point.
        if !surface && !usable(point) {
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
        cache.path_offsets[index] = cache.paths.len();
        cache.direct[index].position = point;
        let clearance = |axis: Vec3| (scene::architectural_cast(point, axis, nodes, objects).0 - 0.002).max(0.0);
        let axes = |sign: f32| vec3(clearance(Vec3::X * sign), clearance(Vec3::Y * sign), clearance(Vec3::Z * sign));
        if !surface || point != lattice {
            (cache.direct[index].positive, cache.direct[index].negative) = (axes(1.0), axes(-1.0));
        }
        let receiver_normal = cache.direct[index].normal;
        // Per-site static rotations keep a shared ray pattern from banding narrow window light.
        let spin_axis = if surface { receiver_normal } else { Vec3::Z };
        let key = (point * 10000.0).round();
        let angle = scene::materials::hash(
            Vec2::new(key.x + key.z * 17.0, key.y) + Vec2::new(receiver_normal.x, receiver_normal.y) * 37.0,
        ) * (2.0 * PI);
        let (sin, cos) = angle.sin_cos();
        let mut cone_weights = [Vec3::ZERO; 2];
        for (sample, &direction) in directions.iter().enumerate() {
            let direction = direction * cos
                + spin_axis.cross(direction) * sin
                + spin_axis * (spin_axis.dot(direction) * (1.0 - cos));
            if surface && receiver_normal.dot(direction) <= 0.0 {
                continue;
            }
            let point = if surface {
                let footprint = surfaces.footprints[index - lattice_count];
                let jitter = (Vec2::new(sample as f32 * 0.754_877_7, sample as f32 * 0.569_840_3).fract() - 0.5) * 2.0;
                let candidate = point + footprint[0] * jitter.x + footprint[1] * jitter.y;
                // A free endpoint can lie across a thin partition, so origins never move through geometry.
                if inside(candidate) || !scene::segment_visible(point, candidate, nodes, objects) {
                    point
                } else {
                    candidate
                }
            } else {
                point
            };
            let (distance, object) = trace(point, direction);
            // Visibility reuses the transport rays; overlapping cones keep furniture silhouettes.
            let depth = distance.min(4.0);
            for (side, sign) in [1.0, -1.0].into_iter().enumerate() {
                let cosine = (direction * sign).max(Vec3::ZERO);
                let weights = cosine * cosine;
                let weights = weights * weights;
                let weights = weights * weights;
                let weights = weights * weights;
                cone_weights[side] += weights;
                if side == 0 {
                    cache.direct[index].mean_positive += weights * depth;
                    cache.direct[index].square_positive += weights * (depth * depth);
                } else {
                    cache.direct[index].mean_negative += weights * depth;
                    cache.direct[index].square_negative += weights * (depth * depth);
                }
            }
            if distance >= FAR {
                if direction.z >= 0.0 {
                    cache.sky[index].project_receiver(
                        direction,
                        Vec3::splat(0.25 + direction.z.sqrt() * 0.75),
                        receiver_normal,
                    );
                } else {
                    cache.ground[index].project_receiver(direction, Vec3::ONE, receiver_normal);
                }
                continue;
            }
            let object_index = object;
            let object = objects[object as usize];
            let hit = point + direction * distance;
            let local = scene::unrotate_z(hit - object.pos, object.rotation);
            let normal = scene::rotate_z(scene::object_normal(object, local), object.rotation);
            let albedo = average_albedo(object, local).min(Vec3::splat(0.85));
            let point = hit + normal * 0.002;
            cache.paths.push(Bounce { object: object_index, direction, point, normal, albedo, ..Bounce::default() });
        }
        cache.path_lengths[index] = cache.paths.len() - cache.path_offsets[index];
        let probe = &mut cache.direct[index];
        probe.mean_positive /= cone_weights[0].max(Vec3::splat(1.0e-8));
        probe.square_positive /= cone_weights[0].max(Vec3::splat(1.0e-8));
        probe.mean_negative /= cone_weights[1].max(Vec3::splat(1.0e-8));
        probe.square_negative /= cone_weights[1].max(Vec3::splat(1.0e-8));
    }
    // Interpolation links are fixed, so they are cached once.
    for path in &mut cache.paths {
        let item = objects[path.object as usize];
        if item.surface_cache != u32::MAX {
            let local = scene::unrotate_z(path.point - path.normal * 0.002 - item.pos, item.rotation);
            path.lookup = surface::lookup(&cache.direct, item, local, path.normal);
        }
    }
    cache.active = cache
        .direct
        .iter()
        .enumerate()
        .filter_map(|(index, probe)| (probe.positive.x >= 0.0).then_some(index))
        .collect();
    // Coplanar receivers can sit on opposite sides of a perpendicular wall.
    for (index, neighbours) in cache.surface_neighbours.iter_mut().enumerate() {
        let from = cache.direct[lattice_count + index].position;
        for neighbour in neighbours.iter_mut() {
            if *neighbour == u32::MAX {
                continue;
            }
            if !scene::segment_visible(from, cache.direct[*neighbour as usize].position, nodes, objects) {
                *neighbour = u32::MAX;
            }
        }
    }
    // Relocated volume sites can be diagonal neighbours, so denoising links are checked once too.
    for index in 0..lattice_count {
        let coordinate = uvec3(index as u32 % size.x, index as u32 / size.x % size.y, index as u32 / (size.x * size.y));
        if cache.direct[index].positive.x < 0.0 {
            continue;
        }
        for axis in 0..3 {
            for side in 0..2 {
                let mut neighbour = coordinate.as_ivec3();
                neighbour[axis] += if side == 0 { -1 } else { 1 };
                if neighbour.cmplt(IVec3::ZERO).any() || neighbour.cmpge(size.as_ivec3()).any() {
                    continue;
                }
                let n = neighbour.as_uvec3();
                let slot = (n.x + size.x * (n.y + size.y * n.z)) as usize;
                if cache.direct[slot].positive.x < 0.0 {
                    continue;
                }
                let from = cache.direct[index].position;
                if scene::segment_visible(from, cache.direct[slot].position, nodes, objects) {
                    cache.volume_neighbours[index][axis * 2 + side] = slot as u32;
                }
            }
        }
    }
    // Unit direct lighting is evaluated once per architectural receiver, not at every transport-ray hit.
    {
        let mut needed = vec![false; count];
        for path in &cache.paths {
            if path.lookup.confidence() >= 0.95 {
                for (index, weight) in path.lookup.taps() {
                    if weight > 0.0 {
                        needed[index as usize] = true;
                    }
                }
            }
        }
        let unit_incident = |point: Vec3, normal: Vec3, slot: usize| {
            let fixture = Object { emission: 1.0, emission_color: Vec3::ONE, ..objects[lights[slot] as usize] };
            let (direction, radiance, distance) = lighting::incident(fixture, point, Vec2::ZERO);
            let contribution = radiance * (normal.dot(direction).max(0.0) / PI);
            if contribution.max_element() < 0.0005 {
                return Vec3::ZERO;
            }
            contribution * scene::cast::<true, false>(point, direction, nodes, objects, 0.0, distance - 0.002).0
        };
        let mut direct_sources = vec![Vec3::ZERO; count * lights.len()];
        for (index, &needed) in needed.iter().enumerate() {
            if !needed {
                continue;
            }
            let probe = cache.direct[index];
            for slot in 0..lights.len() {
                direct_sources[index * lights.len() + slot] = unit_incident(probe.position, probe.normal, slot);
            }
        }
        for index in 0..count {
            for path in
                &mut cache.paths[cache.path_offsets[index]..cache.path_offsets[index] + cache.path_lengths[index]]
            {
                for slot in 0..lights.len() {
                    let incoming = if path.lookup.confidence() >= 0.95 {
                        path.lookup.taps().into_iter().fold(Vec3::ZERO, |sum, (index, weight)| {
                            sum + direct_sources[index as usize * lights.len() + slot] * weight
                        }) / path.lookup.confidence()
                    } else {
                        unit_incident(path.point, path.normal, slot)
                    };
                    cache.fixtures[index * lights.len() + slot].project_receiver(
                        path.direction,
                        path.albedo * incoming,
                        cache.direct[index].normal,
                    );
                }
            }
        }
        drop(direct_sources);
    }
    for path in &mut cache.paths {
        path.volume_offset = cache.volume_weights.len() as u32;
        if path.lookup.confidence() <= 0.01 {
            // Diagonal connectivity around thin partitions is validated once, while baking.
            let mut taps = grid.stencil(&cache.direct, path.point, path.normal);
            taps.retain(|&(index, _)| {
                scene::segment_visible(path.point, cache.direct[index as usize].position, nodes, objects)
            });
            let total: f32 = taps.iter().map(|&(_, weight)| weight).sum();
            for (_, weight) in &mut taps {
                *weight /= total.max(1.0e-20);
            }
            cache.volume_weights.extend(taps);
        }
        path.volume_length = cache.volume_weights.len() as u32 - path.volume_offset;
    }
    cache.raw = cache.direct.clone();
    cache.bounces = from_fn(|_| cache.direct.clone());
    let mut probes = cache.direct.to_vec();
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
mod transport_tests;
