use crate::render::{
    DOOR_HEIGHT, EXTERIOR_WALL_THICKNESS, WALL_HEIGHT, WINDOW_HEAD, WINDOW_SILL, photometry::warm_white,
};
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
    Satin,
    Stone,
    Ceramic,
    Metal,
    Paint,
    Foliage,
    VariegatedFoliage,
    MarbledFoliage,
    Soil,
    Floorboards,
    Shag,
    Knit,
    Emissive,
    Mirror,
    Maple,
    MapleHorizontal,
    GlazedTile,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
#[repr(u32)]
pub enum Kind {
    #[default]
    Box,
    Bed,
    Sofa,
    Pillow,
    Table,
    Counter,
    Ceiling,
    DrawerUnit,
    KitchenPanel,
    KnifeBlock,
    AirFryer,
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
    Downlight,
    Lamp,
}

/// 2700 K Planckian white, linear sRGB normalized to photometric Y=1.
pub const DEFAULT_LIGHT_COLOR: Vec3 = Vec3::new(1.929_37, 0.805_206_9, 0.192_963_1);

/// One representation for architecture and procedural furniture.
#[derive(Clone, Copy, ShaderData)]
pub struct Object {
    pub pos: Vec3,
    pub size: Vec3,
    /// Precomputed sine and cosine around z.
    pub rotation: Vec2,
    pub kind: Kind,
    pub material: Material,
    /// Base of automatically projected surface irradiance samples, or MAX.
    pub surface_cache: u32,
    /// Preview light output; zero for objects that do not emit light.
    pub emission: f32,
    /// Linear RGB source colour, separate from the fixture's material.
    pub emission_color: Vec3,
    /// Outward normal for exterior architecture; zero for interior objects.
    pub exterior: Vec3,
}
impl Object {
    /// An unrotated, non-emissive object centred at `pos`.
    pub const fn new(kind: Kind, pos: Vec3, size: Vec3, material: Material) -> Self {
        Self {
            pos,
            size,
            rotation: Vec2::Y,
            kind,
            material,
            surface_cache: u32::MAX,
            emission: 0.0,
            emission_color: DEFAULT_LIGHT_COLOR,
            exterior: Vec3::ZERO,
        }
    }

    /// Ceiling downlights and table lamps, bound to Home Assistant entities.
    pub const fn is_light(self) -> bool {
        matches!(self.kind, Kind::Downlight | Kind::Lamp)
    }

