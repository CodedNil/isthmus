//! Stackless SDF acceleration shared by view, visibility and lighting rays.
use crate::{
    home::{Kind, Object, Room},
    irradiance,
    render::{
        geometry::{layout, light_culling, plants},
        light::sun::{Sun, SunSettings},
    },
    scene,
};
use isthmus::{ShaderData, glam::Vec3};
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::io;

#[derive(Clone, Copy, ShaderData)]
pub struct Node {
    pub min: Vec3,
    pub max: Vec3,
    pub skip: u32,
    pub object: u32,
}

pub const BRANCH: u32 = u32::MAX;

pub struct Scene {
    signature: u64,
    pub objects: Vec<Object>,
    pub nodes: Vec<Node>,
    pub lights: Vec<u32>,
    pub light_masks: Vec<u32>,
    pub contacts: Vec<u32>,
    /// The object reflected rays follow, or 0 when the home has no mirror.
    pub mirror: u32,
    pub lighting_revision: u64,
    light_entities: Vec<(&'static str, u32)>,
    pub radius: f32,
    pub sun: Sun,
    pub sun_settings: SunSettings,
    indirect: irradiance::Cache,
    clock: f64,
    last_bake: f64,
    pub grid: irradiance::Grid,
    pub probes: Vec<irradiance::Probe>,
}

impl Scene {
    /// Cheap geometry fingerprint that needs no tracing.
    pub fn geometry_signature(rooms: &[Room]) -> u64 {
        source_signature(rooms)
    }

    pub fn encode_light_controls(&mut self) -> Result<Vec<u8>, io::Error> {
        self.indirect.encode_controls(self.signature, self.sun, &self.objects, &self.nodes)
    }

    pub const fn signature(&self) -> u64 {
        self.signature
    }

    pub fn transport_statistics(&self) -> (usize, usize, usize) {
        self.indirect.statistics()
    }

    /// Exposure and white balance for this view's illumination.
    pub fn camera_settings(&self, eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) -> (f32, Vec3) {
        scene::camera_settings(self, eye, forward, right, up)
    }

    /// Builds the scene for an explicit clock, so previews and tests are deterministic.
    pub fn new_at(rooms: &[Room], unix_seconds: f64, sun_settings: SunSettings) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let controls = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/baked/lighting.bin")).ok();
        #[cfg(target_arch = "wasm32")]
        let controls: Option<Vec<u8>> = None;
        let controls = controls.and_then(|bytes| irradiance::Controls::decode(&bytes, source_signature(rooms)).ok());
        Self::build_scene(rooms, unix_seconds, sun_settings, controls)
    }

