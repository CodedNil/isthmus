//! One stackless tree for the fixed overhead camera; leaves index render objects.
use crate::home::{Kind, Object, Room};
use isthmus::{ShaderData, glam::Vec3};

#[derive(Clone, Copy, ShaderData)]
pub struct Node {
    pub min: Vec3,
    pub max: Vec3,
    pub skip: u32,
    pub object: u32,
}
pub const BRANCH: u32 = u32::MAX;

pub struct Scene {
    pub objects: Box<[Object]>,
    pub nodes: Box<[Node]>,
    pub lights: Box<[u32]>,
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
    pub fn new(rooms: &[Room]) -> Self {
        Self::new_at(rooms, super::sun::utc_now(), super::SunSettings::default())
    }

    /// Deterministic clock and geographic settings for previews and tests.
    pub fn new_at(rooms: &[Room], unix_seconds: f64, sun_settings: super::SunSettings) -> Self {
        let sun = super::Sun::at_utc(unix_seconds, sun_settings);
        let objects = super::plants::compile(super::layout::compile(rooms));
        let mut names = rooms.iter().flat_map(|room| room.lights.iter().map(|light| light.entity_id));
        // Plant expansion preserves fixture order; retain bindings through BVH reordering.
        let entities: Vec<_> = objects
            .iter()
            .map(|o| if matches!(o.kind, Kind::Downlight | Kind::Lamp) { names.next() } else { None })
            .collect();
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
        // Keep adjacent BVH leaves adjacent in the storage buffer as well.
        let mut ordered = Vec::with_capacity(objects.len());
        let mut light_entities = Vec::new();
        for node in &mut nodes {
            if node.object != BRANCH {
                if let Some(entity) = entities[node.object as usize] {
                    light_entities.push((entity, ordered.len() as u32));
                }
                ordered.push(objects[node.object as usize]);
                node.object = (ordered.len() - 1) as u32;
            }
        }
        let objects = ordered;
        let radius = nodes.first().map_or(0.0, |node| node.min.abs().max(node.max.abs()).length());
        let min = nodes.first().map_or(Vec3::ZERO, |n| n.min);
        let max = nodes.first().map_or(Vec3::ONE, |n| n.max);
        let lights: Box<[u32]> = objects
            .iter()
            .enumerate()
            .filter(|(_, o)| matches!(o.kind, Kind::Downlight | Kind::Lamp))
            .map(|(i, _)| i as u32)
            .collect();
        let (grid, probes, indirect) = super::scene::irradiance::bake(&objects, &nodes, &lights, min, max, sun);
        Self {
            sun,
            sun_settings,
            indirect,
            clock: unix_seconds,
            last_bake: unix_seconds,
            objects: objects.into_boxed_slice(),
            nodes: nodes.into_boxed_slice(),
            lights,
            light_entities: light_entities.into_boxed_slice(),
            radius,
            grid,
            probes,
        }
    }

    /// Set an entity's output in [0, 1], updating every fixture in its group.
    /// Cached transport also updates bounced light without rebuilding geometry.
    pub fn set_light_state(&mut self, entity_id: &str, brightness: f32) -> bool {
        let brightness = if brightness.is_finite() { brightness.clamp(0.0, 1.0) } else { 0.0 };
        let mut found = false;
        let mut changed = false;
        for &(entity, index) in &self.light_entities {
            if entity == entity_id {
                changed |= self.objects[index as usize].emission.to_bits() != brightness.to_bits();
                self.objects[index as usize].emission = brightness;
                found = true;
            }
        }
        if changed {
            self.indirect.compose(self.sun, &self.objects, &self.lights, self.grid, &mut self.probes);
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
            // A restored tab can cross sunrise or sunset. Update source colors
            // immediately; visibility then converges within the normal budget.
            self.indirect.compose(self.sun, &self.objects, &self.lights, self.grid, &mut self.probes);
        }
        if jumped || (!self.indirect.updating() && (unix_seconds - self.last_bake).abs() >= 60.0) {
            self.indirect.begin_update();
            self.last_bake = unix_seconds;
        }
        self.indirect.step(self.sun, &self.objects, &self.nodes, &self.lights, self.grid, &mut self.probes);
    }

    /// Synchronous relight for explicit time jumps, captures and configuration.
    pub fn set_time(&mut self, unix_seconds: f64) {
        if !unix_seconds.is_finite() {
            return;
        }
        self.clock = unix_seconds;
        self.last_bake = unix_seconds;
        self.sun = super::Sun::at_utc(unix_seconds, self.sun_settings);
        self.indirect.relight(self.sun, &self.objects, &self.nodes, &self.lights, self.grid, &mut self.probes);
    }

    pub fn set_sun_settings(&mut self, settings: super::SunSettings) {
        self.sun_settings = settings;
        self.set_time(self.clock);
    }
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
