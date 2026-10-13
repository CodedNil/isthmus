//! Geometry transport and per-fixture bounce bases never change with the clock.
//! Sun visibility refreshes incrementally; two diffuse bounces use no feedback.
use super::{Grid, Probe, SPACING};
use crate::{
    home::{Kind, Material, Object},
    render::{
        Sun,
        acceleration::{BRANCH, Node},
        scene::{self, FAR, fixtures},
    },
};
use core::{array::from_fn, f32::consts::PI};
use isthmus::glam::{UVec3, Vec2, Vec3, uvec3, vec2, vec3};

const RAYS: usize = 32;
const UPDATE_BUDGET: usize = 16;

#[derive(Clone, Copy, Default)]
struct Coefficients {
    constant: Vec3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

impl Coefficients {
    fn project(&mut self, direction: Vec3, radiance: Vec3) {
        self.constant += radiance / RAYS as f32;
        self.x += radiance * (direction.x * 2.0 / RAYS as f32);
        self.y += radiance * (direction.y * 2.0 / RAYS as f32);
        self.z += radiance * (direction.z * 2.0 / RAYS as f32);
    }

    fn add(&mut self, other: Self, weight: Vec3) {
        self.constant += other.constant * weight;
        self.x += other.x * weight;
        self.y += other.y * weight;
        self.z += other.z * weight;
    }

    const fn probe(self, clearance: Probe) -> Probe {
        Probe { constant: self.constant, x: self.x, y: self.y, z: self.z, ..clearance }
    }

    const fn from_probe(probe: Probe) -> Self {
        Self { constant: probe.constant, x: probe.x, y: probe.y, z: probe.z }
    }
}
#[derive(Clone, Copy, Default)]
struct Bounce {
    point: Vec3,
    normal: Vec3,
    albedo: Vec3,
}

pub struct Cache {
    directions: [Vec3; RAYS],
    paths: Box<[Bounce]>,
    sky: Box<[Coefficients]>,
    sun: Box<[Coefficients]>,
    fixtures: Box<[Coefficients]>,
    /// Direct sources and their first bounce; second bounce only reads this field.
    direct: Box<[Probe]>,
    cursor: usize,
    remaining: usize,
}
impl Cache {
    fn sun_at(&self, index: usize, sun: Sun, objects: &[Object], nodes: &[Node]) -> Coefficients {
        let mut result = Coefficients::default();
        if sun.radiance.max_element() <= 0.0 {
            return result;
        }
        for (sample, bounce) in self.paths[index * RAYS..(index + 1) * RAYS].iter().enumerate() {
            let cosine = bounce.normal.dot(sun.direction).max(0.0);
            if cosine <= 0.0 || bounce.albedo == Vec3::ZERO {
                continue;
            }
            let visibility = scene::cast::<true, false>(bounce.point, sun.direction, nodes, objects, 0.0, FAR).0;
            result.project(self.directions[sample], bounce.albedo * (cosine * visibility / PI));
        }
        result
    }

    fn compose_at(&mut self, index: usize, sun: Sun, objects: &[Object], lights: &[u32]) {
        let mut result = Coefficients::default();
        result.add(self.sky[index], sun.sky);
        result.add(self.sun[index], sun.radiance);
        for (light, &object) in lights.iter().enumerate() {
            result.add(self.fixtures[index * lights.len() + light], Vec3::splat(objects[object as usize].emission));
        }
        self.direct[index] = result.probe(self.direct[index]);
    }

    fn second_bounce(&self, index: usize, grid: Grid) -> Probe {
        let mut result = Coefficients::from_probe(self.direct[index]);
        for (sample, bounce) in self.paths[index * RAYS..(index + 1) * RAYS].iter().enumerate() {
            if bounce.albedo == Vec3::ZERO {
                continue;
            }
            let incident = grid.sample(&self.direct, bounce.point, bounce.normal);
            // A conservative gain damps low-order SH ringing. There is no
            // iterative accumulation: brightness cannot grow between frames.
            result.project(self.directions[sample], incident * bounce.albedo * 0.65);
        }
        result.probe(self.direct[index])
    }

    /// State changes blend precomputed light bases, without casting any rays.
    pub(crate) fn compose(&mut self, sun: Sun, objects: &[Object], lights: &[u32], grid: Grid, probes: &mut [Probe]) {
        for index in 0..self.direct.len() {
            self.compose_at(index, sun, objects, lights);
        }
        for (index, probe) in probes.iter_mut().enumerate() {
            *probe = self.second_bounce(index, grid);
        }
    }

    pub(crate) const fn updating(&self) -> bool {
        self.remaining != 0
    }

    pub(crate) fn begin_update(&mut self) {
        self.cursor = 0;
        self.remaining = self.direct.len() * 2;
    }

