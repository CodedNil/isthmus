use isthmus::glam::{Vec3, vec3};

#[derive(Clone, Copy)]
pub struct View {
    pub name: &'static str,
    pub eye: Vec3,
    pub focus: Vec3,
    pub fov: f32,
}
impl View {
    pub const fn new(name: &'static str, eye: Vec3, focus: Vec3) -> Self {
        Self { name, eye, focus, fov: 1.30 }
    }

    pub const fn with_fov(mut self, fov: f32) -> Self {
        self.fov = fov;
        self
    }

    pub fn rays(self, aspect: f32) -> (Vec3, Vec3, Vec3) {
        let forward = (self.focus - self.eye).normalize();
        let right = if forward.z.abs() > 0.999 { Vec3::X } else { Vec3::Z.cross(forward).normalize() };
        let up = forward.cross(right).normalize();
        let scale = (self.fov * 0.5).tan();
        (forward, right * (aspect * scale), up * scale)
    }
}
pub fn all(radius: f32) -> [View; 17] {
    [
        View::new("plan", vec3(0.0, 0.0, (radius * 2.0).clamp(6.0, 40.0)), Vec3::ZERO).with_fov(0.65),
        View::new("entrance", vec3(-0.8500, -0.9500, 1.5500), vec3(0.0037, -1.4219, 1.3298)).with_fov(1.40000),
        View::new("bookshelf", vec3(-1.0408, 0.9973, 1.4547), vec3(-1.4782, 0.1170, 1.2711)).with_fov(1.14854),
        View::new("kitchen-fridge", vec3(-4.9526, 0.0112, 1.6746), vec3(-4.1448, -0.5523, 1.5012)).with_fov(0.94981),
        View::new("kitchen-window", vec3(-4.2804, -0.7666, 1.3001), vec3(-4.2728, -1.7653, 1.2499)).with_fov(1.36277),
        View::new("kitchen-cooker", vec3(-3.2400, 0.2997, 1.5868), vec3(-3.9990, -0.3274, 1.4118)).with_fov(1.04192),
        View::new("lounge", vec3(-2.3985, 0.1780, 1.2997), vec3(-3.2580, 0.6124, 1.0305)).with_fov(1.04914),
        View::new("desk", vec3(-2.3447, 0.5002, 1.3007), vec3(-1.5230, 1.0386, 1.1141)).with_fov(0.94988),
        View::new("bedroom", vec3(3.9, 0.0, 1.5), vec3(5.5, 2.5, 0.9)),
        View::new("hall", vec3(2.9, -0.35, 1.55), vec3(-0.5, -0.35, 1.25)),
        View::new("bathroom", vec3(2.1, -0.70, 1.65), vec3(1.2, -2.0, 0.95)),
        View::new("ensuite", vec3(1.72, 1.90, 1.65), vec3(0.65, 2.50, 0.8)),
        View::new("boiler-room", vec3(1.5, -0.15, 1.75), vec3(1.5, 0.85, 1.0)).with_fov(1.65),
        View::new("spare-room", vec3(3.2, -1.25, 1.55), vec3(4.6, -2.6, 1.2)),
        View::new("storage-top", vec3(-0.55, -2.35, 1.65), vec3(-1.75, -2.35, 1.05)),
        View::new("storage-bottom", vec3(-0.55, -1.25, 1.65), vec3(-1.75, -1.25, 1.05)),
        View::new("overhead", vec3(5.5, 6.0, 8.0), Vec3::ZERO),
    ]
}
