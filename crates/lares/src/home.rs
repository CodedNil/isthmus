use isthmus::{
    ShaderData,
    glam::{Vec2, Vec3, vec2},
};

#[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
#[repr(u32)]
pub enum Material {
    #[default]
    Wood,
    Carpet,
    Tile,
    Fabric,
    Stone,
    Ceramic,
    Metal,
    Paint,
    Foliage,
    Soil,
    Floorboards,
    Shag,
    Knit,
    Emissive,
    Mirror,
    Maple,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
#[repr(u32)]
pub enum Kind {
    #[default]
    Box,
    Bed,
    Sofa,
    Table,
    Counter,
    KitchenSink,
    Kettle,
    EntryDoor,
    Microwave,
    CookerHood,
    Splashback,
    Blind,
    TrailingPlant,
    Cupboard,
    Fridge,
    Oven,
    Sink,
    Toilet,
    Bath,
    Shower,
    Radiator,
    Appliance,
    Chair,
    Plant,
    Ottoman,
    Rug,
    Workstation,
    DiningChair,
    Pegboard,
    Poster,
    PosterNight,
    PosterBlue,
    ShoeStorage,
    Mirror,
    DisplayShelf,
    WallSwords,
    Palm,
    SillPlant,
    WallArt,
    SideTable,
    Blossom,
    Window,
    SillArrangement,
    Curtain,
    /// Compiled vegetation parts; room authors place a whole plant.
    Leaf,
    Petal,
    Stem,
}

/// One representation for architecture and procedural furniture.
#[derive(Clone, Copy, ShaderData)]
pub struct Object {
    pub pos: Vec3,
    pub size: Vec3,
    /// Precomputed sine and cosine around z.
    pub rotation: Vec2,
    pub kind: Kind,
    pub material: Material,
}
impl Object {
    pub fn extent(self) -> Vec3 {
        match self.kind {
            Kind::Box => self.size * 0.5,
            Kind::KitchenSink => self.size * 0.5 + Vec3::new(0.06, 0.06, 0.32),
            Kind::Rug => self.size * 0.5 + Vec3::splat(0.005) + Vec3::Z * 0.04,
            Kind::Ottoman => self.size * 0.5 + Vec3::splat(0.04),
            Kind::Chair => Vec3::new(0.39, 0.43, 0.68),
            Kind::Workstation => Vec3::new(0.73, 0.62, 0.75),
            Kind::Plant | Kind::Blossom | Kind::Palm | Kind::SillPlant => {
                Vec3::new(self.size.x * 0.20, self.size.y * 0.20, self.size.z * 0.5)
            }
            Kind::SillArrangement | Kind::WallArt => self.size * 0.5 + Vec3::splat(0.05),
            Kind::Stem => self.size.abs() * 0.5 + Vec3::splat(0.004),
            Kind::Leaf => Vec3::new(
                self.size.x * 0.5 + 0.005,
                self.size.y * 0.5 + 0.005,
                self.size.z.abs() * 0.5 + self.size.x * 0.09 + self.size.y * 0.16 + 0.003,
            ),
            Kind::Petal => self.size * 0.5 + Vec3::splat(0.005),
            Kind::SideTable => Vec3::new(self.size.x * 0.5 + 0.02, self.size.y * 0.5 + 0.02, self.size.z * 0.5 + 0.44),
            Kind::Toilet => {
                (self.size * 0.5)
                    .max(Vec3::splat(self.size.x * 0.3) + Vec3::new(0.0, self.size.y * 0.2, self.size.z * 0.08))
                    + Vec3::splat(0.1)
            }
            _ => self.size * 0.5 + Vec3::splat(0.1),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Opening {
    pub pos: Vec3,
    pub size: Vec3,
}

bitflags::bitflags! {
    #[derive(Clone, Copy)]
    pub struct Walls: u32 { const LEFT=1; const TOP=2; const RIGHT=4; const BOTTOM=8; }
}

pub struct Room {
    pub position: Vec2,
    pub outline: Vec<Vec2>,
    pub walls: Walls,
    pub floor: Material,
    pub furniture: Vec<Object>,
    pub openings: Vec<Opening>,
}
impl Room {
    pub fn new(position: Vec2, size: Vec2, floor: Material) -> Self {
        let h = size * 0.5;
        Self {
            position,
            outline: vec![vec2(-h.x, -h.y), vec2(h.x, -h.y), h, vec2(-h.x, h.y)],
            floor,
            walls: Walls::all(),
            furniture: vec![],
            openings: vec![],
        }
    }

    pub const fn walls(mut self, walls: Walls) -> Self {
        self.walls = walls;
        self
    }

    /// Counterclockwise room-local outline, with horizontal/vertical edges.
    pub fn outline(mut self, vertices: &[(f32, f32)]) -> Self {
        self.outline = vertices.iter().copied().map(Vec2::from).collect();
        self
    }

    pub fn cut_wall(mut self, pos: Vec2, size: Vec2) -> Self {
        self.openings.push(Opening {
            pos: pos.extend(super::render::WALL_HEIGHT * 0.5),
            size: size.extend(super::render::WALL_HEIGHT * 2.0),
        });
        self
    }

    pub fn door(self, pos: Vec2, rotation: f32) -> Self {
        self.door_width(pos, rotation, 0.8)
    }

    pub fn window(self, pos: Vec2, rotation: f32) -> Self {
        self.window_width(pos, rotation, 1.2)
    }

    pub fn door_width(self, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.opening(pos, rotation, width, 0.0, super::render::DOOR_HEIGHT)
    }

    pub fn window_width(self, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.opening(pos, rotation, width, super::render::WINDOW_SILL, super::render::WINDOW_HEAD).furniture(
            Kind::Window,
            pos.extend(super::render::WINDOW_SILL),
            (width, 0.28, super::render::WINDOW_HEAD - super::render::WINDOW_SILL),
            rotation,
        )
    }

    fn opening(mut self, pos: Vec2, rotation: f32, width: f32, bottom: f32, top: f32) -> Self {
        let (sin, cos) = rotation.to_radians().sin_cos();
        let depth = super::render::WALL_THICKNESS * 4.0;
        self.openings.push(Opening {
            pos: pos.extend(f32::midpoint(bottom, top)),
            size: Vec3::new(cos.abs() * width + sin.abs() * depth, sin.abs() * width + cos.abs() * depth, top - bottom),
        });
        self
    }

    pub fn furniture(mut self, kind: Kind, pos: impl Into<Vec3>, size: impl Into<Vec3>, rotation: f32) -> Self {
        let (sin, cos) = rotation.to_radians().sin_cos();
        self.furniture.push(Object {
            kind,
            pos: pos.into(),
            size: size.into(),
            rotation: vec2(sin, cos),
            material: Material::Wood,
        });
        self
    }
}

pub fn rooms() -> Vec<Room> {
    vec![
        hall(),
        lounge(),
        kitchen(),
        storage(vec2(-1.65, 2.5)),
        storage(vec2(-1.65, 1.4)),
        bedroom(),
        ensuite(),
        boiler_room(),
        spare_room(),
        bathroom(),
    ]
}

/// Shared wood floor; only the outer edges have walls.
fn hall() -> Room {
    Room::new(vec2(1.35, 0.5), vec2(4.5, 1.1), Material::Floorboards)
        .walls(Walls::TOP)
        .outline(&[(-2.25, -0.55), (2.25, -0.55), (2.25, 0.55), (-1.15, 0.55), (-1.15, 2.55), (-2.25, 2.55)])
        .door(vec2(-1.7, 2.55), 0.0)
        .furniture(Kind::EntryDoor, (-1.7, 2.55, 0.0), (0.79, 0.045, 2.04), 0.0)
        .furniture(Kind::Radiator, (-0.425, 0.45, 0.0), (1.2, 0.1, 0.6), 0.0)
        .furniture(Kind::ShoeStorage, (-1.30, 1.87, 0.0), (0.76, 0.22, 1.02), 90.0)
        .furniture(Kind::SillPlant, (-1.40, 2.05, 1.025), (0.43, 0.43, 0.70), 0.0)
        .furniture(Kind::PosterBlue, (-1.215, 1.75, 1.36), (0.33, 0.02, 0.54), 90.0)
        .furniture(Kind::Mirror, (-1.215, 1.18, 0.12), (0.40, 0.035, 1.90), 90.0)
}

fn lounge() -> Room {
    Room::new(vec2(-2.75, -1.4), vec2(6.1, 2.7), Material::Floorboards)
        .walls(Walls::LEFT | Walls::BOTTOM)
        .cut_wall(vec2(-2.2, 1.35), vec2(1.6, 0.3))
        .cut_wall(vec2(-0.15, 1.8), vec2(0.2, 1.0))
        .outline(&[
            (-3.05, -1.35), (3.05, -1.35), (3.05, 1.35), (1.85, 1.35),
            (1.85, 2.25), (-0.15, 2.25), (-0.15, 1.35), (-3.05, 1.35),
        ])
        .window_width(vec2(-1.15, -1.35), 0.0, 1.4)
        .window_width(vec2(1.75, -1.35), 0.0, 1.4)
        .furniture(Kind::Radiator, (-1.15, -1.25, 0.0), (1.4, 0.1, 0.6), 0.0)
        .furniture(Kind::Curtain, (-1.15, -1.25, 0.67), (1.80, 0.38, 1.63), 0.0)
        .furniture(Kind::Curtain, (1.75, -1.25, 0.67), (1.80, 0.38, 1.63), 0.0)
        .furniture(Kind::SillArrangement, (-1.40, -1.33, 0.945), (0.24, 0.20, 0.64), 0.0)
        .furniture(Kind::Sofa, (-2.56, 0.20, 0.0), (2.28, 0.95, 0.83), -90.0)
        .furniture(Kind::Ottoman, (-1.66, 0.60, 0.02), (0.90, 0.70, 0.43), -90.0)
        .furniture(Kind::Rug, (-1.30, 0.17, 0.0), (1.65, 2.94, 0.04), 0.0)
        .furniture(Kind::Plant, (-2.6, -1.00, 0.0), (0.78, 0.78, 1.9), 0.0)
        .furniture(Kind::SideTable, (-1.72, -0.70, 0.02), (0.54, 0.54, 0.55), 0.0)
        .furniture(Kind::Blossom, (-2.48, 1.47, 0.0), (0.40, 0.40, 1.4), 0.0)
        .furniture(Kind::Table, (0.35, -0.70, 0.0), (1.2, 0.8, 0.75), 90.0)
        .furniture(Kind::DiningChair, (-0.16, -0.98, 0.0), (0.44, 0.49, 0.94), -90.0)
        .furniture(Kind::DiningChair, (-0.16, -0.39, 0.0), (0.44, 0.49, 0.94), -90.0)
        .furniture(Kind::DiningChair, (0.86, -0.98, 0.0), (0.44, 0.49, 0.94), 90.0)
        .furniture(Kind::DiningChair, (0.86, -0.39, 0.0), (0.44, 0.49, 0.94), 90.0)
        .furniture(Kind::Workstation, (2.43, -0.48, 0.0), (1.417, 1.20, 1.50), 90.0)
        .furniture(Kind::Pegboard, (2.85, -1.235, 1.08), (0.38, 0.20, 0.56), 180.0)
        .furniture(Kind::Poster, (3.00, 0.58, 1.35), (0.32, 0.02, 0.46), 90.0)
        .furniture(Kind::PosterNight, (3.00, 1.00, 1.35), (0.32, 0.02, 0.46), 90.0)
        .furniture(Kind::Palm, (2.66, 0.80, 0.0), (0.90, 0.90, 1.65), 0.0)
        .furniture(Kind::SillPlant, (1.78, -1.33, 0.945), (0.24, 0.24, 0.42), 0.0)
        .furniture(Kind::Chair, (1.83, -0.43, 0.0), (0.72, 0.72, 1.3), -90.0)

        // Display wall on the storage side of the kitchen/hall junction.
        .furniture(Kind::DisplayShelf, (0.55, 2.04, 0.0), (0.76, 0.32, 1.90), 0.0)
        .furniture(Kind::WallSwords, (1.26, 2.17, 0.28), (0.42, 0.10, 1.92), 0.0)

        // Art hangs above the sofa.
        .furniture(Kind::WallArt, (-2.98, 0.40, 0.60), (2.3, 0.08, 1.6), 90.0)
}

fn kitchen() -> Room {
    Room::new(vec2(-4.2, 1.5), vec2(3.2, 3.1), Material::Floorboards)
        .walls(Walls::LEFT | Walls::TOP)
        .outline(&[(-1.6, -1.55), (1.3, -1.55), (1.3, -0.65), (1.8, -0.65), (1.8, 1.55), (-1.6, 1.55)])
        .window_width(vec2(0.3, 1.55), 180.0, 1.4)
        .furniture(Kind::Counter, (-1.275, 1.225, 0.0), (0.55, 0.55, 0.9), 0.0)
        .furniture(Kind::Counter, (1.475, 1.225, 0.0), (0.55, 0.55, 0.9), 0.0)
        .furniture(Kind::Counter, (-1.275, -0.70, 0.0), (1.10, 0.55, 0.9), -90.0)
        .furniture(Kind::Counter, (-1.275, 0.675, 0.0), (0.55, 0.55, 0.9), -90.0)
        .furniture(Kind::KitchenSink, (0.1, 1.225, 0.0), (2.2, 0.55, 0.9), 0.0)
        .furniture(Kind::Counter, (1.475, 0.4, 0.0), (1.1, 0.55, 0.9), 90.0)
        // Shallower upper cabinets, anchored against the walls behind the worktops.
        .furniture(Kind::Cupboard, (-1.425, -0.65, 1.45), (0.85, 0.30, 0.72), -90.0)
        .furniture(Kind::Cupboard, (-1.425, 0.63, 1.45), (0.85, 0.30, 0.72), -90.0)
        .furniture(Kind::Cupboard, (1.625, 0.45, 1.45), (1.0, 0.30, 0.72), 90.0)
        .furniture(Kind::Fridge, (1.475, -0.425, 0.0), (0.55, 0.55, 1.9), 90.0)
        .furniture(Kind::Oven, (-1.275, 0.125, 0.0), (0.55, 0.55, 0.9), -90.0)
        .furniture(Kind::Kettle, (1.47, 1.12, 0.905), (0.20, 0.25, 0.26), 90.0)
        .furniture(Kind::Microwave, (1.46, 0.66, 0.905), (0.48, 0.36, 0.30), 90.0)
        .furniture(Kind::CookerHood, (-1.34, 0.125, 1.47), (0.62, 0.44, 0.76), -90.0)
        .furniture(Kind::Cupboard, (-0.86, 1.395, 1.45), (0.98, 0.30, 0.72), 0.0)
        .furniture(Kind::Cupboard, (1.34, 1.395, 1.45), (0.70, 0.30, 0.72), 0.0)
        .furniture(Kind::Splashback, (-1.535, -0.02, 0.90), (2.65, 0.018, 0.55), -90.0)
        .furniture(Kind::Splashback, (0.12, 1.485, 0.90), (3.08, 0.018, 0.06), 0.0)
        .furniture(Kind::Splashback, (-0.95, 1.485, 0.96), (0.98, 0.018, 0.49), 0.0)
        .furniture(Kind::Splashback, (1.34, 1.485, 0.96), (0.68, 0.018, 0.49), 0.0)
        .furniture(Kind::Splashback, (1.735, 0.52, 0.90), (1.36, 0.018, 0.55), 90.0)
        .furniture(Kind::Blind, (0.30, 1.545, 1.00), (1.33, 0.025, 1.03), 0.0)
        .furniture(Kind::SillPlant, (0.35, 1.43, 0.96), (0.29, 0.29, 0.48), 0.0)
        .furniture(Kind::TrailingPlant, (-1.39, -0.58, 2.17), (0.40, 0.40, 0.50), 0.0)
        .furniture(Kind::TrailingPlant, (1.04, 1.32, 2.17), (0.40, 0.40, 0.62), 0.0)
}

fn storage(pos: Vec2) -> Room {
    Room::new(pos, vec2(1.5, 1.1), Material::Carpet).door(vec2(0.75, 0.0), -90.0)
}

fn bedroom() -> Room {
    Room::new(vec2(3.85, -0.95), vec2(3.9, 3.6), Material::Carpet)
        .outline(&[(-1.95, -1.8), (1.95, -1.8), (1.95, 1.8), (-0.25, 1.8), (-0.25, 0.9), (-1.95, 0.9)])
        .door(vec2(-0.25, 1.35), -90.0)
        .window(vec2(0.0, -1.8), 0.0)
        .furniture(Kind::Bed, (0.8, -0.45, 0.0), (1.4, 2.1, 0.55), 90.0)
        .furniture(Kind::Cupboard, (1.625, -1.45, 0.0), (0.4, 0.55, 0.5), 90.0)
        .furniture(Kind::Cupboard, (1.225, 1.45, 0.0), (1.35, 0.6, 2.0), 0.0)
        .furniture(Kind::Cupboard, (-1.5, 0.08, 0.0), (1.54, 0.8, 0.8), -90.0)
        .furniture(Kind::Radiator, (0.0, -1.7, 0.0), (1.4, 0.1, 0.6), 0.0)
}

fn ensuite() -> Room {
    Room::new(vec2(1.1, -1.4), vec2(1.6, 2.7), Material::Tile)
        .door(vec2(0.8, -0.85), 90.0)
        .window(vec2(0.0, -1.35), 0.0)
        .furniture(Kind::Shower, (-0.4, 0.65, 0.0), (0.7, 1.3, 2.0), 0.0)
        .furniture(Kind::Toilet, (-0.425, -0.9, 0.0), (0.55, 0.65, 0.4), -90.0)
        .furniture(Kind::Sink, (0.35, 0.075, 0.0), (0.45, 0.45, 0.85), 0.0)
        .furniture(Kind::Radiator, (0.375, -1.25, 0.0), (0.7, 0.1, 0.6), 0.0)
}

fn boiler_room() -> Room {
    Room::new(vec2(1.5, -0.55), vec2(0.8, 1.0), Material::Tile).door_width(vec2(0.0, 0.5), 180.0, 0.6).furniture(
        Kind::Appliance,
        (0.0, -0.1, 0.0),
        (0.6, 0.6, 1.6),
        0.0,
    )
}

fn spare_room() -> Room {
    Room::new(vec2(4.2, 1.95), vec2(3.2, 2.2), Material::Carpet)
        .outline(&[(-1.6, -0.9), (-0.6, -0.9), (-0.6, -1.1), (1.6, -1.1), (1.6, 1.1), (-1.6, 1.1)])
        .door(vec2(-1.1, -0.9), 180.0)
        .window(vec2(1.6, 0.0), -90.0)
        .furniture(Kind::Cupboard, (0.0, 0.75, 0.0), (3.1, 0.6, 2.0), 0.0)
        .furniture(Kind::Radiator, (1.5, 0.0, 0.0), (0.75, 0.1, 0.6), 90.0)
}

fn bathroom() -> Room {
    Room::new(vec2(1.4, 2.05), vec2(2.4, 2.0), Material::Tile)
        .door(vec2(0.7, -1.0), 180.0)
        .furniture(Kind::Shower, (0.85, 0.525, 0.0), (0.6, 0.85, 2.0), 0.0)
        .furniture(Kind::Bath, (-0.75, 0.0, 0.0), (0.8, 1.9, 0.55), 0.0)
        .furniture(Kind::Toilet, (0.075, 0.625, 0.0), (0.55, 0.65, 0.4), 0.0)
        .furniture(Kind::Sink, (0.0, -0.725, 0.0), (0.45, 0.45, 0.85), -180.0)
        .furniture(Kind::Radiator, (1.1, -0.375, 0.0), (0.7, 0.1, 0.6), 90.0)
}