    pub fn extent(self) -> Vec3 {
        match self.kind {
            Kind::Box | Kind::Ceiling | Kind::Downlight | Kind::Lamp => self.size * 0.5,
            Kind::KitchenSink => self.size * 0.5 + Vec3::new(0.06, 0.06, 0.32),
            Kind::Rug => self.size * 0.5 + Vec3::new(0.075, 0.075, 0.050),
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

/// A fixture and its future Home Assistant binding. Shared IDs control a group.
pub struct Light {
    pub entity_id: &'static str,
    pub fixture: Object,
}

pub struct Room {
    pub position: Vec2,
    pub outline: Vec<Vec2>,
    pub walls: Walls,
    pub floor: Material,
    pub furniture: Vec<Object>,
    pub openings: Vec<Opening>,
    pub lights: Vec<Light>,
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
            lights: vec![],
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
        self.openings.push(Opening { pos: pos.extend(WALL_HEIGHT * 0.5), size: size.extend(WALL_HEIGHT * 2.0) });
        self
    }

    pub fn door(self, pos: Vec2, rotation: f32) -> Self {
        self.door_width(pos, rotation, 0.8)
    }

    pub fn window(self, pos: Vec2, rotation: f32) -> Self {
        self.window_width(pos, rotation, 1.2)
    }

    pub fn door_width(self, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.opening(pos, rotation, width, 0.0, DOOR_HEIGHT)
    }

    pub fn window_width(self, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.window_frame(pos, rotation, width, WINDOW_SILL, WINDOW_HEAD)
    }

    pub fn window_frame(self, pos: Vec2, rotation: f32, width: f32, sill: f32, head: f32) -> Self {
        self.opening(pos, rotation, width, sill, head).furniture(
            Kind::Window,
            pos.extend(sill),
            (width, EXTERIOR_WALL_THICKNESS, head - sill),
            rotation,
        )
    }

    fn opening(mut self, pos: Vec2, rotation: f32, width: f32, bottom: f32, top: f32) -> Self {
        let (sin, cos) = rotation.to_radians().sin_cos();
        let depth = EXTERIOR_WALL_THICKNESS * 2.0;
        self.openings.push(Opening {
            pos: pos.extend(f32::midpoint(bottom, top)),
            size: Vec3::new(cos.abs() * width + sin.abs() * depth, sin.abs() * width + cos.abs() * depth, top - bottom),
        });
        self
    }

    /// Room-local ceiling fixtures, all driven by the same entity. Positions are
    /// explicit so the same helper works for rectangular and irregular rooms.
    pub fn downlights(self, entity_id: &'static str, positions: &[(f32, f32)]) -> Self {
        positions.iter().fold(self, |room, &(x, y)| {
            let pos = Vec3::new(x, y, WALL_HEIGHT - 0.032);
            room.light(entity_id, Object {
                emission_color: warm_white(4000.0),
                ..Object::new(Kind::Downlight, pos, Vec3::new(0.11, 0.11, 0.032), Material::Paint)
            })
        })
    }

    /// A table lamp resting on a surface at the supplied room-local height.
    pub fn lamp(self, entity_id: &'static str, position: impl Into<Vec3>) -> Self {
        self.light(entity_id, Object::new(Kind::Lamp, position.into(), Vec3::new(0.20, 0.20, 0.40), Material::Metal))
    }

    fn light(mut self, entity_id: &'static str, fixture: Object) -> Self {
        self.lights.push(Light { entity_id, fixture: Object { emission: 1.0, ..fixture } });
        self
    }

    pub fn furniture(self, kind: Kind, pos: impl Into<Vec3>, size: impl Into<Vec3>, rotation: f32) -> Self {
        self.furniture_material(kind, pos, size, rotation, Material::Wood)
    }

    pub fn furniture_material(
        mut self,
        kind: Kind,
        pos: impl Into<Vec3>,
        size: impl Into<Vec3>,
        rotation: f32,
        material: Material,
    ) -> Self {
        let (sin, cos) = rotation.to_radians().sin_cos();
        self.furniture
            .push(Object { rotation: vec2(sin, cos), ..Object::new(kind, pos.into(), size.into(), material) });
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
        .downlights("light.hall_downlights", &[(-1.675, 0.0), (0.0, 0.0), (1.675, 0.0), (-1.7, 1.55)])
        .outline(&[(-2.25, -0.55), (2.25, -0.55), (2.25, 0.55), (-1.15, 0.55), (-1.15, 2.55), (-2.25, 2.55)])
        .door(vec2(-1.7, 2.55), 0.0)
        .furniture(Kind::EntryDoor, (-1.7, 2.55, 0.0), (0.79, 0.045, 2.04), 0.0)
        .furniture(Kind::Radiator, (-0.425, 0.45, 0.0), (1.2, 0.1, 0.6), 0.0)
        .furniture(Kind::ShoeStorage, (-1.30, 1.47, 0.0), (0.76, 0.22, 1.02), 90.0)
        .furniture(Kind::SillPlant, (-1.30, 1.64, 1.025), (0.53, 0.53, 0.76), 0.0)
        .furniture(Kind::PosterBlue, (-1.215, 1.38, 1.36), (0.33, 0.02, 0.54), 90.0)
        .furniture(Kind::Mirror, (-1.215, 0.77, 0.61), (0.40, 0.035, 1.41), 90.0)
}

fn lounge() -> Room {
    Room::new(vec2(-2.75, -1.4), vec2(6.1, 2.7), Material::Floorboards)
        .walls(Walls::LEFT | Walls::BOTTOM)
        .downlights("light.lounge_left_downlights", &[(0.70, -0.65), (2.10, -0.65), (0.70, 0.65), (2.10, 0.65)])
        .downlights("light.lounge_right_downlights", &[(-2.10, -0.65), (-0.70, -0.65), (-2.10, 0.65), (-0.70, 0.65)])
        .cut_wall(vec2(-2.2, 1.35), vec2(1.6, 0.3))
        .cut_wall(vec2(-0.15, 1.8), vec2(0.2, 1.0))
        .outline(&[
            (-3.05, -1.35), (3.05, -1.35), (3.05, 1.35), (1.85, 1.35),
            (1.85, 2.25), (-0.15, 2.25), (-0.15, 1.35), (-3.05, 1.35),
        ])
        .window_width(vec2(-1.15, -1.35), 0.0, 1.4)
        .window_width(vec2(1.75, -1.35), 0.0, 0.70)
        .furniture(Kind::Radiator, (-1.15, -1.25, 0.0), (1.4, 0.1, 0.6), 0.0)
        .furniture(Kind::Curtain, (-1.15, -1.25, 0.67), (1.80, 0.38, 1.63), 0.0)
        .furniture(Kind::Curtain, (1.75, -1.25, 0.67), (1.10, 0.38, 1.63), 0.0)
        .furniture(Kind::SillArrangement, (-1.40, -1.33, 0.945), (0.24, 0.20, 0.64), 0.0)
        .furniture(Kind::Sofa, (-2.56, 0.20, 0.0), (2.28, 0.95, 0.83), -90.0)
        .furniture_material(Kind::Pillow, (-2.65, -0.47, 0.45), (0.39, 0.17, 0.39), -98.0, Material::Satin)
        .furniture_material(Kind::Pillow, (-2.65, 0.87, 0.45), (0.40, 0.18, 0.39), -78.0, Material::Knit)
        .furniture(Kind::Ottoman, (-1.66, 0.60, 0.02), (0.90, 0.70, 0.43), -90.0)
        .furniture(Kind::Rug, (-1.50, 0.17, 0.0), (1.65, 2.94, 0.012), 0.0)
        .furniture(Kind::Plant, (-2.6, -1.00, 0.0), (1.22, 1.10, 1.9), 0.0)
        .furniture(Kind::SideTable, (-1.72, -0.70, 0.02), (0.54, 0.54, 0.55), 0.0)
        .lamp("light.lounge_table_lamp", (-1.82, -0.785, 0.57))
        .furniture(Kind::Blossom, (-2.65, 1.50, 0.0), (0.72, 0.60, 1.30), 0.0)
        .furniture(Kind::Table, (0.35, -0.70, 0.0), (1.2, 0.8, 0.75), 90.0)
        .furniture(Kind::DiningChair, (-0.16, -0.98, 0.0), (0.44, 0.49, 0.94), -90.0)
        .furniture(Kind::DiningChair, (-0.16, -0.39, 0.0), (0.44, 0.49, 0.94), -90.0)
        .furniture(Kind::DiningChair, (0.86, -0.98, 0.0), (0.44, 0.49, 0.94), 90.0)
        .furniture(Kind::DiningChair, (0.86, -0.39, 0.0), (0.44, 0.49, 0.94), 90.0)
        .furniture(Kind::Workstation, (2.43, -0.48, 0.0), (1.417, 1.20, 1.50), 90.0)
        .furniture(Kind::Pegboard, (2.58, -1.235, 1.08), (0.76, 0.20, 0.56), 180.0)
        .furniture(Kind::Poster, (3.00, 0.58, 1.35), (0.32, 0.02, 0.46), 90.0)
        .furniture(Kind::PosterNight, (3.00, 1.00, 1.35), (0.32, 0.02, 0.46), 90.0)
        .furniture(Kind::Palm, (2.61, 0.50, 0.0), (1.30, 1.30, 1.15), 0.0)
        .furniture(Kind::SillPlant, (1.78, -1.33, 0.945), (0.24, 0.24, 0.42), 0.0)
        .furniture(Kind::Chair, (1.83, -0.43, 0.0), (0.72, 0.72, 1.3), 115.0)

        // Display wall on the storage side of the kitchen/hall junction.
        .furniture(Kind::DisplayShelf, (0.85, 2.04, 0.0), (0.64, 0.32, 1.90), 0.0)
        .furniture(Kind::WallSwords, (1.51, 2.17, 0.28), (0.42, 0.10, 1.92), 0.0)

        // Art hangs above the sofa.
        .furniture(Kind::WallArt, (-2.98, 0.40, 0.60), (2.3, 0.08, 1.6), 90.0)
}

fn kitchen() -> Room {
    Room::new(vec2(-4.2, 1.5), vec2(3.2, 3.1), Material::Floorboards)
        .walls(Walls::LEFT | Walls::RIGHT | Walls::TOP)
        .downlights("light.kitchen_downlights", &[(-0.5, 0.0), (0.7, 0.0)])
        .cut_wall(vec2(1.3, -1.10), vec2(0.3, 0.95))
        .outline(&[(-1.6, -1.55), (1.3, -1.55), (1.3, -0.65), (1.8, -0.65), (1.8, 1.55), (-1.6, 1.55)])
        .window_frame(vec2(0.3, 1.55), 180.0, 1.4, 1.02, WINDOW_HEAD)
        .furniture(Kind::Counter, (-1.275, 1.225, 0.0), (0.55, 0.55, 0.9), 0.0)
        .furniture(Kind::Counter, (1.475, 1.225, 0.0), (0.55, 0.55, 0.9), 0.0)
        .furniture(Kind::Counter, (-1.275, -0.975, 0.0), (0.55, 0.55, 0.9), -90.0)
        .furniture(Kind::DrawerUnit, (-1.275, -0.425, 0.0), (0.55, 0.55, 0.9), -90.0)
        .furniture(Kind::Counter, (-1.275, 0.675, 0.0), (0.55, 0.55, 0.9), -90.0)
        .furniture(Kind::KitchenSink, (0.1, 1.225, 0.0), (2.2, 0.55, 0.9), 0.0)
        .furniture(Kind::DrawerUnit, (1.475, 0.525, 0.0), (0.95, 0.55, 0.9), 90.0)
        .furniture(Kind::KitchenPanel, (-1.08, -0.105, 0.0), (0.025, 2.295, 0.14), 0.0)
        .furniture(Kind::KitchenPanel, (0.10, 1.03, 0.0), (2.385, 0.025, 0.14), 0.0)
        .furniture(Kind::KitchenPanel, (1.28, 0.1975, 0.0), (0.025, 1.69, 0.14), 0.0)
        .furniture(Kind::KitchenPanel, (-1.315, -1.24, 0.0), (0.495, 0.025, 0.14), 0.0)
        .furniture(Kind::KitchenPanel, (0.10, 1.395, 2.148), (3.30, 0.345, 0.044), 0.0)
        // Continuous cornice spans the extractor opening and joins the window run.
        .furniture(Kind::KitchenPanel, (-1.425, -0.0025, 2.148), (2.495, 0.345, 0.044), -90.0)
        .furniture(Kind::KitchenPanel, (1.625, 0.6525, 2.148), (1.185, 0.345, 0.044), 90.0)
        .furniture(Kind::KnifeBlock, (1.46, 0.13, 0.905), (0.18, 0.15, 0.34), 90.0)
        // Shallower upper cabinets, anchored against the walls behind the worktops.
        .furniture(Kind::Cupboard, (-1.425, -0.70, 1.45), (1.10, 0.30, 0.72), -90.0)
        .furniture(Kind::Cupboard, (-1.425, 0.855, 1.45), (0.78, 0.30, 0.72), -90.0)
        .furniture(Kind::Cupboard, (1.625, 0.6525, 1.45), (1.185, 0.30, 0.72), 90.0)
        .furniture(Kind::Fridge, (1.475, -0.32, 0.0), (0.65, 0.55, 2.32), 90.0)
        .furniture(Kind::Oven, (-1.275, 0.125, 0.0), (0.55, 0.55, 0.9), -90.0)
        .furniture(Kind::Kettle, (1.47, 0.46, 0.905), (0.20, 0.25, 0.26), 90.0)
        .furniture(Kind::Microwave, (1.39, 1.13, 0.905), (0.48, 0.36, 0.30), 45.0)
        .furniture(Kind::CookerHood, (-1.34, 0.155, 1.43), (0.56, 0.44, WALL_HEIGHT - 1.43), -90.0)
        .furniture(Kind::Cupboard, (-0.975, 1.395, 1.45), (1.15, 0.30, 0.72), 0.0)
        .furniture(Kind::Cupboard, (1.375, 1.395, 1.45), (0.75, 0.30, 0.72), 0.0)
        .furniture(Kind::Splashback, (-1.535, 0.15, 0.90), (2.70, 0.018, 0.55), -90.0)
        .furniture(Kind::Splashback, (0.10, 1.485, 0.90), (3.30, 0.018, 0.15), 0.0)
        .furniture(Kind::Splashback, (-1.00, 1.485, 0.90), (1.20, 0.018, 0.55), 0.0)
        .furniture(Kind::Splashback, (1.40, 1.485, 0.90), (0.80, 0.018, 0.55), 0.0)
        .furniture(Kind::Splashback, (1.735, 0.67, 0.90), (1.66, 0.018, 0.55), 90.0)
        .furniture(Kind::SillPlant, (0.35, 1.43, 1.05), (0.29, 0.29, 0.48), 0.0)
        .furniture(Kind::TrailingPlant, (-1.23, -0.075, 1.80), (0.30, 0.30, 0.42), -90.0)
        .furniture(Kind::TrailingPlant, (-1.23, 0.385, 1.83), (0.24, 0.24, 0.39), -90.0)
        .furniture(Kind::TrailingPlant, (1.07, 1.23, 1.608), (0.34, 0.34, 0.54), 0.0)
        .furniture(Kind::AirFryer, (-1.27, -0.75, 0.905), (0.36, 0.38, 0.42), -90.0)
}

fn storage(pos: Vec2) -> Room {
    Room::new(pos, vec2(1.5, 1.1), Material::Carpet).door(vec2(0.75, 0.0), -90.0)
}

fn bedroom() -> Room {
    Room::new(vec2(3.85, -0.95), vec2(3.9, 3.6), Material::Carpet)
        .downlights("light.bedroom_downlights", &[(-0.95, -0.95), (0.95, -0.95), (-0.95, 0.15), (0.95, 0.15)])
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
        .downlights("light.ensuite_downlight", &[(0.0, -0.4)])
        .downlights("light.ensuite_shower_downlight", &[(-0.4, 0.7)])
        .door(vec2(0.8, -0.85), 90.0)
        .window(vec2(0.0, -1.35), 0.0)
        .furniture(Kind::Shower, (-0.4, 0.65, 0.0), (0.7, 1.3, 2.0), 0.0)
        .furniture(Kind::Toilet, (-0.425, -0.9, 0.0), (0.40, 0.65, 0.80), -90.0)
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
        .downlights("light.office_downlights", &[(-0.725, -0.1), (0.725, -0.1)])
        .outline(&[(-1.6, -0.9), (-0.6, -0.9), (-0.6, -1.1), (1.6, -1.1), (1.6, 1.1), (-1.6, 1.1)])
        .door(vec2(-1.1, -0.9), 180.0)
        .window(vec2(1.6, 0.0), -90.0)
        .furniture(Kind::Cupboard, (0.0, 0.75, 0.0), (3.1, 0.6, 2.0), 0.0)
        .furniture(Kind::Radiator, (1.5, 0.0, 0.0), (0.75, 0.1, 0.6), 90.0)
}

fn bathroom() -> Room {
    Room::new(vec2(1.4, 2.05), vec2(2.4, 2.0), Material::Tile)
        .downlights("light.bathroom_downlight", &[(0.0, 0.0)])
        .downlights("light.bathroom_shower_downlight", &[(0.85, 0.55)])
        .door(vec2(0.7, -1.0), 180.0)
        .furniture(Kind::Shower, (0.85, 0.525, 0.0), (0.6, 0.85, 2.0), 0.0)
        .furniture(Kind::Bath, (-0.75, 0.0, 0.0), (0.8, 1.9, 0.55), 0.0)
        .furniture(Kind::Toilet, (0.075, 0.625, 0.0), (0.40, 0.65, 0.80), 0.0)
        .furniture(Kind::Sink, (0.0, -0.725, 0.0), (0.45, 0.45, 0.85), -180.0)
        .furniture(Kind::Radiator, (1.1, -0.375, 0.0), (0.7, 0.1, 0.6), 90.0)
}
