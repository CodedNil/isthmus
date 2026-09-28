//! Grow static branches on the CPU; leaves remain individually culled SDF sheets.
use crate::home::{Kind, Material, Object};
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
        let blossom = self.plant.kind == Kind::Blossom;
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
                    if blossom {
                        self.add(Kind::Petal, attachment, Vec3::splat(s.x * (0.055 + 0.018 * (h + f).sin())), turn);
                    } else {
                        let length = s.x * (0.12 + 0.025 * (h * 3.1 + f).sin());
                        self.leaf(attachment, turn, length, length * 0.47, 0.20 + 0.40 * (f + h).sin().abs());
                    }
                }
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
        for branch in 0..5 {
            let angle = branch as f32 * 2.399_963;
            let dir = vec3(angle.cos(), angle.sin(), 0.0);
            let mut previous = Vec3::Z * (-s.z * 0.35);
            for node in 1..=9 {
                let t = node as f32 / 9.0;
                let tip = dir * s.x * (0.12 + 0.30 * t) + Vec3::Z * (-s.z * 0.35 - s.z * t * t);
                self.stem(previous, tip);
                self.leaf(tip, angle + if node % 2 == 0 { 0.65 } else { -0.65 }, s.x * 0.26, s.x * 0.14, 0.50);
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
            Kind::Plant | Kind::Blossom => growth.tree(),
            Kind::Palm => growth.palm(),
            Kind::SillPlant => growth.sill(),
            _ => {}
        }
    }
    result
}
