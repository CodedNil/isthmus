use isthmus::glam::{Vec2, Vec3, vec3};

const MIN_DISTANCE: f32 = 1.0;
const MAX_DISTANCE: f32 = 60.0;
const MAX_PITCH: f32 = 1.553_343;
const ORBIT_SPEED: f32 = 0.008;

#[derive(Clone, Copy)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { target: Vec3::ZERO, distance: 18.0, yaw: 0.0, pitch: 1.0, fov: 0.9 }
    }
}

impl Camera {
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * ORBIT_SPEED;
        self.pitch = (self.pitch + delta.y * ORBIT_SPEED).clamp(-MAX_PITCH, MAX_PITCH);
    }

    pub fn zoom(&mut self, amount: f32) {
        self.distance = (self.distance * (1.0 - amount * 0.12)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn rays(self, aspect: f32) -> (Vec3, Vec3, Vec3, Vec3) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let eye = self.target + vec3(cos_pitch * sin_yaw, cos_pitch * cos_yaw, sin_pitch) * self.distance;
        let forward = (self.target - eye).normalize();
        let scale = (self.fov * 0.5).tan();
        let right = forward.cross(Vec3::Z).normalize();
        let up = right.cross(forward) * scale;
        (eye, forward, right * (aspect * scale), up)
    }
}
