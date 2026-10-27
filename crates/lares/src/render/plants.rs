//! Grow static branches on the CPU; leaves remain individually culled SDF sheets.
use crate::home::{DEFAULT_LIGHT_COLOR, Kind, Material, Object};
use core::f32::consts::PI;
use isthmus::glam::{Vec3, vec2, vec3};

struct Growth<'a> {
    objects: &'a mut Vec<Object>,
    plant: Object,
    walls: &'a [Object],
}
impl Growth<'_> {
    fn add(&mut self, kind: Kind, center: Vec3, size: Vec3, turn: f32) {
        let r = self.plant.rotation;
        let (sin, cos) = turn.sin_cos();
        self.objects.push(Object {
            pos: self.plant.pos + vec3(center.x * r.y - center.y * r.x, center.x * r.x + center.y * r.y, center.z),
            size,
            rotation: vec2(sin * r.y + cos * r.x, cos * r.y - sin * r.x),
            kind,
            emission: 0.0,
            emission_color: DEFAULT_LIGHT_COLOR,
            exterior: Vec3::ZERO,
            material: if self.plant.kind == Kind::SillPlant && self.plant.size.z > 0.60 {
                Material::MarbledFoliage
            } else if kind == Kind::Leaf && self.plant.kind == Kind::Plant {
                Material::VariegatedFoliage
            } else if kind == Kind::Stem && self.plant.kind != Kind::Palm && self.plant.kind != Kind::SillPlant {
                Material::Wood
            } else {
                Material::Foliage
            },
        });
    }

    // A scene-derived deformation field, evaluated once during growth. Each wall
    // uses the side occupied by the pot, so even a tip beyond the wall is brought
    // back indoors. Finite wall extents preserve doorways and corners.
    fn repel(&self, local: Vec3, margin: f32) -> Vec3 {
        let r = self.plant.rotation;
        let world = self.plant.pos + vec3(local.x * r.y - local.y * r.x, local.x * r.x + local.y * r.y, local.z);
        let mut point = world;
        for _ in 0..3 {
            for wall in self.walls {
                let axis = usize::from(wall.size.x >= wall.size.y);
                let tangent = 1 - axis;
                let half = wall.size * 0.5;
                if (point[tangent] - wall.pos[tangent]).abs() > half[tangent] + margin
                    || (point.z - wall.pos.z).abs() > half.z + margin
                {
                    continue;
                }
                let side = if self.plant.pos[axis] >= wall.pos[axis] { 1.0 } else { -1.0 };
                // Contact projection only: untouched foliage stays where it grew.
                let clearance = side * (point[axis] - wall.pos[axis]) - half[axis] - margin;
                let push = (-clearance).max(0.0);
                point[axis] += side * push;
            }
        }
        let q = point - self.plant.pos;
        vec3(q.x * r.y + q.y * r.x, -q.x * r.x + q.y * r.y, q.z)
    }

    fn stem(&mut self, a: Vec3, b: Vec3) {
        let a = self.repel(a, 0.012);
        let b = self.repel(b, 0.012);
        self.add(Kind::Stem, (a + b) * 0.5, b - a, 0.0);
    }

    fn leaf(&mut self, root: Vec3, angle: f32, length: f32, width: f32, droop: f32) {
        let lean = droop.clamp(-0.95, 0.95);
        let horizontal = (1.0 - lean * lean).sqrt();
        let direction = vec3(angle.cos() * horizontal, angle.sin() * horizontal, -lean);
        let attachment = self.repel(root, 0.012);
        let start = self.repel(root, 0.003);
        let tip = self.repel(root + direction * length, 0.003);
        let delta = tip - start;
        let length = delta.length().max(0.001);
        let yaw = delta.y.atan2(delta.x);
        let (sin, cos) = yaw.sin_cos();
        let lean = (delta.z / length).clamp(-0.95, 0.95);
        let upright = (1.0 - lean * lean).sqrt();
        let mut center = start + delta * 0.48;
        // Project the actual curved blade, rather than reserving its entire
        // width on every wall axis. Contact leaves retain a natural silhouette.
        for _ in 0..3 {
            for u in -10..=10 {
                let fraction = u as f32 / 10.0;
                let x = length * 0.5 * fraction;
                for v in [-1.0, 0.0, 1.0] {
                    let y = width * 0.5 * (1.0 - fraction * fraction) * v;
                    let z = -0.18 * x * x / (length * 0.5) + 0.32 * y.abs();
                    let q = vec3(x * upright - z * lean, y, z * upright + x * lean);
                    let point = center + vec3(q.x * cos - q.y * sin, q.x * sin + q.y * cos, q.z);
                    center += self.repel(point, 0.003) - point;
                }
            }
        }
        let displaced_start = center - delta * 0.48;
        if attachment.distance_squared(displaced_start) > 0.0001 {
            self.stem(attachment, displaced_start);
        }
        self.add(Kind::Leaf, center, vec3(length, width, -delta.z), yaw);
    }

    fn tree(&mut self) {
        let s = self.plant.size;
        // Asymmetric woody scaffolding; secondary shoots carry alternating leaves.
        for i in 0..14 {
            let f = i as f32;
            let angle = f * 2.399_963;
            let dir = vec3(angle.cos(), angle.sin(), 0.0);
            let start = vec3(0.012, -0.012, s.z * (-0.24 + f * 0.038));
            let reach = s.x * (0.30 + 0.09 * (f * 3.7).sin()) * (1.0 - f * 0.008);
            let elbow = start + dir * reach * 0.55 + Vec3::Z * s.z * 0.11;
            let tip = start + dir * reach + Vec3::Z * s.z * (0.16 + 0.035 * f.sin());
            self.stem(start, elbow);
            self.stem(elbow, tip);
            for j in 0..4 {
                let g = j as f32;
                let root = start.lerp(tip, (g + 1.0) / 5.0);
                let turn = angle + if j % 2 == 0 { -0.80 } else { 0.95 };
                let shoot = root + vec3(turn.cos(), turn.sin(), 0.55) * s.x * (0.13 + 0.025 * (f + g).sin());
                self.stem(root, shoot);
                for k in 0..6 {
                    let h = k as f32;
                    let attachment = root.lerp(shoot, (h + 1.0) / 6.0);
                    let turn = turn + if k % 2 == 0 { -0.9 } else { 0.9 } + 0.18 * (f + h).sin();
                    let length = s.x * (0.09 + 0.020 * (h * 3.1 + f).sin());
                    self.leaf(attachment, turn, length, length * 0.56, 0.20 + 0.40 * (f + h).sin().abs());
                }
            }
        }
    }

    fn blossom(&mut self) {
        let size = self.plant.size;
        // Uneven cut sprays: short, spreading branches below a few taller tips.
        for i in 0..9 {
            let branch = i as f32;
            let angle = branch * 2.399_963;
            let dir = vec3(angle.cos() * 0.65, 0.35 + 0.65 * angle.sin(), 0.0);
            let root = dir * 0.020 - Vec3::Z * size.z * 0.29;
            let height = 0.10 + 0.36 * (branch * 1.7).sin().abs();
            let reach = size.x * (0.28 + 0.20 * (branch * 2.3).cos().abs());
            let point = |t: f32| {
                root + (dir * reach + Vec3::Y * 0.14) * t * t
                    + vec3(-dir.y, dir.x, 0.0) * (t * PI).sin() * 0.035
                    + Vec3::Z * size.z * (height + 0.29) * (1.16 * t - 0.16 * t * t)
            };
            for segment in 0..8 {
                self.stem(point(segment as f32 / 8.0), point((segment + 1) as f32 / 8.0));
            }
            for j in 0..6 {
                let shoot_index = j as f32;
                let start = point(0.30 + shoot_index * 0.115);
                let turn = angle + if j % 2 == 0 { -1.10 } else { 1.05 };
                let spread = size.x * (0.13 + 0.10 * (branch + shoot_index * 2.7).sin().abs());
                let end = start
                    + vec3(
                        turn.cos() * spread,
                        turn.sin() * spread,
                        size.z * (0.035 + 0.105 * (branch * 2.1 + shoot_index).sin().abs()),
                    );
                let shoot = |t: f32| start.lerp(end, t) - Vec3::Z * (t * (1.0 - t) * 0.10);
                for segment in 0..4 {
                    self.stem(shoot(segment as f32 / 4.0), shoot((segment + 1) as f32 / 4.0));
                }
                for k in 0..3 {
                    let node_index = k as f32;
                    let node = shoot(0.35 + node_index * 0.30);
                    let phase = angle + shoot_index * 1.9 + node_index * 2.4;
                    let twig = node + vec3(phase.cos() * 0.045, phase.sin() * 0.045, 0.015 + 0.045 * phase.cos().abs());
                    self.stem(node, twig);
                    for flower in 0..3 {
                        let flower_index = flower as f32;
                        let turn = phase + flower_index * 2.399_963;
                        let center = node.lerp(twig, 0.25 + flower_index * 0.35)
                            + vec3(turn.cos() * 0.023, turn.sin() * 0.023, 0.009 * flower_index.sin());
                        let diameter = 1.5
                            * (0.023 + 0.009 * (branch + shoot_index * 2.1 + node_index + flower_index).sin().abs());
                        self.add(
                            Kind::Petal,
                            center,
                            vec3(diameter, diameter, diameter * (0.65 + 0.30 * turn.sin().abs())),
                            turn,
                        );
                    }
                }
            }
            for bud in 0..5 {
                let phase = branch * 1.7 + bud as f32 * 2.4;
                let center = point(0.86 + bud as f32 * 0.035) + vec3(phase.cos() * 0.012, phase.sin() * 0.012, 0.0);
                self.add(Kind::Petal, center, vec3(0.0345, 0.0345, 0.030), phase);
            }
        }
    }

    fn palm(&mut self) {
        let s = self.plant.size;
        for i in 0..14 {
            let f = i as f32;
            let angle = f * 2.399_963;
            let dir = vec3(angle.cos(), angle.sin(), 0.0);
            let root = dir * 0.022 - Vec3::Z * s.z * 0.32;
            // Upright inner fronds fill the crown; outer fronds arch outward.
            let inner = i % 3 == 0;
            let reach = s.x * if inner { 0.16 } else { 0.43 + 0.10 * f.sin() };
            let arch = |t: f32| {
                root + dir * reach * t
                    + Vec3::Z * s.z * (1.48 * t - if inner { 0.66 } else { 1.02 + 0.12 * f.sin() } * t * t)
            };
            let mut previous = root;
            for j in 1..=16 {
                let t = j as f32 / 16.0;
                let point = arch(t);
                self.stem(previous, point);
                previous = point;
                if j >= 3 {
                    let length = s.x * 0.27 * (1.0 - t * 0.75);
                    for side in [-1.0, 1.0] {
                        self.leaf(
                            point,
                            angle + side * 1.05,
                            length,
                            length * 0.11,
                            if inner { -0.65 } else { t * 0.55 - 0.35 },
                        );
                    }
                }
            }
        }
    }

    fn trailing(&mut self) {
        let s = self.plant.size;
        // Uneven vines and alternating leaves spill over the cabinet edge.
        for branch in 0..7 {
            let f = branch as f32;
            let angle = f * 2.399_963;
            let dir = vec3(angle.cos(), angle.sin(), 0.0);
            let root = Vec3::Z * (s.z * 0.5 - 0.05);
            let length = s.z * (0.50 + 0.60 * (f * 1.7).sin().abs());
            let point = |t: f32| {
                root + dir * s.x * (0.06 + (0.20 + 0.12 * f.sin()) * t)
                    + Vec3::Y * s.y * 0.22
                    + vec3((t * 7.0 + f).sin() * 0.035, 0.0, -length * t * t)
            };
            let mut previous = root;
            for node in 0..=10 {
                let n = node as f32;
                let t = (n + 0.22 * (f * 3.1 + n).sin()).max(0.0) / 10.0;
                let tip = point(t);
                self.stem(previous, tip);
                let side = if node % 2 == 0 { -1.0 } else { 1.0 };
                let turn = angle + side * 1.1 + 0.5 * (f + n * 1.9).sin();
                let leaf = s.x * (0.19 + 0.13 * (f * 2.1 + n * 1.7).cos().abs());
                self.leaf(tip, turn, leaf, leaf * 0.72, -0.15 + 0.65 * (f + n).sin().abs());
                if node % 3 == 0 {
                    self.leaf(tip, turn + 1.8, leaf * 0.8, leaf * 0.60, 0.35);
                }
                previous = tip;
            }
        }
    }

    fn sill(&mut self) {
        let s = self.plant.size;
        let hallway = s.z > 0.60;
        for i in 0..if hallway { 7 } else { 9 } {
            let f = i as f32;
            let angle = f * 2.399_963;
            let root = Vec3::Z * -s.z * 0.30;
            let tip = vec3(angle.cos() * s.x * 0.06, angle.sin() * s.x * 0.06, s.z * (0.10 + 0.13 * f.sin()));
            self.stem(root, tip);
            self.leaf(
                tip,
                angle,
                s.x * if hallway { 0.75 } else { 0.60 },
                s.x * if hallway { 0.45 } else { 0.30 },
                if hallway { 0.10 + 0.65 * (f * 1.7).sin() } else { -0.80 },
            );
        }
    }
}

