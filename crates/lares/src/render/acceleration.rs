//! Stackless SDF acceleration shared by view, visibility and lighting rays.
use crate::home::{Object, Room};
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
    pub objects: Box<[Object]>,
    pub nodes: Box<[Node]>,
    pub lights: Box<[u32]>,
    pub light_masks: Box<[u32]>,
    pub contacts: Box<[u32]>,
    pub lighting_revision: u64,
    light_entities: Box<[(&'static str, u32)]>,
    pub radius: f32,
    pub sun: super::Sun,
    pub sun_settings: super::SunSettings,
    indirect: super::scene::irradiance::Cache,
    clock: f64,
    last_bake: f64,
    pub grid: super::scene::irradiance::Grid,
    pub probes: Box<[super::scene::irradiance::Probe]>,
}

impl Scene {
    /// Cheap geometry fingerprint: no GI rays, shadow bake or source solve.
    pub fn geometry_signature(rooms: &[Room]) -> u64 {
        source_signature(rooms)
    }

    pub fn encode_light_controls(&self) -> Result<Vec<u8>, io::Error> {
        self.indirect.encode_controls(self.signature)
    }

    pub const fn signature(&self) -> u64 {
        self.signature
    }

    pub const fn transport_statistics(&self) -> (usize, usize, usize) {
        self.indirect.statistics()
    }

    pub fn camera_exposure(&self, eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) -> f32 {
        self.camera_settings(eye, forward, right, up).0
    }

    /// Exposure and bounded white balance for this view's illumination.
    pub fn camera_settings(&self, eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) -> (f32, Vec3) {
        super::scene::camera_settings(self, eye, forward, right, up)
    }

    pub fn new(rooms: &[Room]) -> Self {
        Self::new_at(rooms, super::sun::utc_now(), super::SunSettings::default())
    }

    /// Deterministic clock and geographic settings for previews and tests.
    pub fn new_at(rooms: &[Room], unix_seconds: f64, sun_settings: super::SunSettings) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let controls = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/baked/lighting.bin")).ok();
        #[cfg(target_arch = "wasm32")]
        let controls: Option<Vec<u8>> = None;
        let controls =
            controls.and_then(|bytes| super::scene::irradiance::Controls::decode(&bytes, source_signature(rooms)).ok());
        Self::build_scene(rooms, unix_seconds, sun_settings, controls)
    }

    /// Load solved fixture responses, rejecting missing/stale data before tracing.
    pub fn from_baked_at(
        rooms: &[Room],
        unix_seconds: f64,
        sun_settings: super::SunSettings,
        bytes: &[u8],
    ) -> Result<Self, &'static str> {
        let controls = super::scene::irradiance::Controls::decode(bytes, source_signature(rooms))?;
        Ok(Self::build_scene(rooms, unix_seconds, sun_settings, Some(controls)))
    }

    fn build_scene(
        rooms: &[Room],
        unix_seconds: f64,
        sun_settings: super::SunSettings,
        controls: Option<super::scene::irradiance::Controls>,
    ) -> Self {
        let sun = super::Sun::at_utc(unix_seconds, sun_settings);
        let (mut objects, nodes, light_entities) = geometry(rooms);
        let radius = nodes.first().map_or(0.0, |node| node.min.abs().max(node.max.abs()).length());
        let min = nodes.first().map_or(Vec3::ZERO, |n| n.min);
        let max = nodes.first().map_or(Vec3::ONE, |n| n.max);
        let lights: Box<[u32]> = (0..objects.len() as u32).filter(|&i| objects[i as usize].is_light()).collect();
        let light_masks = super::light_culling::masks(&objects, &nodes, &lights);
        let contacts = super::scene::contacts::build(&objects, &nodes, min, max);
        let signature = source_signature(rooms);
        let (grid, mut probes, mut indirect) =
            super::scene::irradiance::bake_cached(&mut objects, &nodes, &lights, min, max, sun, controls);
        indirect.bind_groups(&light_entities);
        indirect.prepare_controls(sun, &objects, &mut probes);
        Self {
            signature,
            sun,
            sun_settings,
            indirect,
            clock: unix_seconds,
            last_bake: unix_seconds,
            objects: objects.into_boxed_slice(),
            nodes: nodes.into_boxed_slice(),
            lights,
            light_masks,
            contacts,
            lighting_revision: 0,
            light_entities: light_entities.into_boxed_slice(),
            radius,
            grid,
            probes,
        }
    }

    /// Set an entity's output in [0, 1], updating every fixture in its group.
    /// Cached transport also updates bounced light without rebuilding geometry.
    pub fn set_light_state(&mut self, entity_id: &str, brightness: f32) -> bool {
        self.set_light_states(&[(entity_id, brightness)])
    }

    /// Apply grouped source changes with one solved-field blend.
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

    /// Changes linear RGB source colour and all cached diffuse bounces.
    /// Visibility is geometric and does not need to be traced or baked again.
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

    /// Direct sun updates once a second. Bounced sun refreshes once a minute
    /// in small batches, so the browser never rebakes a whole room on a frame.
    pub fn update_time(&mut self, unix_seconds: f64) {
        if !unix_seconds.is_finite() {
            return;
        }
        if self.clock as i64 != unix_seconds as i64 {
            self.sun = super::Sun::at_utc(unix_seconds, self.sun_settings);
        }
        let jumped = (unix_seconds - self.clock).abs() > 120.0;
        self.clock = unix_seconds;
        if jumped {
            self.lighting_revision = self.lighting_revision.wrapping_add(1);
            // A restored tab can cross sunrise or sunset. Update source colors
            // immediately; visibility then converges within the normal budget.
            self.indirect.compose(self.sun, &self.objects, &mut self.probes);
        }
        if jumped || (!self.indirect.updating() && (unix_seconds - self.last_bake).abs() >= 60.0) {
            self.indirect.begin_update();
            self.last_bake = unix_seconds;
        }
        self.indirect.step(self.sun, &self.objects, &self.nodes, &mut self.probes);
    }

    /// Synchronous relight for explicit time jumps, captures and configuration.
    pub fn set_time(&mut self, unix_seconds: f64) {
        if !unix_seconds.is_finite() {
            return;
        }
        self.clock = unix_seconds;
        self.lighting_revision = self.lighting_revision.wrapping_add(1);
        self.last_bake = unix_seconds;
        self.sun = super::Sun::at_utc(unix_seconds, self.sun_settings);
        self.indirect.relight(self.sun, &self.objects, &self.nodes, &mut self.probes);
    }

    pub fn set_sun_settings(&mut self, settings: super::SunSettings) {
        self.sun_settings = settings;
        self.set_time(self.clock);
    }
}
fn source_signature(rooms: &[Room]) -> u64 {
    let mut words = Vec::new();
    // Authored coordinates are stable across native/WASM. Quantization only
    // removes sub-micron trigonometric differences in quarter-turn rotations;
    // SDF source changes are independently covered by the source version.
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
    let objects = super::plants::compile(super::layout::compile(rooms));
    let mut names = rooms.iter().flat_map(|room| room.lights.iter().map(|light| light.entity_id));
    // Plant expansion preserves fixture order; retain bindings through BVH reordering.
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
    // Object IDs belong to authored expansion order, not floating-point SAH
    // tie breaks. Offline response fields and light slots must match WASM.
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
    // Exact surface-area heuristic. Building is paid once on the CPU; the
    // resulting tree avoids grouping large floors with tiny foliage by count.
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
    // Tall geometry first provides an early nearest hit for the overhead rays.
    let height = |items: &[Node]| items.iter().fold(f32::MIN, |height, node| height.max(node.max.z));
    let (near, far) = if height(left) >= height(right) { (left, right) } else { (right, left) };
    build(near, nodes);
    build(far, nodes);
    nodes[slot].skip = nodes.len() as u32;
}
