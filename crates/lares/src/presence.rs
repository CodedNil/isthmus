//! Live coordinates and a fixed initial room alignment. No inferred pose.
use isthmus::{
    ShaderData,
    glam::{Vec2, Vec3, vec2},
};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
pub use server::router;
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
mod server;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Track {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub z: f32,
    pub minz: Option<f32>,
    pub maxz: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub connected: bool,
    pub stale: bool,
    pub tracks: Vec<Track>,
}
impl Snapshot {
    pub(crate) const fn empty(connected: bool) -> Self {
        Self { connected, stale: true, tracks: Vec::new() }
    }
}

/// Three measured correspondences after the sensor was tilted, 2026-10-29.
/// See tools/presence-calibration.json for samples and plan-space anchors.
pub fn room_position(x: f32, y: f32) -> Vec2 {
    const SX: f32 = 1.86;
    const SY: f32 = 4.88;
    const KX: f32 = -2.95;
    const KY: f32 = 2.45;
    const HX: f32 = -0.69;
    const HY: f32 = 6.07;
    // Authored coordinates: desk 40 cm toward keyboard, kitchen centre,
    // and centre of the entrance-corridor/lounge intersection.
    const WX: f32 = -0.52;
    const WY: f32 = -1.83;
    const KWX: f32 = -4.2;
    const KWY: f32 = 1.5;
    const HWX: f32 = -0.35;
    const HWY: f32 = 0.5;
    const DX: f32 = KX - SX;
    const DY: f32 = KY - SY;
    const EX: f32 = HX - SX;
    const EY: f32 = HY - SY;
    const DET: f32 = DX * EY - DY * EX;
    // Solve once at compile time. No matrix inversion or fitting per update.
    const XX: f32 = ((KWX - WX) * EY - (HWX - WX) * DY) / DET;
    const XY: f32 = (DX * (HWX - WX) - EX * (KWX - WX)) / DET;
    const YX: f32 = ((KWY - WY) * EY - (HWY - WY) * DY) / DET;
    const YY: f32 = (DX * (HWY - WY) - EX * (KWY - WY)) / DET;
    // The layout compiler centres the plan at (0, 0.15) and flips Y.
    vec2(WX + XX * (x - SX) + XY * (y - SY), 0.15 - WY - YX * (x - SX) - YY * (y - SY))
}

#[derive(Clone, Copy, ShaderData)]
pub struct Ghost {
    pub position: Vec3,
    pub opacity: f32,
}

struct Detection {
    id: u32,
    point: Vec2,
    target: Vec2,
    opacity: f32,
    visible: bool,
}
#[derive(Default)]
pub struct Presence {
    pub connected: bool,
    detections: Vec<Detection>,
    pub ghosts: Vec<Ghost>,
    received: Option<Instant>,
    tick: Option<Instant>,
}
impl Presence {
    pub fn ingest(&mut self, snapshot: Snapshot) {
        self.received = Some(Instant::now());
        self.connected = snapshot.connected && !snapshot.stale;
        for detection in &mut self.detections {
            detection.visible = false;
        }
        if snapshot.connected && !snapshot.stale {
            for track in snapshot.tracks.into_iter().take(10) {
                if !track.x.is_finite()
                    || !track.y.is_finite()
                    || track.x.abs() > 20.0
                    || !(0.0..=20.0).contains(&track.y)
                {
                    continue;
                }
                let point = room_position(track.x, track.y);
                if let Some(detection) = self.detections.iter_mut().find(|d| d.id == track.id) {
                    detection.target = point;
                    detection.visible = true;
                    if detection.point.distance(point) > 2.0 {
                        detection.point = point;
                    }
                } else {
                    if self.detections.len() >= 20 {
                        let oldest = self
                            .detections
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| a.opacity.total_cmp(&b.opacity))
                            .map_or(0, |(i, _)| i);
                        self.detections.remove(oldest);
                    }
                    self.detections.push(Detection { id: track.id, point, target: point, opacity: 0.0, visible: true });
                }
            }
        }
    }

    /// Smooth radar jitter, not body motion or gestures. Lost tracks fade away.
    pub fn advance(&mut self) -> bool {
        let now = Instant::now();
        let dt = self.tick.replace(now).map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32().min(0.1));
        let blend = 1.0 - (-dt / 0.18).exp();
        let stale = self.received.is_none_or(|last| last.elapsed().as_secs_f32() > 5.0);
        if stale {
            self.connected = false;
        }
        self.ghosts.clear();
        let mut moving = false;
        for detection in &mut self.detections {
            if stale {
                detection.visible = false;
            }
            let target_opacity = f32::from(u8::from(detection.visible));
            detection.point = detection.point.lerp(detection.target, blend);
            detection.opacity += (target_opacity - detection.opacity) * blend;
            moving |= detection.point.distance_squared(detection.target) > 0.000_001
                || (target_opacity - detection.opacity).abs() > 0.005;
            if detection.opacity > 0.002 {
                self.ghosts.push(Ghost { position: detection.point.extend(0.0), opacity: detection.opacity });
            }
        }
        self.detections.retain(|d| d.visible || d.opacity > 0.002);
        moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::{self, Kind};
    #[test]
    fn alignment_preserves_all_three_post_tilt_correspondences() {
        assert!(room_position(1.86, 4.88).distance(vec2(-0.52, 1.98)) < 0.00001);
        let rooms = home::rooms();
        let vertices = rooms.iter().flat_map(|room| room.outline.iter().map(|point| room.position + *point));
        let min = vertices.clone().fold(Vec2::splat(f32::MAX), Vec2::min);
        let max = vertices.fold(Vec2::splat(f32::MIN), Vec2::max);
        let origin = (min + max) * 0.5;
        let desk = rooms
            .iter()
            .find_map(|room| {
                room.furniture
                    .iter()
                    .find(|object| object.kind == Kind::Chair)
                    .map(|chair| room.position + chair.pos.truncate())
            })
            .unwrap();
        assert!(room_position(1.86, 4.88).distance(vec2(desk.x - origin.x + 0.40, origin.y - desk.y)) < 0.00001);
        let kitchen = rooms[2].position - origin;
        assert!(room_position(-2.95, 2.45).distance(vec2(kitchen.x, -kitchen.y)) < 0.00001);
        assert!(room_position(-0.69, 6.07).distance(vec2(-0.35, -0.35)) < 0.00001);
    }
}
