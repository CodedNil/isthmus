use isthmus::glam::{Vec2, Vec3};

const MIN_DISTANCE: f32 = 1.0;
const MAX_DISTANCE: f32 = 60.0;
/// Fixed overhead perspective: x points right and y points down.
#[derive(Clone, Copy)]
pub struct Camera {
    pub target: Vec2,
    pub distance: f32,
    pub fov: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self { target: Vec2::ZERO, distance: 18.0, fov: 0.65 }
    }
}
impl Camera {
    pub fn pan(&mut self, delta: Vec2, viewport: Vec2) {
        let units_per_pixel = 2.0 * self.distance * (self.fov * 0.5).tan() / viewport.y.max(1.0);
        self.target -= delta * units_per_pixel;
    }

    pub fn zoom(&mut self, amount: f32) {
        self.distance = (self.distance * (amount * 0.12).exp()).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn rays(self, aspect: f32) -> (Vec3, Vec3, Vec3, Vec3) {
        let scale = (self.fov * 0.5).tan();
        (self.target.extend(self.distance), -Vec3::Z, Vec3::X * (aspect * scale), -Vec3::Y * scale)
    }
}