    /// At most 16 probes / 512 cached paths per frame, independent of resolution.
    pub(crate) fn step(
        &mut self,
        sun: Sun,
        objects: &[Object],
        nodes: &[Node],
        lights: &[u32],
        grid: Grid,
        probes: &mut [Probe],
    ) {
        if self.remaining == 0 {
            return;
        }
        let count = self.direct.len();
        let transport_pass = self.cursor < count;
        let start = if transport_pass { self.cursor } else { self.cursor - count };
        let end = (start + UPDATE_BUDGET).min(count);
        if transport_pass {
            for index in start..end {
                self.sun[index] = self.sun_at(index, sun, objects, nodes);
                self.compose_at(index, sun, objects, lights);
            }
        }
        // The second sweep removes dependence on the order in which direct
        // probes were refreshed. Both sweeps have the same bounded budget.
        for (index, probe) in probes.iter_mut().enumerate().take(end).skip(start) {
            *probe = self.second_bounce(index, grid);
        }
        self.remaining -= end - start;
        self.cursor += end - start;
    }

    pub(crate) fn relight(
        &mut self,
        sun: Sun,
        objects: &[Object],
        nodes: &[Node],
        lights: &[u32],
        grid: Grid,
        probes: &mut [Probe],
    ) {
        for index in 0..self.direct.len() {
            self.sun[index] = self.sun_at(index, sun, objects, nodes);
        }
        self.compose(sun, objects, lights, grid, probes);
        self.remaining = 0;
        self.cursor = 0;
    }
}

pub fn bake(
    objects: &[Object],
    nodes: &[Node],
    lights: &[u32],
    min: Vec3,
    max: Vec3,
    sun: Sun,
) -> (Grid, Box<[Probe]>, Cache) {
    let origin = vec3(min.x, min.y, 0.10);
    let size = ((vec3(max.x, max.y, 2.6) - origin) / SPACING).ceil().as_uvec3() + UVec3::ONE;
    let grid = Grid { origin, size };
    let count = (size.x * size.y * size.z) as usize;
    let directions = from_fn(|sample| {
        let h = 1.0 - 2.0 * (sample as f32 + 0.5) / RAYS as f32;
        let (sin, cos) = (sample as f32 * 2.399_963).sin_cos();
        let r = (1.0 - h * h).sqrt();
        vec3(r * cos, r * sin, h)
    });
    let mut cache = Cache {
        directions,
        paths: vec![Bounce::default(); count * RAYS].into_boxed_slice(),
        sky: vec![Coefficients::default(); count].into_boxed_slice(),
        sun: vec![Coefficients::default(); count].into_boxed_slice(),
        fixtures: vec![Coefficients::default(); count * lights.len()].into_boxed_slice(),
        direct: vec![Probe::default(); count].into_boxed_slice(),
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
            if scene::render_furniture::<scene::Distance>(object.kind, local, object.size).0 < -0.002 {
                return true;
            }
        }
        false
    };
    for index in 0..count {
        let coordinate = uvec3(index as u32 % size.x, index as u32 / size.x % size.y, index as u32 / (size.x * size.y));
        let point = origin + coordinate.as_vec3() * SPACING;
        if inside(point) {
            cache.direct[index].positive = Vec3::splat(-FAR);
            cache.direct[index].negative = Vec3::splat(-FAR);
            continue;
        }
        for axis in 0..3 {
            let mut direction = Vec3::ZERO;
            direction[axis] = 1.0;
            cache.direct[index].positive[axis] =
                (scene::cast::<false, false>(point, direction, nodes, objects, 0.0, FAR).0 - 0.002).max(0.0);
            cache.direct[index].negative[axis] =
                (scene::cast::<false, false>(point, -direction, nodes, objects, 0.0, FAR).0 - 0.002).max(0.0);
        }
        for (sample, &direction) in directions.iter().enumerate() {
            let (distance, object) = scene::cast::<false, false>(point, direction, nodes, objects, 0.0, FAR);
            if distance >= FAR {
                cache.sky[index].project(direction, Vec3::splat(0.25 + direction.z.max(0.0).sqrt() * 0.75));
                continue;
            }
            let object = objects[object as usize];
            let hit = point + direction * distance;
            let local = scene::unrotate_z(hit - object.pos, object.rotation);
            let normal =
                scene::unrotate_z(scene::object_normal(object, local), vec2(-object.rotation.x, object.rotation.y));
            let albedo = average_albedo(object, local).min(Vec3::splat(0.85));
            let point = hit + normal * 0.015;
            cache.paths[index * RAYS + sample] = Bounce { point, normal, albedo };
            for (light, &fixture) in lights.iter().enumerate() {
                // Unit brightness: entity changes only scale the cached transport.
                let fixture = Object { emission: 1.0, ..objects[fixture as usize] };
                let (direction_to_light, radiance, distance_to_light) = fixtures::incident(fixture, point, Vec2::ZERO);
                let contribution = albedo * radiance * (normal.dot(direction_to_light).max(0.0) / PI);
                if contribution.max_element() < 0.0005 {
                    continue;
                }
                let visibility = scene::cast::<true, false>(
                    point,
                    direction_to_light,
                    nodes,
                    objects,
                    0.0,
                    distance_to_light - 0.002,
                )
                .0;
                cache.fixtures[index * lights.len() + light].project(direction, contribution * visibility);
            }
        }
    }
    let mut probes = vec![Probe::default(); count].into_boxed_slice();
    cache.relight(sun, objects, nodes, lights, grid, &mut probes);
    (grid, probes, cache)
}

