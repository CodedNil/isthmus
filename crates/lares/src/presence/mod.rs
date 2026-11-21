//! Live radar tracks mapped into room coordinates.
use isthmus::{
    ShaderData,
    glam::{Mat2, Vec2, Vec3, vec2},
};
pub(crate) use sensor::subscribe;
use serde::Deserialize;
use std::time::Instant;

mod sensor;

#[derive(Clone, Debug, Deserialize)]
pub struct Track {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub z: f32,
    pub minz: Option<f32>,
    pub maxz: Option<f32>,
}

#[derive(Clone, Debug)]
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

/// A spot measured twice: where the radar reports it, and where it sits on the authored plan (metres).
struct Anchor {
    sensor: Vec2,
    plan: Vec2,
}

// Capture `sensor` by standing still at the spot and averaging the live track; `plan` is read off the authored layout.
const DESK: Anchor = Anchor { sensor: vec2(1.49, 5.19), plan: vec2(-0.52, -1.83) };
const KITCHEN: Anchor = Anchor { sensor: vec2(-1.55, 1.03), plan: vec2(-4.2, 1.5) };
const HALL: Anchor = Anchor { sensor: vec2(-1.49, 4.85), plan: vec2(-0.35, 0.5) };

/// The layout compiler centres the plan at this Y and flips the axis.
const PLAN_CENTRE_Y: f32 = 0.15;

/// The unique linear map plus offset taking the three anchors' sensor points onto their plan points.
fn sensor_to_plan() -> impl Fn(Vec2) -> Vec2 {
    let sensor_edges = Mat2::from_cols(KITCHEN.sensor - DESK.sensor, HALL.sensor - DESK.sensor);
    let plan_edges = Mat2::from_cols(KITCHEN.plan - DESK.plan, HALL.plan - DESK.plan);
    let linear = plan_edges * sensor_edges.inverse();
    move |sensor| DESK.plan + linear * (sensor - DESK.sensor)
}

/// Maps a radar position to world coordinates.
pub fn room_position(x: f32, y: f32) -> Vec2 {
    let plan = sensor_to_plan()(vec2(x, y));
    vec2(plan.x, PLAN_CENTRE_Y - plan.y)
}

#[derive(Clone, Copy, ShaderData)]
pub struct Ghost {
    pub position: Vec3,
    pub opacity: f32,
}

/// A tracked person, drawn gliding from where they were drawn toward their latest radar sample.
struct Detection {
    id: u32,
    /// Where they are drawn now.
    point: Vec2,
    /// Where the current glide started.
    from: Vec2,
    /// The latest radar sample.
    target: Vec2,
    /// How far through the glide, from 0 to 1.
    progress: f32,
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
    /// Smoothed seconds between radar samples, which sets how long each glide takes.
    interval: f32,
}

impl Presence {
    pub fn ingest(&mut self, snapshot: Snapshot) {
        let now = Instant::now();
        if let Some(last) = self.received.replace(now) {
            let gap = now.duration_since(last).as_secs_f32().clamp(0.2, 2.0);
            self.interval = if self.interval > 0.0 { self.interval.mul_add(0.7, gap * 0.3) } else { gap };
        }
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
                    // A new glide starts from wherever the last one got to, so motion never jumps.
                    if detection.point.distance(point) > 2.0 {
                        detection.point = point;
                    }
                    detection.from = detection.point;
                    detection.target = point;
                    detection.progress = 0.0;
                    detection.visible = true;
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
                    self.detections.push(Detection {
                        id: track.id,
                        point,
                        from: point,
                        target: point,
                        progress: 1.0,
                        opacity: 0.0,
                        visible: true,
                    });
                }
            }
        }
    }

    /// Whether a currently tracked person stands within `radius` metres of `point`.
    pub fn occupied(&self, point: Vec2, radius: f32) -> bool {
        self.connected && self.detections.iter().any(|d| d.visible && d.point.distance(point) <= radius)
    }

    /// Glides each person between radar samples, one sample interval behind, and fades out lost tracks.
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
            detection.progress = (detection.progress + dt / self.interval.max(0.2)).min(1.0);
            detection.point = detection.from.lerp(detection.target, detection.progress);
            detection.opacity += (target_opacity - detection.opacity) * blend;
            moving |= detection.progress < 1.0 || (target_opacity - detection.opacity).abs() > 0.005;
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
    use crate::{home, home::Kind, presence::*};
    use core::time::Duration;

    #[test]
    fn people_glide_linearly_between_samples_instead_of_stopping_after_each_one() {
        let track = |x: f32| Snapshot {
            connected: true,
            stale: false,
            tracks: vec![Track { id: 1, x, y: 4.0, z: 0.0, minz: None, maxz: None }],
        };
        let ago = |seconds: f32| Instant::now().checked_sub(Duration::from_secs_f32(seconds)).unwrap();
        let mut presence = Presence::default();
        presence.ingest(track(0.0));
        let start = presence.detections[0].point;
        // The next sample arrives one second later, so the glide takes one second.
        presence.received = Some(ago(1.0));
        presence.ingest(track(0.3));
        let end = room_position(0.3, 4.0);
        // Frames are 0.1 s apart, the longest step `advance` accepts.
        for frame in 1..=10 {
            presence.tick = Some(ago(0.1));
            presence.advance();
            let expected = start.lerp(end, frame as f32 / 10.0);
            assert!(presence.detections[0].point.distance(expected) < 0.001, "frame {frame}");
        }
        assert!(presence.detections[0].point.distance(end) < 0.00001, "the glide ends on the sample");
        presence.tick = Some(ago(0.1));
        assert!(!presence.advance(), "a finished glide needs no more frames");
    }
    #[test]
    fn alignment_preserves_all_three_post_tilt_correspondences() {
        assert!(room_position(1.49, 5.19).distance(vec2(-0.52, 1.98)) < 0.00001);
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
        assert!(room_position(1.49, 5.19).distance(vec2(desk.x - origin.x + 0.40, origin.y - desk.y)) < 0.00001);
        let kitchen = rooms[2].position - origin;
        assert!(room_position(-1.55, 1.03).distance(vec2(kitchen.x, -kitchen.y)) < 0.00001);
        assert!(room_position(-1.49, 4.85).distance(vec2(-0.35, -0.35)) < 0.00001);
    }
}
