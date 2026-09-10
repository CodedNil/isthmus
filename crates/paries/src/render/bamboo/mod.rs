use super::{Frame, Program};
use isthmus::prelude::*;

mod habitat;
mod landscape;
mod undergrowth;
mod vegetation;

#[derive(Clone, Copy, PartialEq)]
enum Feature {
    Bamboo,
    Undergrowth,
    Rocks,
    Deadwood,
}

#[derive(Clone, Copy)]
struct Placement {
    bank_position: f32,
    depth: f32,
    seed: f32,
    feature: Feature,
}

impl Placement {
    fn position(self, size: Vec2) -> Vec2 {
        let y = 0.39 + self.depth * 0.77;
        let bank = landscape::river(y);
        let bank_side = self.bank_position.signum();
        let available = (size.x / size.y * 0.56 - bank_side * bank.x - bank.y).max(0.04);
        let mut x = bank.x + bank_side * (bank.y + 0.018 + self.bank_position.abs() * available);
        let uv_x = x * size.y / size.x + 0.5;
        if self.feature == Feature::Bamboo && uv_x > 0.6 {
            let opening = 0.8 + (uv_x - 0.6) * 0.72;
            x = (opening - 0.5) * size.x / size.y;
        }
        vec2(x, y)
    }
}

pub struct Bamboo {
    plants: Vec<Placement>,
}

impl Default for Bamboo {
    fn default() -> Self {
        let mut plants: Vec<_> = (0..280)
            .map(|index| {
                let seed = index as f32 * 19.31 + 7.0;
                let bamboo = index < 136;
                let depth = if bamboo {
                    ((index as f32 + hash(seed)) / 136.0).powf(1.5)
                } else {
                    (((index - 136) as f32 + hash(seed)) / 144.0).powf(1.1)
                };
                let side = if index % 2 == 0 { -1.0 } else { 1.0 };
                Placement {
                    bank_position: side
                        * (0.035
                            + ((index / 2) as f32 * 0.618_034 + side * 0.19 + hash(seed + 16.0) * 0.08).fract().abs()
                                * 1.08),
                    depth,
                    seed,
                    feature: if bamboo { Feature::Bamboo } else { Feature::Undergrowth },
                }
            })
            .collect();
        for (feature, bank_position, depth, seed) in [
            (Feature::Rocks, 0.08, 0.46, 117.0),
            (Feature::Rocks, -0.12, 0.65, 153.0),
            (Feature::Rocks, 0.4, 0.83, 187.0),
            (Feature::Deadwood, -0.5, 0.6, 93.0),
            (Feature::Deadwood, 0.75, 0.24, 211.0),
        ] {
            plants.push(Placement { bank_position, depth, seed, feature });
        }
        plants.sort_by(|left, right| left.depth.total_cmp(&right.depth));
        Self { plants }
    }
}

impl Bamboo {
    pub fn show(&mut self, frame: &mut Frame<'_>) {
        landscape::draw(frame);
        for plant in self.plants.iter().filter(|plant| plant.feature == Feature::Bamboo) {
            vegetation::draw(frame, *plant, true);
        }
        habitat::fish(frame);
        landscape::water(frame);
        for (index, plant) in self.plants.iter().enumerate() {
            let root = plant.position(frame.screen_size);
            match plant.feature {
                Feature::Bamboo => vegetation::draw(frame, *plant, false),
                Feature::Undergrowth => undergrowth::draw(frame, root, plant.depth, plant.seed),
                Feature::Rocks => habitat::rocks(frame, root, plant.depth, plant.seed),
                Feature::Deadwood => habitat::deadwood(frame, root, plant.depth, plant.seed),
            }
            if index == self.plants.len() / 3 || index == self.plants.len() * 2 / 3 {
                landscape::haze(frame, plant.depth);
            }
        }
        landscape::haze(frame, 1.0);
        landscape::falling_leaves(frame);
        landscape::butterflies(frame);
    }
}

fn atmosphere(uv: Vec2) -> Vec3 {
    let warmth = (uv.x * 0.85 + (1.0 - uv.y) * 0.25).smoothstep(0.15, 0.85);
    vec3(0.61, 0.72, 0.8).lerp(vec3(0.97, 0.82, 0.65), warmth)
}

fn daylight(color: Vec3, uv: Vec2) -> Vec3 {
    let warmth = uv.x.smoothstep(0.25, 0.85);
    let shade = uv.y.smoothstep(0.25, 0.95);
    color * vec3(0.87, 1.01, 1.17).lerp(vec3(1.13, 1.0, 0.91), warmth)
        + vec3(0.038, 0.006, 0.055) * shade * (1.0 - warmth)
}

fn segment(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let axis = end - start;
    (point - start - axis * ((point - start).dot(axis) / axis.length_squared().max(0.000_001)).saturate()).length()
}

fn hash(value: f32) -> f32 {
    (value.sin() * 43_758.547).fract().abs()
}

fn noise(point: Vec2) -> f32 {
    let cell = point.floor();
    let fraction = point - cell;
    let blend = fraction * fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    let corner = |offset: Vec2| {
        let p = cell + offset;
        let mut bits = (p.x as i32 as u32).wrapping_mul(0x9e37_79b9) ^ (p.y as i32 as u32).wrapping_mul(0x85eb_ca6b);
        bits = (bits ^ (bits >> 16)).wrapping_mul(0x7feb_352d);
        bits = (bits ^ (bits >> 15)).wrapping_mul(0x846c_a68b);
        (bits ^ (bits >> 16)) as f32 * 2.328_306_4e-10
    };
    corner(Vec2::ZERO).lerp(corner(Vec2::X), blend.x).lerp(corner(Vec2::Y).lerp(corner(Vec2::ONE), blend.x), blend.y)
}

fn wash(point: Vec2) -> f32 {
    let warp = vec2(noise(point * 0.7), noise(point * 0.7 + 31.0));
    noise(point + warp * 1.4) * 0.65 + noise(point * 2.7 + warp) * 0.35
}

fn pigment(color: Vec3, point: Vec2, pixel: f32) -> Vec3 {
    let pooling = wash(point * 24.0) - 0.5;
    let grain = (noise(point * 420.0) - 0.5) * (1.0 - pixel * 420.0).saturate();
    color * (1.0 + pooling * 0.18 + grain * 0.06)
}

fn wind(point: Vec2, time: f32) -> f32 {
    let gust = (point.x * 1.8 + point.y * 2.6 - time * 0.27).sin();
    gust * 0.6 + (point.x * 4.3 - point.y + time * 0.43).sin() * 0.25 + (time * 0.81 + point.x * 7.0).sin() * 0.15
}