fn average_albedo(object: Object, local: Vec3) -> Vec3 {
    let (material, tint) = if object.kind == Kind::Box {
        (object.material, if object.material == Material::Paint { scene::WALL_COLOR } else { Vec3::ONE })
    } else {
        let surface = scene::render_furniture::<scene::Hit>(object.kind, local, object.size);
        (surface.material, surface.tint)
    };
    let base = match material {
        Material::Floorboards => vec3(0.15, 0.085, 0.045),
        Material::Wood => vec3(0.35, 0.18, 0.075),
        Material::Maple | Material::MapleHorizontal => vec3(0.68, 0.58, 0.42),
        Material::Tile => vec3(0.035, 0.04, 0.045),
        Material::Metal | Material::Mirror => Vec3::splat(0.08),
        Material::Emissive => Vec3::splat(0.04),
        _ => Vec3::ONE,
    };
    tint.clamp(Vec3::ZERO, Vec3::ONE).powf(2.2) * base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        home::Room,
        render::{Scene, SunSettings},
    };
    const MIDNIGHT: f64 = 1_782_000_000.0;

    fn room(position: Vec2) -> Room {
        Room::new(position, Vec2::splat(3.0), Material::Carpet)
    }

    #[test]
    fn fixtures_bounce_onto_ceiling_and_dimming_does_not_accumulate_energy() {
        let room = room(Vec2::ZERO).downlights("light.test", &[(0.0, 0.0)]);
        let mut scene = Scene::new_at(&[room], MIDNIGHT, SunSettings::default());
        let sample = |scene: &Scene| scene.grid.sample(&scene.probes, vec3(0.0, 0.0, 2.25), -Vec3::Z);
        let lit = sample(&scene);
        scene.set_light_state("light.test", 0.0);
        let dark = sample(&scene);
        assert!(lit.x > dark.x + 0.001, "ceiling should receive bounced light: {lit:?}, {dark:?}");
        assert!(lit.x > lit.z, "warm emitters should tint the bounce");
        scene.set_light_state("light.test", 0.5);
        let dim = sample(&scene);
        assert!(dim.x > dark.x && dim.x < lit.x);
        scene.set_light_state("light.test", 1.0);
        assert!((sample(&scene) - lit).length() < 1.0e-6);
    }

    #[test]
    fn bounced_fixture_light_does_not_cross_a_closed_room_wall() {
        let left = room(vec2(-1.6, 0.0)).downlights("light.left", &[(0.0, 0.0)]);
        let right = room(vec2(1.6, 0.0));
        let mut scene = Scene::new_at(&[left, right], MIDNIGHT, SunSettings::default());
        let sample = |scene: &Scene| scene.grid.sample(&scene.probes, vec3(1.6, 0.0, 1.0), -Vec3::X);
        let lit = sample(&scene);
        scene.set_light_state("light.left", 0.0);
        assert!((lit - sample(&scene)).length() < 0.0001, "GI should respect the partition");
    }

    #[test]
    fn budgeted_updates_converge_to_a_full_relight() {
        let room = Room::new(Vec2::ZERO, Vec2::splat(3.0), Material::Carpet).downlights("light.test", &[(0.0, 0.0)]);
        let scene = Scene::new_at(&[room], MIDNIGHT, SunSettings::default());
        let (grid, mut probes, mut cache) =
            bake(&scene.objects, &scene.nodes, &scene.lights, scene.nodes[0].min, scene.nodes[0].max, scene.sun);
        let noon = Sun::at_utc(MIDNIGHT + 12.0 * 3600.0, SunSettings::default());
        cache.begin_update();
        let old = cache.remaining;
        cache.step(noon, &scene.objects, &scene.nodes, &scene.lights, grid, &mut probes);
        assert!(old - cache.remaining <= UPDATE_BUDGET);
        while cache.updating() {
            cache.step(noon, &scene.objects, &scene.nodes, &scene.lights, grid, &mut probes);
        }
        let incremental = probes.clone();
        cache.relight(noon, &scene.objects, &scene.nodes, &scene.lights, grid, &mut probes);
        for (a, b) in incremental.iter().zip(&probes) {
            assert!((a.constant - b.constant).length() < 1.0e-5);
            assert!((a.x - b.x).length() < 1.0e-5);
        }
    }
}
