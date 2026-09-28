//! One stackless tree for the fixed overhead camera; leaves index render objects.
use crate::home::{Object, Room};
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
    pub radius: f32,
    pub grid: super::scene::irradiance::Grid,
    pub probes: Box<[super::scene::irradiance::Probe]>,
}

impl Scene {
    pub fn new(rooms: &[Room]) -> Self {
        let objects = super::plants::compile(super::layout::compile(rooms));
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
        for node in &mut nodes {
            if node.object != BRANCH {
                ordered.push(objects[node.object as usize]);
                node.object = (ordered.len() - 1) as u32;
            }
        }
        let objects = ordered;
        let radius = nodes.first().map_or(0.0, |node| node.min.abs().max(node.max.abs()).length());
        let min = nodes.first().map_or(Vec3::ZERO, |n| n.min);
        let max = nodes.first().map_or(Vec3::ONE, |n| n.max);
        let (grid, probes) = super::scene::irradiance::bake(&objects, min, max);
        Self { objects: objects.into_boxed_slice(), nodes: nodes.into_boxed_slice(), radius, grid, probes }
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