    /// Builds the scene from baked fixture responses, rejecting stale data.
    pub fn from_baked_at(
        rooms: &[Room],
        unix_seconds: f64,
        sun_settings: SunSettings,
        bytes: &[u8],
    ) -> Result<Self, &'static str> {
        let controls = irradiance::Controls::decode(bytes, source_signature(rooms))?;
        Ok(Self::build_scene(rooms, unix_seconds, sun_settings, Some(controls)))
    }

    fn build_scene(
        rooms: &[Room],
        unix_seconds: f64,
        sun_settings: SunSettings,
        controls: Option<irradiance::Controls>,
    ) -> Self {
        let sun = Sun::at_utc(unix_seconds, sun_settings);
        let (mut objects, nodes, light_entities) = geometry(rooms);
        let radius = nodes.first().map_or(0.0, |node| node.min.abs().max(node.max.abs()).length());
        let min = nodes.first().map_or(Vec3::ZERO, |n| n.min);
        let max = nodes.first().map_or(Vec3::ONE, |n| n.max);
        let lights: Vec<u32> = (0..objects.len() as u32).filter(|&i| objects[i as usize].is_light()).collect();
        let light_masks = light_culling::masks(&objects, &nodes, &lights);
        let contacts = scene::contacts::build(&objects, &nodes, min, max);
        let signature = source_signature(rooms);
        let mirror = objects.iter().position(|item| item.kind == Kind::Mirror).unwrap_or(0) as u32;
        let (grid, mut probes, mut indirect) =
            irradiance::bake_cached(&mut objects, &nodes, &lights, min, max, sun, controls);
        indirect.bind_groups(&light_entities);
        indirect.prepare_controls(sun, &objects, &mut probes);
        Self {
            signature,
            sun,
            sun_settings,
            indirect,
            clock: unix_seconds,
            last_bake: unix_seconds,
            objects,
            nodes,
            lights,
            light_masks,
            contacts,
            mirror,
            lighting_revision: 0,
            light_entities,
            radius,
            grid,
            probes,
        }
    }

    /// Sets an entity's output in [0, 1] on every fixture in its group, including bounced light.
    pub fn set_light_state(&mut self, entity_id: &str, brightness: f32) -> bool {
        self.set_light_states(&[(entity_id, brightness)])
    }

    /// Applies several entity changes with one blend of the solved field.
    pub fn set_light_states(&mut self, states: &[(&str, f32)]) -> bool {
        let mut found = false;
        let mut changed = false;
        for &(entity_id, brightness) in states {
            let brightness = if brightness.is_finite() { brightness.clamp(0.0, 1.0) } else { 0.0 };
            for &(entity, index) in &self.light_entities {
                if entity == entity_id {
                    changed |= self.objects[index as usize].emission.to_bits() != brightness.to_bits();
                    self.objects[index as usize].emission = brightness;
                    found = true;
                }
            }
        }
        if changed {
            self.lighting_revision = self.lighting_revision.wrapping_add(1);
            self.indirect.fixture_changes(&self.objects, &mut self.probes);
        }
        found
    }

    /// Sets an entity's linear RGB colour, including its cached bounces.
    pub fn set_light_color(&mut self, entity_id: &str, color: Vec3) -> bool {
        let color = if color.is_finite() { color.max(Vec3::ZERO) } else { Vec3::ZERO };
        let mut found = false;
        let mut changed = false;
        for &(entity, index) in &self.light_entities {
            if entity == entity_id {
                let light = &mut self.objects[index as usize];
                changed |= light.emission_color != color;
                light.emission_color = color;
                found = true;
            }
        }
        if changed {
            self.lighting_revision = self.lighting_revision.wrapping_add(1);
            self.indirect.fixture_changes(&self.objects, &mut self.probes);
        }
        found
    }

    /// Direct sun updates every second and bounced sun every minute, in small batches.
    pub fn update_time(&mut self, unix_seconds: f64) {
        if !unix_seconds.is_finite() {
            return;
        }
        if self.clock as i64 != unix_seconds as i64 {
            self.sun = Sun::at_utc(unix_seconds, self.sun_settings);
        }
        let jumped = (unix_seconds - self.clock).abs() > 120.0;
        self.clock = unix_seconds;
        if jumped {
            self.lighting_revision = self.lighting_revision.wrapping_add(1);
            // A restored tab can cross sunrise or sunset, so source colours update at once.
            self.indirect.compose(self.sun, &self.objects, &mut self.probes);
        }
        if jumped || (!self.indirect.updating() && (unix_seconds - self.last_bake).abs() >= 60.0) {
            self.indirect.begin_update();
            self.last_bake = unix_seconds;
        }
        self.indirect.step(self.sun, &self.objects, &self.nodes, &mut self.probes);
    }

    /// Relights immediately, for time jumps and captures.
    pub fn set_time(&mut self, unix_seconds: f64) {
        if !unix_seconds.is_finite() {
            return;
        }
        self.clock = unix_seconds;
        self.lighting_revision = self.lighting_revision.wrapping_add(1);
        self.last_bake = unix_seconds;
        self.sun = Sun::at_utc(unix_seconds, self.sun_settings);
        self.indirect.relight(self.sun, &self.objects, &self.nodes, &mut self.probes);
    }
}

