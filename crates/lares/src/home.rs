use isthmus::{
    InlineVec, ShaderData,
    glam::{Vec2, Vec3},
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
}

#[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
#[repr(u32)]
pub enum FurnitureKind {
    #[default]
    Bed,
    Sofa,
    Table,
    Counter,
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
    WallArt,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
#[repr(u32)]
pub enum Action {
    #[default]
    Add,
    Subtract,
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct Operation {
    pub action: Action,
    pub pos: Vec2,
    pub size: Vec2,
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct Furniture {
    pub kind: FurnitureKind,
    /// Room-local position with z as base elevation.
    pub pos: Vec3,
    /// Footprint in x/y and height in z.
    pub size: Vec3,
    /// Rotation around vertical z in radians.
    pub rotation: f32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
#[repr(u32)]
pub enum OpeningKind {
    #[default]
    Door,
    Window,
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct Opening {
    pub kind: OpeningKind,
    pub pos: Vec2,
    pub width: f32,
    pub rotation: f32,
}

const MAX_OPERATIONS: usize = 2;
const MAX_FURNITURE: usize = 10;
const MAX_OPENINGS: usize = 2;

#[derive(Clone, Copy, Default, ShaderData)]
pub struct Room {
    pub position: Vec2,
    pub size: Vec2,
    pub walls: Walls,
    pub floor: Material,
    pub operations: InlineVec<Operation, MAX_OPERATIONS>,
    pub furniture: InlineVec<Furniture, MAX_FURNITURE>,
    pub openings: InlineVec<Opening, MAX_OPENINGS>,
}

#[derive(Clone, Copy, ShaderData)]
pub struct Bounds {
    pub min: Vec2,
    pub max: Vec2,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Default, PartialEq, Eq, ShaderData)]
    #[shader_data(bitflags)]
    pub struct Walls: u32 {
        const LEFT = 1;
        const TOP = 2;
        const RIGHT = 4;
        const BOTTOM = 8;
    }
}

impl Room {
    pub const fn new(pos: Vec2, size: Vec2, floor: Material) -> Self {
        Self {
            position: pos,
            size,
            walls: Walls::all(),
            floor,
            operations: InlineVec::new(),
            openings: InlineVec::new(),
            furniture: InlineVec::new(),
        }
    }

    pub const fn walls(mut self, walls: Walls) -> Self {
        self.walls = walls;
        self
    }

    pub const fn add(mut self, pos: Vec2, size: Vec2) -> Self {
        self.operations.push(Operation { action: Action::Add, pos, size });
        self
    }

    pub const fn subtract(mut self, pos: Vec2, size: Vec2) -> Self {
        self.operations.push(Operation { action: Action::Subtract, pos, size });
        self
    }

    pub const fn door(self, pos: Vec2, rotation: f32) -> Self {
        self.opening(OpeningKind::Door, pos, rotation, 0.8)
    }

    pub const fn door_width(self, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.opening(OpeningKind::Door, pos, rotation, width)
    }

    pub const fn window(self, pos: Vec2, rotation: f32) -> Self {
        self.opening(OpeningKind::Window, pos, rotation, 1.2)
    }

    pub const fn window_width(self, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.opening(OpeningKind::Window, pos, rotation, width)
    }

    const fn opening(mut self, kind: OpeningKind, pos: Vec2, rotation: f32, width: f32) -> Self {
        self.openings.push(Opening { kind, pos, width, rotation: rotation.to_radians() });
        self
    }

    pub const fn furniture(mut self, kind: FurnitureKind, pos: Vec3, size: Vec3, rotation: f32) -> Self {
        self.furniture.push(Furniture { kind, pos, size, rotation: rotation.to_radians() });
        self
    }
}

pub struct Home {
    pub rooms: Box<[Room]>,
    pub bounds: Box<[Bounds]>,
    pub radius: f32,
}

impl Home {
    pub fn template() -> Self {
        let builders = template::rooms();
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for room in &builders {
            min = min.min(room.position - room.size * 0.5);
            max = max.max(room.position + room.size * 0.5);
        }
        let origin = (min + max) * 0.5;
        let mut rooms = Vec::with_capacity(builders.len());
        let mut bounds = Vec::with_capacity(builders.len());
        let mut radius: f32 = 0.0;
        for mut room in builders {
            let position = room.position - origin;
            room.position = Vec2::new(position.x, -position.y);
            let mut lower = -room.size * 0.5;
            let mut upper = room.size * 0.5;
            for operation in room.operations.as_slice() {
                if operation.action == Action::Add {
                    let center = Vec2::new(operation.pos.x, -operation.pos.y);
                    lower = lower.min(center - operation.size * 0.5);
                    upper = upper.max(center + operation.size * 0.5);
                }
            }
            for item in room.furniture.as_slice() {
                let center = Vec2::new(item.pos.x, -item.pos.y);
                let half = item.size.truncate() * 0.5;
                let (sin, cos) = item.rotation.sin_cos();
                let extent =
                    Vec2::new(cos.abs() * half.x + sin.abs() * half.y, sin.abs() * half.x + cos.abs() * half.y);
                lower = lower.min(center - extent);
                upper = upper.max(center + extent);
            }
            let lower = room.position + lower - Vec2::splat(0.1);
            let upper = room.position + upper + Vec2::splat(0.1);
            radius = radius.max(lower.abs().max(upper.abs()).length());
            bounds.push(Bounds { min: lower, max: upper });
            rooms.push(room);
        }
        Self { rooms: rooms.into_boxed_slice(), bounds: bounds.into_boxed_slice(), radius }
    }
}

mod template {
    use super::{FurnitureKind, Material, Room, Walls};
    use isthmus::glam::{Vec2, Vec3, vec2};

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
    const fn hall() -> Room {
        Room::new(vec2(1.35, 0.5), vec2(4.5, 1.1), Material::Wood)
            .walls(Walls::TOP)
            .add(vec2(-1.7, 1.55), vec2(1.1, 2.0))
            .door(vec2(-1.7, 2.55), 0.0)
            .furniture(FurnitureKind::Radiator, Vec3::new(-0.425, 0.45, 0.0), Vec3::new(1.2, 0.1, 0.6), 0.0)
    }

    fn lounge() -> Room {
        Room::new(vec2(-2.75, -1.4), vec2(6.1, 2.7), Material::Wood)
            .walls(Walls::LEFT | Walls::BOTTOM)
            .subtract(vec2(-2.2, 1.35), vec2(1.6, 0.3))
            .add(vec2(0.85, 1.8), vec2(2.0, 0.9))
            .window_width(vec2(-1.15, -1.35), 0.0, 1.4)
            .window(vec2(1.75, -1.35), 0.0)
            .furniture(
                FurnitureKind::Radiator,
                Vec3::new(-1.15, -1.25, 0.0),
                Vec3::new(1.4, 0.1, 0.6),
                0.0,
            )
            .furniture(
                FurnitureKind::Sofa,
                Vec3::new(-2.525, 0.0, 0.0),
                Vec3::new(2.3, 0.95, 0.85),
                -90.0,
            )
            .furniture(
                FurnitureKind::Ottoman,
                Vec3::new(-1.75, 0.5, 0.0),
                Vec3::new(0.8, 0.6, 0.45),
                -90.0,
            )
            .furniture(FurnitureKind::Rug, Vec3::new(-2.2, 0.0, 0.0), Vec3::new(2.6, 2.0, 0.05), 0.0)
            .furniture(
                FurnitureKind::Plant,
                Vec3::new(-2.6, -1.0, 0.0),
                Vec3::new(0.95, 0.95, 1.9),
                0.0,
            )
            .furniture(FurnitureKind::Table, Vec3::new(0.35, -0.9, 0.0), Vec3::new(1.2, 0.8, 0.75), 0.0)
            .furniture(
                FurnitureKind::Workstation,
                Vec3::new(2.55, -0.475, 0.0),
                Vec3::new(1.7, 0.9, 1.4),
                -90.0,
            )
            .furniture(
                FurnitureKind::Chair,
                Vec3::new(2.05, -0.475, 0.0),
                Vec3::new(0.72, 0.72, 1.3),
                -90.0,
            )
            .furniture(
                FurnitureKind::Appliance,
                Vec3::new(2.7, 0.35, 0.0),
                Vec3::new(0.3, 0.3, 0.9),
                0.0,
            )

            // Art hangs above the sofa.
            .furniture(
                FurnitureKind::WallArt,
                Vec3::new(-2.85, 1.4, 1.15),
                Vec3::new(1.9, 0.9, 0.9),
                90.0,
            )
    }

    fn kitchen() -> Room {
        Room::new(vec2(-4.2, 1.5), vec2(3.2, 3.1), Material::Wood)
            .walls(Walls::LEFT | Walls::TOP)
            .add(vec2(1.65, 0.45), vec2(0.3, 2.2))
            .subtract(vec2(1.55, -1.15), vec2(0.5, 1.0))
            .window_width(vec2(0.3, 1.55), 0.0, 1.4)
            .furniture(FurnitureKind::Counter, Vec3::new(-1.275, 1.225, 0.0), Vec3::new(0.55, 0.55, 0.9), 0.0)
            .furniture(FurnitureKind::Counter, Vec3::new(1.475, 1.225, 0.0), Vec3::new(0.55, 0.55, 0.9), 0.0)
            .furniture(FurnitureKind::Counter, Vec3::new(-1.275, -0.15, 0.0), Vec3::new(2.2, 0.55, 0.9), -90.0)
            .furniture(FurnitureKind::Counter, Vec3::new(0.1, 1.225, 0.0), Vec3::new(2.2, 0.55, 0.9), 0.0)
            .furniture(FurnitureKind::Counter, Vec3::new(1.475, 0.4, 0.0), Vec3::new(1.1, 0.55, 0.9), 90.0)
            .furniture(FurnitureKind::Fridge, Vec3::new(1.475, -0.425, 0.0), Vec3::new(0.55, 0.55, 1.9), 90.0)
            .furniture(FurnitureKind::Oven, Vec3::new(-1.275, 0.125, 0.0), Vec3::new(0.55, 0.55, 0.9), -90.0)
            .furniture(FurnitureKind::Counter, Vec3::new(-1.275, 0.125, 0.9), Vec3::new(0.45, 0.45, 0.02), -90.0)
            .furniture(FurnitureKind::Sink, Vec3::new(0.2, 1.2, 0.88), Vec3::new(0.65, 0.5, 0.02), 0.0)
            .furniture(FurnitureKind::Appliance, Vec3::new(1.4, 1.15, 0.9), Vec3::new(0.5, 0.4, 0.3), 45.0)
    }

    const fn storage(pos: Vec2) -> Room {
        Room::new(pos, vec2(1.5, 1.1), Material::Carpet).door(vec2(0.75, 0.0), -90.0)
    }

    const fn bedroom() -> Room {
        Room::new(vec2(3.85, -0.95), vec2(3.9, 3.6), Material::Carpet)
            .subtract(vec2(-1.1, 1.4), vec2(1.7, 1.0))
            .door(vec2(-0.25, 1.35), -90.0)
            .window(vec2(0.0, -1.8), 0.0)
            .furniture(FurnitureKind::Bed, Vec3::new(0.8, -0.45, 0.0), Vec3::new(1.4, 2.1, 0.55), 90.0)
            .furniture(FurnitureKind::Cupboard, Vec3::new(1.625, -1.45, 0.0), Vec3::new(0.4, 0.55, 0.5), 90.0)
            .furniture(FurnitureKind::Cupboard, Vec3::new(1.225, 1.45, 0.0), Vec3::new(1.35, 0.6, 2.0), 0.0)
            .furniture(FurnitureKind::Cupboard, Vec3::new(-1.5, 0.08, 0.0), Vec3::new(1.54, 0.8, 0.8), -90.0)
            .furniture(FurnitureKind::Radiator, Vec3::new(0.0, -1.7, 0.0), Vec3::new(1.4, 0.1, 0.6), 0.0)
    }

    const fn ensuite() -> Room {
        Room::new(vec2(1.1, -1.4), vec2(1.6, 2.7), Material::Tile)
            .door(vec2(0.8, -0.85), 90.0)
            .window(vec2(0.0, -1.35), 0.0)
            .furniture(FurnitureKind::Shower, Vec3::new(-0.4, 0.65, 0.0), Vec3::new(0.7, 1.3, 2.0), 0.0)
            .furniture(FurnitureKind::Toilet, Vec3::new(-0.425, -0.9, 0.0), Vec3::new(0.55, 0.65, 0.4), -90.0)
            .furniture(FurnitureKind::Sink, Vec3::new(0.35, 0.075, 0.0), Vec3::new(0.45, 0.45, 0.85), 0.0)
            .furniture(FurnitureKind::Radiator, Vec3::new(0.375, -1.25, 0.0), Vec3::new(0.7, 0.1, 0.6), 0.0)
    }

    const fn boiler_room() -> Room {
        Room::new(vec2(1.5, -0.55), vec2(0.8, 1.0), Material::Tile).door_width(vec2(0.0, 0.5), 180.0, 0.6).furniture(
            FurnitureKind::Appliance,
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(0.6, 0.6, 1.6),
            0.0,
        )
    }

    const fn spare_room() -> Room {
        Room::new(vec2(4.2, 1.95), vec2(3.2, 2.2), Material::Carpet)
            .subtract(vec2(-1.1, -1.4), vec2(1.0, 1.0))
            .door(vec2(-1.1, -0.9), 180.0)
            .window(vec2(1.6, 0.0), -90.0)
            .furniture(FurnitureKind::Cupboard, Vec3::new(0.0, 0.75, 0.0), Vec3::new(3.1, 0.6, 2.0), 0.0)
            .furniture(FurnitureKind::Radiator, Vec3::new(1.5, 0.0, 0.0), Vec3::new(0.75, 0.1, 0.6), 90.0)
    }

    const fn bathroom() -> Room {
        Room::new(vec2(1.4, 2.05), vec2(2.4, 2.0), Material::Tile)
            .door(vec2(0.7, -1.0), 180.0)
            .furniture(FurnitureKind::Shower, Vec3::new(0.85, 0.525, 0.0), Vec3::new(0.6, 0.85, 2.0), 0.0)
            .furniture(FurnitureKind::Bath, Vec3::new(-0.75, 0.0, 0.0), Vec3::new(0.8, 1.9, 0.55), 0.0)
            .furniture(FurnitureKind::Toilet, Vec3::new(0.075, 0.625, 0.0), Vec3::new(0.55, 0.65, 0.4), 0.0)
            .furniture(FurnitureKind::Sink, Vec3::new(0.0, -0.725, 0.0), Vec3::new(0.45, 0.45, 0.85), -180.0)
            .furniture(FurnitureKind::Radiator, Vec3::new(1.1, -0.375, 0.0), Vec3::new(0.7, 0.1, 0.6), 90.0)
    }
}