pub(super) fn compile(objects: Vec<Object>) -> Vec<Object> {
    let walls: Vec<_> = objects
        .iter()
        .copied()
        .filter(|o| {
            (o.kind == Kind::Box && o.material == Material::Paint && o.size.z > 0.25 && o.size.x.min(o.size.y) < 0.36)
                || o.kind == Kind::Window
        })
        .map(|mut o| {
            // Window clearance follows its thin glazing plane, in world axes.
            if o.kind == Kind::Window && o.rotation.x.abs() > 0.5 {
                o.size = vec3(o.size.y, o.size.x, o.size.z);
            }
            o
        })
        .collect();
    let mut result = Vec::with_capacity(objects.len() + 1800);
    for plant in objects {
        result.push(plant);
        let mut growth = Growth { objects: &mut result, plant, walls: &walls };
        match plant.kind {
            Kind::TrailingPlant => growth.trailing(),
            Kind::Plant => growth.tree(),
            Kind::Blossom => growth.blossom(),
            Kind::Palm => growth.palm(),
            Kind::SillPlant => growth.sill(),
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_clearance_keeps_foliage_and_stems_inside_walls_across_placements() {
        // Exercise scene-derived deformation with different species, rotations,
        // distances and a corner. Sample the actual curved leaf sheet, not just
        // its centre; this catches leaves whose tips clear but sides intersect.
        for kind in [Kind::Plant, Kind::Palm, Kind::SillPlant] {
            for degree in (0..360).step_by(45) {
                for clearance in [0.15, 0.35, 0.60] {
                    let (sin, cos) = (degree as f32).to_radians().sin_cos();
                    let plant = Object {
                        rotation: vec2(sin, cos),
                        ..Object::new(kind, vec3(clearance, clearance, 1.0), vec3(1.2, 1.2, 1.6), Material::Foliage)
                    };
                    let walls = [
                        Object::new(Kind::Box, vec3(-0.05, 0.0, 1.2), vec3(0.10, 8.0, 2.4), Material::Paint),
                        Object::new(Kind::Box, vec3(0.0, -0.05, 1.2), vec3(8.0, 0.10, 2.4), Material::Paint),
                    ];
                    let parts = compile(vec![walls[0], walls[1], plant]);
                    for part in &parts[3..] {
                        let rotation = part.rotation;
                        let world = |p: Vec3| {
                            part.pos
                                + vec3(p.x * rotation.y - p.y * rotation.x, p.x * rotation.x + p.y * rotation.y, p.z)
                        };
                        if part.kind == Kind::Stem {
                            for endpoint in [-0.5, 0.5] {
                                let p = world(part.size * endpoint);
                                assert!(p.x >= 0.004 && p.y >= 0.004, "stem crosses architectural wall");
                            }
                        } else if part.kind == Kind::Leaf {
                            let lean = (-part.size.z / part.size.x).clamp(-0.95, 0.95);
                            let upright = (1.0 - lean * lean).sqrt();
                            for u in -5..=5 {
                                let x = part.size.x * 0.5 * u as f32 / 5.0;
                                for v in [-1.0, 0.0, 1.0] {
                                    let y = part.size.y * 0.5 * (1.0 - (u as f32 / 5.0).powi(2)) * v;
                                    let z = -0.18 * x * x / (part.size.x * 0.5) + 0.32 * y.abs();
                                    let p = world(vec3(x * upright - z * lean, y, z * upright + x * lean));
                                    assert!(p.x >= 0.0 && p.y >= 0.0, "curved leaf crosses architectural wall: {p:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
