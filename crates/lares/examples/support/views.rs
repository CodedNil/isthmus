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
pub fn all(radius: f32) -> [View; 23] {
    [
        View::new("plan", vec3(0.0, 0.0, (radius * 2.0).clamp(6.0, 40.0)), Vec3::ZERO).with_fov(0.65),
        View::new("entrance", vec3(-0.85431, -0.38029, 1.35667), vec3(-0.17642, -1.07997, 1.13102)).with_fov(1.10561),
        View::new("bookshelf", vec3(-1.15423, 0.72441, 1.11246), vec3(-1.39835, -0.22112, 0.89707)).with_fov(1.12711),
        View::new("kitchen-fridge", vec3(-4.95260, 0.01120, 1.67460), vec3(-4.26375, -0.69012, 1.49117))
            .with_fov(1.13397),
        View::new("kitchen-window", vec3(-4.33227, -0.41070, 1.44069), vec3(-4.31548, -1.40443, 1.33016))
            .with_fov(1.25156),
        View::new("kitchen-cooker", vec3(-3.50531, 0.31370, 1.51752), vec3(-4.27181, -0.30707, 1.35285))
            .with_fov(1.08999),
        View::new("lounge", vec3(-2.75995, 0.37564, 1.45681), vec3(-3.57957, 0.82981, 1.10758)).with_fov(1.04666),
        View::new("desk", vec3(-1.98664, 0.34030, 1.24400), vec3(-1.23941, 0.96279, 1.01129)).with_fov(1.04362),
        View::new("dining", vec3(-2.40034, -0.02514, 1.39139), vec3(-2.27983, 0.95542, 1.23654)).with_fov(1.03674),
        View::new("kitchen-cabinet", vec3(-4.05000, -1.80000, 1.55000), vec3(-3.44749, -2.57425, 1.74370))
            .with_fov(1.16807),
        View::new("bedroom1", vec3(2.50520, 2.30455, 1.15277), vec3(3.43977, 1.95614, 1.08064)).with_fov(1.10820),
        View::new("bedroom2", vec3(3.90121, -0.64343, 1.34542), vec3(4.40665, 0.16946, 1.05602)).with_fov(1.34120),
        View::new("bedroom3", vec3(4.73941, 0.03663, 1.51359), vec3(3.89370, 0.51838, 1.28403)).with_fov(1.22692),
        View::new("hall", vec3(2.9, -0.35, 1.55), vec3(-0.5, -0.35, 1.25)),
        View::new("bathroom1", vec3(2.17413, -1.11655, 1.51711), vec3(1.53695, -1.83463, 1.23719)).with_fov(1.56842),
        View::new("bathroom2", vec3(0.86351, -1.58123, 1.58850), vec3(1.74380, -1.98533, 1.33991)).with_fov(1.37318),
        View::new("ensuite1", vec3(2.07304, 2.42122, 1.55360), vec3(1.12789, 2.18927, 1.32361)).with_fov(1.65041),
        View::new("ensuite2", vec3(0.65938, 1.56283, 1.76520), vec3(1.26054, 2.23295, 1.32983)).with_fov(1.35449),
        View::new("boiler", vec3(1.12309, -0.69455, 1.80088), vec3(1.53195, 0.11364, 1.37702)).with_fov(1.44514),
        View::new("spare", vec3(3.30182, -1.38197, 1.18138), vec3(4.09022, -1.97745, 1.02704)).with_fov(1.53604),
        View::new("storage-top", vec3(-0.55, -2.35, 1.65), vec3(-1.75, -2.35, 1.05)),
        View::new("storage-bottom", vec3(-0.55, -1.25, 1.65), vec3(-1.75, -1.25, 1.05)),
        View::new("overhead", vec3(5.5, 6.0, 8.0), Vec3::ZERO),
    ]
}
