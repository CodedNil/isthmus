//! Conservative visibility of an entire object's bounds to each finite emitter.
//! A solid architectural box can reject a light only when its middle plane
//! covers every segment from the receiver box to all four emitter samples.
use super::acceleration::{BRANCH, Node};
use crate::home::{Kind, Object};
use isthmus::glam::{Vec2, Vec3, vec3};

pub(super) fn masks(objects: &[Object], nodes: &[Node], lights: &[u32]) -> Box<[u32]> {
    let walls: Vec<_> = nodes
        .iter()
        .filter(|node| {
            node.object != BRANCH && {
                let item = objects[node.object as usize];
                matches!(item.kind, Kind::Box | Kind::Ceiling) && item.rotation == Vec2::Y
            }
        })
        .collect();
    let mut masks = vec![u32::MAX; objects.len()];
    for node in nodes.iter().filter(|node| node.object != BRANCH) {
        // Shadow rays start outside the surface normal by at most 2 mm.
        let min = node.min - Vec3::splat(0.003);
        let max = node.max + Vec3::splat(0.003);
        for (slot, &index) in lights.iter().take(32).enumerate() {
            let light = objects[index as usize];
            let downlight = light.kind == Kind::Downlight;
            let center = light.pos + Vec3::Z * if downlight { -0.025 } else { 0.005 };
            let half = if downlight { 0.0275 } else { 0.0175 };
            let extent = vec3(half, half, 0.0);
            let separation = (min - (center + extent)).max(center - extent - max).max(Vec3::ZERO);
            if separation.length_squared() >= super::photometry::LIGHT_RANGE.powi(2)
                || (downlight && center.z <= min.z)
                || walls.iter().any(|wall| occludes(min, max, center, half, wall.min, wall.max))
            {
                masks[node.object as usize] &= !(1 << slot);
            }
        }
    }
    masks.into_boxed_slice()
}

fn occludes(min: Vec3, max: Vec3, source: Vec3, half: f32, wall_min: Vec3, wall_max: Vec3) -> bool {
    let emitter_min = source - vec3(half, half, 0.0);
    let emitter_max = source + vec3(half, half, 0.0);
    for axis in 0..3 {
        let plane = wall_min[axis].midpoint(wall_max[axis]);
        // The complete receiver and emitter must be on opposite sides.
        if !((max[axis] < plane && emitter_min[axis] > plane + 0.003)
            || (min[axis] > plane && emitter_max[axis] < plane - 0.003))
        {
            continue;
        }
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        let mut covered = true;
        'corners: for corner in 0..8 {
            let receiver = vec3(
                if corner & 1 == 0 { min.x } else { max.x },
                if corner & 2 == 0 { min.y } else { max.y },
                if corner & 4 == 0 { min.z } else { max.z },
            );
            for sample in 0..4 {
                let emitter = source
                    + vec3(if sample & 1 == 0 { -half } else { half }, if sample & 2 == 0 { -half } else { half }, 0.0);
                let t = (plane - receiver[axis]) / (emitter[axis] - receiver[axis]);
                let crossing = receiver + (emitter - receiver) * t;
                // Stay strictly inside the blocker, away from numerical edges.
                if crossing[a] <= wall_min[a] + 0.0001
                    || crossing[a] >= wall_max[a] - 0.0001
                    || crossing[b] <= wall_min[b] + 0.0001
                    || crossing[b] >= wall_max[b] - 0.0001
                {
                    covered = false;
                    break 'corners;
                }
            }
        }
        if covered {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_only_fully_covered_receiver_and_emitter_bounds() {
        let min = vec3(-2.0, -0.5, 0.5);
        let max = vec3(-1.0, 0.5, 1.5);
        let light = vec3(2.0, 0.0, 2.0);
        let wall_min = vec3(-0.1, -1.0, 0.0);
        let wall_max = vec3(0.1, 1.0, 3.0);
        assert!(occludes(min, max, light, 0.0275, wall_min, wall_max));
        assert!(
            !occludes(min, max, light, 0.0275, wall_min, vec3(0.1, 0.1, 3.0)),
            "part of the object sees past the wall"
        );
        assert!(
            !occludes(min, max, light, 0.0275, wall_min, vec3(0.1, 1.0, 1.0)),
            "part of the object sees over the wall"
        );
        assert!(!occludes(min, max, vec3(-3.0, 0.0, 2.0), 0.0275, wall_min, wall_max), "same side of wall");
    }
}
