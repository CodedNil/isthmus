//! Furniture that slides between its authored pose and a second one, such as a chair pulled up to a desk.
use crate::{
    home::{Motion, Object, Trigger},
    presence::Presence,
};
use core::f32::consts::{PI, TAU};
use isthmus::glam::{Vec2, Vec3, vec2, vec3};
use std::sync::Arc;

/// The most objects that can move at once, since the shaders find them through a fixed set of slots.
pub const MOVERS: usize = 4;

/// What a moving object's trigger can look at.
pub struct Inputs<'presence> {
    pub presence: &'presence Presence,
    /// Where the object would end up, on the floor plan.
    pub destination: Vec2,
}

impl Inputs<'_> {
    /// Whether someone stands within `radius` metres of where the object would end up.
    pub fn occupied(&self, radius: f32) -> bool {
        self.presence.occupied(self.destination, radius)
    }
}

/// One moving object, and the two poses it eases between.
pub struct Mover {
    pub entity_id: &'static str,
    index: usize,
    rest: (Vec3, f32),
    moved: (Vec3, f32),
    trigger: Trigger,
    delay: f32,
    duration: f32,
}

impl Mover {
    /// Binds a room's motion to the object compiled for it; offsets are authored in room axes, where Y is flipped.
    pub fn new(index: usize, object: &Object, motion: &Motion) -> Self {
        let offset = vec3(motion.offset.x, -motion.offset.y, motion.offset.z);
        Self {
            entity_id: motion.entity_id,
            index,
            rest: (object.pos, object.rotation.x.atan2(object.rotation.y)),
            moved: (object.pos + offset, motion.rotation.to_radians()),
            trigger: Arc::clone(&motion.trigger),
            delay: motion.delay,
            duration: motion.duration,
        }
    }

    pub const fn index(&self) -> u32 {
        self.index as u32
    }

    /// Where the object ends up, on the floor plan.
    pub fn destination(&self) -> Vec2 {
        self.moved.0.truncate()
    }

    /// Whether the object should be at its destination right now.
    pub fn wanted(&self, presence: &Presence) -> bool {
        (self.trigger)(&Inputs { presence, destination: self.destination() })
    }

    /// Seconds the trigger must hold before the object moves.
    pub const fn delay(&self) -> f32 {
        self.delay
    }

    /// Seconds the slide takes.
    pub const fn duration(&self) -> f32 {
        self.duration
    }

    /// Places the object `progress` of the way from its authored pose to the moved one, easing at both ends,
    /// and returns whether that changed anything.
    pub fn apply(&self, objects: &mut [Object], progress: f32) -> bool {
        let t = progress.clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        // The shortest way round, so nothing spins the long way.
        let turn = (self.moved.1 - self.rest.1 + PI).rem_euclid(TAU) - PI;
        let (sin, cos) = (self.rest.1 + turn * eased).sin_cos();
        let (pos, rotation) = (self.rest.0.lerp(self.moved.0, eased), vec2(sin, cos));
        let object = &mut objects[self.index];
        let changed = object.pos != pos || object.rotation != rotation;
        (object.pos, object.rotation) = (pos, rotation);
        changed
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        home::{Kind, Material, Room},
        render::{geometry::acceleration::Scene, light::sun::SunSettings},
    };
    use isthmus::glam::{Vec2, Vec3, vec2, vec3};

    fn scene() -> Scene {
        let room = Room::new(Vec2::ZERO, Vec2::splat(4.0), Material::Paint)
            .furniture(Kind::Chair, (0.0, 0.5, 0.0), (0.7, 0.7, 1.3), 170.0)
            .moves("chair", (0.4, -0.2, 0.0), -170.0, |_| false);
        Scene::new_at(&[room], 1_782_043_200.0, SunSettings::default())
    }

    #[test]
    fn objects_slide_to_their_moved_pose_by_name_and_back() {
        let mut scene = scene();
        let chair = scene.movers()[0].index() as usize;
        let rest = scene.objects[chair];
        assert!(!scene.set_motion("missing", 1.0));
        assert!(scene.set_motion("chair", 1.0));
        let moved = scene.objects[chair];
        // Authored room offsets flip Y, like positions do.
        assert!((moved.pos - rest.pos - vec3(0.4, 0.2, 0.0)).length() < 1.0e-5);
        assert!(
            (moved.rotation - vec2((-170.0f32).to_radians().sin(), (-170.0f32).to_radians().cos())).length() < 1.0e-5
        );
        assert!((scene.movers()[0].destination() - moved.pos.truncate()).length() < 1.0e-5);
        scene.set_motion("chair", 0.0);
        assert!((scene.objects[chair].pos - rest.pos).length() < 1.0e-6);
        assert!((scene.objects[chair].rotation - rest.rotation).length() < 1.0e-5);
    }

    #[test]
    fn turns_take_the_short_way_and_moving_objects_are_found_by_the_shaders() {
        let mut scene = scene();
        let chair = scene.movers()[0].index() as usize;
        // 170° to -170° is a 20° turn through 180°, not 340° back through zero.
        scene.set_motion("chair", 0.5);
        let angle = scene.objects[chair].rotation.x.atan2(scene.objects[chair].rotation.y).to_degrees();
        assert!(angle.abs() > 175.0, "halfway is {angle}°");
        assert!(scene.objects[chair].is_dynamic());
        assert_eq!(scene.dynamic_objects().x, chair as u32);
        assert_eq!(scene.dynamic_objects().y, u32::MAX);
        assert!(scene.objects[chair].extent().cmpgt(scene.objects[chair].local_extent()).all());
        let _ = Vec3::ZERO;
    }
}
