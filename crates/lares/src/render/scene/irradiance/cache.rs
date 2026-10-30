//! Static geometry transport, solved fixture responses and budgeted daylight.
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
pub(super) use daylight::Packed;
use isthmus::{
    ShaderData,
    glam::{IVec3, UVec3, Vec2, Vec3, uvec3, vec3},
};
use std::collections::BTreeMap;

const RAYS: usize = 256;
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

    fn project_receiver(&mut self, direction: Vec3, radiance: Vec3, normal: Vec3) {
        if normal.length_squared() > 0.5 {
            let cosine = normal.dot(direction).max(0.0);
            self.constant += radiance * (4.0 * cosine / RAYS as f32);
            if cosine > 0.0 {
                // First derivative of the cosine integral, in world space.
                // The existing directional fields let material normals respond
                // to bounced light without adding receiver sites or ray casts.
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
    lookup: super::surface::Lookup,
    point: Vec3,
    normal: Vec3,
    albedo: Vec3,
    volume_offset: u32,
    volume_length: u32,
}

mod daylight;
mod transport;

pub struct Cache {
    pub(super) daylight: Option<daylight::Daylight>,
    surface_slots: Box<[u32]>,
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
    /// Fixed direct sources and their first bounce, shared by every spatial iteration.
    direct: Box<[Probe]>,
    raw: Box<[Probe]>,
    bounces: [Box<[Probe]>; BOUNCES - 2],
    pub(super) active: Box<[usize]>,
    grid: Grid,
    surface_neighbours: Box<[[u32; 9]]>,
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
        // Analytic sunlit exterior ground seen through real apertures. No
        // outdoor probes: a neutral 20%-reflecting Lambertian ground assumption.
        let ground = (sun.radiance * sun.direction.z.max(0.0) + sun.sky * (0.85 * PI)) * (0.20 / PI);
        result.add(self.ground[index], ground);
        result.add(self.sun[index], sun.radiance);
        self.raw[index].light = result;
    }

    /// Geometry-aware spatial denoising, before interpolation and further
    /// bounces. Fixed weights preserve source linearity and constant fields.
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
            // Finite spatial iterations, always adding the fixed source term.
            // No ad-hoc energy loss and no feedback from previous frames.
            result.project_receiver(bounce.direction, incident * bounce.albedo, self.direct[index].normal);
        }
        Probe { light: result, ..self.direct[index] }
    }

    /// Solve cached daylight transport, then add the solved fixture field.
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
        // Runtime fixture transport is now a solved field. It never enters
        // the daylight solver again, and the large first-bounce bank is freed.
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

    /// Blend solved group responses without entering the daylight solver.
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

    /// At most 64 probes / 16384 cached paths per frame, independent of resolution.
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
            // Publish a complete lighting solution, never a moving stripe of
            // partially updated wall texels or a half-filtered source state.
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
) -> (Grid, Box<[Probe]>, Cache) {
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
    controls: Option<super::Controls>,
) -> (Grid, Box<[Probe]>, Cache) {
    if let Some(controls) = controls {
        let mut cache = controls.transport;
        assert_eq!(cache.surface_slots.len(), objects.len(), "baked object layout differs");
        for (object, &slot) in objects.iter_mut().zip(&cache.surface_slots) {
            object.surface_cache = slot;
        }
        cache.active = controls.active;
        cache.groups = controls.responses.keys().cloned().collect();
        cache.responses = controls.responses;
        let mut probes = cache.direct.clone();
        cache.relight(sun, objects, nodes, &mut probes);
        return (cache.grid, probes, cache);
    }
    let origin = vec3(min.x, min.y, 0.10);
    let size = ((vec3(max.x, max.y, 2.6) - origin) / SPACING).ceil().as_uvec3() + UVec3::ONE;
    let grid = Grid { origin, size };
    let lattice_count = (size.x * size.y * size.z) as usize;
    let surfaces = super::surface::sites(objects, nodes, lattice_count as u32);
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
    // Populate every projected receiver's architectural bounds before baking
    // any transport stencil, including stencils that reference later sites.
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
            // A wall chart can land inside a thin mirror/frame mounted a few
            // millimetres in front of it. Discarding that texel makes a dark
            // 20 cm hole which the interpolation spreads beyond the ornament.
            // Move its transport origin to free air on the same side of the
            // wall; thick furniture remains disabled, as before.
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
        // Keep interpolation sites near wall/ceiling intersections populated.
        // Bake at the nearest free indoor point instead of discarding sites
        // inside furniture or sampling exterior sky above the roof.
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
        // Independent rotations prevent a shared finite ray pattern from
        // turning narrow window visibility into coherent square bands. The
        // rotation is static: relighting stays deterministic and noise-free.
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
                // A free endpoint can still lie across a thin partition. Never
                // teleport transport origins through geometry at texel edges.
                let offset = candidate - point;
                let length = offset.length();
                if inside(candidate)
                    || (length > 0.0001
                        && scene::cast::<false, false>(point, offset / length, nodes, objects, 0.0, length).0 < length)
                {
                    point
                } else {
                    candidate
                }
            } else {
                point
            };
            let (distance, object) = trace(point, direction);
            // Reuse the transport rays for visibility; no extra geometry work.
            // Narrow, overlapping cones retain furniture silhouettes while
            // their second moments soften uncertain, partially blocked regions.
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
    // Geometry interpolation links are fixed. Cache them once rather than
    // reconstructing four surface neighbours for every ray of every bounce.
    for path in &mut cache.paths {
        let item = objects[path.object as usize];
        if item.surface_cache != u32::MAX {
            let local = scene::unrotate_z(path.point - path.normal * 0.002 - item.pos, item.rotation);
            path.lookup = super::surface::lookup(&cache.direct, item, local, path.normal);
        }
    }
    cache.active = cache
        .direct
        .iter()
        .enumerate()
        .filter_map(|(index, probe)| (probe.positive.x >= 0.0).then_some(index))
        .collect();
    // Coplanar receivers may be on opposite sides of a perpendicular wall.
    // Plane/normal agreement alone cannot certify a connected filter footprint.
    for (index, neighbours) in cache.surface_neighbours.iter_mut().enumerate() {
        let from = cache.direct[lattice_count + index].position;
        for neighbour in neighbours.iter_mut() {
            if *neighbour == u32::MAX {
                continue;
            }
            let offset = cache.direct[*neighbour as usize].position - from;
            let length = offset.length();
            if length > 0.0001
                && scene::cast::<false, false>(from, offset / length, nodes, objects, 0.0, length).0 < length
            {
                *neighbour = u32::MAX;
            }
        }
    }
    // Evaluate unit direct lighting once per reliable architectural receiver,
    // rather than ray-tracing all fixtures again at every transport-ray hit.
    // The primary shadow atlas remains full resolution; only the already
    // 20 cm projected diffuse transport uses this shared spatial field.
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
            let (direction, radiance, distance) = fixtures::incident(fixture, point, Vec2::ZERO);
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
            cache.volume_weights.extend(grid.stencil(&cache.direct, path.point, path.normal));
        }
        path.volume_length = cache.volume_weights.len() as u32 - path.volume_offset;
    }
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
