//! Grow static branches on the CPU; leaves remain individually culled SDF sheets.
use crate::home::{Kind, Material, Object};
use core::f32::consts::PI;
use isthmus::glam::{Vec3, vec2, vec3};

struct Growth<'a> {
    objects: &'a mut Vec<Object>,
    plant: Object,
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
            material: if kind == Kind::Stem && self.plant.kind != Kind::Palm && self.plant.kind != Kind::SillPlant {
                Material::Wood
            } else {
                Material::Foliage
            },
        });
    }

    fn stem(&mut self, a: Vec3, b: Vec3) {
        self.add(Kind::Stem, (a + b) * 0.5, b - a, 0.0);
    }

    fn leaf(&mut self, root: Vec3, angle: f32, length: f32, width: f32, droop: f32) {
        let lean = droop.clamp(-0.95, 0.95);
        let horizontal = (1.0 - lean * lean).sqrt();
        let direction = vec3(angle.cos() * horizontal, angle.sin() * horizontal, -lean);
        self.add(Kind::Leaf, root + direction * length * 0.48, vec3(length, width, length * lean), angle);
    }

    fn tree(&mut self) {
        let s = self.plant.size;
        // Asymmetric woody scaffolding; secondary shoots carry alternating leaves.
        for i in 0..14 {
            let f = i as f32;
            let angle = f * 2.399_963;
            let dir = vec3(angle.cos(), angle.sin(), 0.0);
            let start = vec3(0.012, -0.012, s.z * (-0.24 + f * 0.038));
            let reach = s.x * (0.35 + 0.08 * (f * 3.7).sin()) * (1.0 - f * 0.022);
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
                    let length = s.x * (0.12 + 0.025 * (h * 3.1 + f).sin());
                    self.leaf(attachment, turn, length, length * 0.47, 0.20 + 0.40 * (f + h).sin().abs());
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
            let reach = s.x * if inner { 0.16 } else { 0.40 + 0.06 * f.sin() };
            let arch =
                |t: f32| root + dir * reach * t + Vec3::Z * s.z * (1.45 * t - if inner { 0.62 } else { 0.84 } * t * t);
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
                            length * 0.15,
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
        for i in 0..9 {
            let f = i as f32;
            let angle = f * 2.399_963;
            let root = Vec3::Z * -s.z * 0.30;
            let tip = vec3(angle.cos() * s.x * 0.06, angle.sin() * s.x * 0.06, s.z * (0.10 + 0.13 * f.sin()));
            self.stem(root, tip);
            let hallway = s.z > 0.60;
            self.leaf(
                tip,
                angle,
                s.x * if hallway { 0.75 } else { 0.60 },
                s.x * if hallway { 0.37 } else { 0.30 },
                if hallway { -0.25 + 0.55 * (f * 1.7).sin() } else { -0.80 },
            );
        }
    }
}

pub(super) fn compile(objects: Vec<Object>) -> Vec<Object> {
    let mut result = Vec::with_capacity(objects.len() + 1800);
    for plant in objects {
        result.push(plant);
        let mut growth = Growth { objects: &mut result, plant };
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