fn source_signature(rooms: &[Room]) -> u64 {
    let mut words = Vec::new();
    // Quantized so native and wasm agree despite sub-micron trigonometric differences.
    let scalar = |value: f32| (f64::from(value) * 100_000.0).round() as i32 as u32;
    let object = |item: Object, words: &mut Vec<u32>| {
        words.extend([item.kind as u32, item.material as u32]);
        words.extend(item.pos.to_array().into_iter().map(scalar));
        words.extend(item.size.to_array().into_iter().map(scalar));
        words.extend(item.rotation.to_array().into_iter().map(scalar));
        words.extend(item.exterior.to_array().into_iter().map(scalar));
    };
    for room in rooms {
        words.extend(room.position.to_array().into_iter().map(scalar));
        words.extend([room.walls.bits(), room.floor as u32, room.outline.len() as u32]);
        for point in &room.outline {
            words.extend(point.to_array().into_iter().map(scalar));
        }
        words.push(room.openings.len() as u32);
        for opening in &room.openings {
            words.extend(opening.pos.to_array().into_iter().map(scalar));
            words.extend(opening.size.to_array().into_iter().map(scalar));
        }
        words.push(room.furniture.len() as u32);
        for &item in &room.furniture {
            object(item, &mut words);
        }
        words.push(room.lights.len() as u32);
        for light in &room.lights {
            object(light.fixture, &mut words);
            words.push(light.entity_id.len() as u32);
            words.extend(light.entity_id.bytes().map(u32::from));
        }
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in env!("LARES_SCENE_VERSION").bytes().chain(words.into_iter().flat_map(u32::to_le_bytes)) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn geometry(rooms: &[Room]) -> (Vec<Object>, Vec<Node>, Vec<(&'static str, u32)>) {
    let objects = plants::compile(layout::compile(rooms));
    let mut names = rooms.iter().flat_map(|room| room.lights.iter().map(|light| light.entity_id));
    // Fixture bindings follow their objects through the BVH reordering.
    let entities: Vec<_> = objects.iter().map(|o| if o.is_light() { names.next() } else { None }).collect();
    let mut leaves: Vec<_> = objects
        .iter()
        .copied()
        .enumerate()
        .map(|(index, object)| {
            let half = object.extent();
            let (sin, cos) = (object.rotation.x.abs(), object.rotation.y.abs());
            let extent = Vec3::new(cos * half.x + sin * half.y, sin * half.x + cos * half.y, half.z);
            Node { min: object.pos - extent, max: object.pos + extent, skip: 0, object: index as u32 }
        })
        .collect();
    let mut nodes = Vec::with_capacity(leaves.len() * 2);
    if !leaves.is_empty() {
        build(&mut leaves, &mut nodes);
    }
    // Object IDs keep authored order so baked responses and light slots match wasm.
    let light_entities =
        entities.iter().enumerate().filter_map(|(index, &name)| name.map(|name| (name, index as u32))).collect();
    (objects, nodes, light_entities)
}

fn build(leaves: &mut [Node], nodes: &mut Vec<Node>) {
    let slot = nodes.len();
    if leaves.len() == 1 {
        nodes.push(Node { skip: (slot + 1) as u32, ..leaves[0] });
        return;
    }
    let min = leaves.iter().fold(Vec3::splat(f32::MAX), |v, n| v.min(n.min));
    let max = leaves.iter().fold(Vec3::splat(f32::MIN), |v, n| v.max(n.max));
    nodes.push(Node { min, max, skip: 0, object: BRANCH });
    // Surface-area heuristic, so large floors are not grouped with tiny foliage.
    let area = |min: Vec3, max: Vec3| {
        let extent = max - min;
        extent.x * extent.y + extent.y * extent.z + extent.z * extent.x
    };
    let sort = |items: &mut [Node], axis| {
        items.sort_unstable_by(|a, b| (a.min[axis] + a.max[axis]).total_cmp(&(b.min[axis] + b.max[axis])));
    };
    let mut best = (f32::MAX, 0, leaves.len() / 2);
    for axis in 0..3 {
        sort(leaves, axis);
        let mut bounds = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let prefix: Vec<_> = leaves
            .iter()
            .map(|node| {
                bounds = (bounds.0.min(node.min), bounds.1.max(node.max));
                area(bounds.0, bounds.1)
            })
            .collect();
        bounds = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for middle in (1..leaves.len()).rev() {
            let node = leaves[middle];
            bounds = (bounds.0.min(node.min), bounds.1.max(node.max));
            let cost = prefix[middle - 1] * middle as f32 + area(bounds.0, bounds.1) * (leaves.len() - middle) as f32;
            if cost < best.0 {
                best = (cost, axis, middle);
            }
        }
    }
    sort(leaves, best.1);
    let (left, right) = leaves.split_at_mut(best.2);
    // Tall geometry first gives overhead rays an early hit.
    let height = |items: &[Node]| items.iter().fold(f32::MIN, |height, node| height.max(node.max.z));
    let (near, far) = if height(left) >= height(right) { (left, right) } else { (right, left) };
    build(near, nodes);
    build(far, nodes);
    nodes[slot].skip = nodes.len() as u32;
}
