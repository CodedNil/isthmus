use super::Track;
use std::mem;
use web_time::Instant;

/// Queue contents, their observed playback position, and their rendered position.
pub struct Queue {
    pub tracks: Vec<Track> = Vec::new(),
    pub index: usize = 0,
    pub timeline: Timeline,
}

impl Queue {
    pub fn observe(&mut self, index: usize, position_ms: f32, rate: f32, observed_at: Instant) {
        self.index = index.min(self.tracks.len().saturating_sub(1));
        self.timeline = Timeline { position_ms, rate, observed_at, ..self.timeline };
    }

    /// Timeline offset of a queue entry, including spacing after preceding tracks.
    pub fn offset_ms(&self, index: usize) -> f32 {
        self.tracks[..index].iter().map(Track::queue_span_ms).sum()
    }

    /// Merge a snapshot without treating metadata arrival as a playback observation.
    pub fn replace(&mut self, mut tracks: Vec<Track>, index: usize, position_ms: f32) {
        let origin = self
            .tracks
            .get(self.index)
            .map(|track| (track.interaction_id, -self.timeline.start_ms - self.offset_ms(self.index)));
        let mut previous = mem::take(&mut self.tracks);
        for track in &mut tracks {
            if let Some(index) =
                previous.iter().position(|old| old.uri == track.uri && old.queue_uid == track.queue_uid)
            {
                let old = previous.remove(index);
                // Unknown duration is missing information, not a new zero-length recording.
                if track.duration_ms == 0 {
                    track.duration_ms = old.duration_ms;
                }
                track.image = track.image.take().or(old.image);
                track.interaction_id = old.interaction_id;
                track.runtime = old.runtime;
            }
        }
        self.tracks = tracks;
        self.index = index.min(self.tracks.len().saturating_sub(1));
        let (origin, progress) = origin
            .and_then(|(id, progress)| {
                self.tracks.iter().position(|track| track.interaction_id == id).map(|index| (index, progress))
            })
            .unwrap_or((self.index, position_ms));
        self.timeline.start_ms = -progress - self.offset_ms(origin);
    }

    pub fn update(&mut self, drag_offset_ms: f32, dragging: bool, delta_time: f32) {
        if self.tracks.is_empty() {
            self.timeline.start_ms = 0.0;
            self.timeline.movement = 0.0;
            return;
        }
        let target = -self.timeline.position_now() - self.offset_ms(self.index) + drag_offset_ms;
        self.timeline.update(target, dragging, delta_time);
    }

    /// Queue entry covering the rendered playhead, including its trailing spacing.
    pub fn span_at_playhead(&self) -> Option<(usize, f32)> {
        let mut start_ms = self.timeline.start_ms;
        self.tracks.iter().enumerate().find_map(|(index, track)| {
            let elapsed = -start_ms;
            let current = (elapsed >= 0.0 && elapsed < track.queue_span_ms()).then_some((index, elapsed));
            start_ms += track.queue_span_ms();
            current
        })
    }
}

/// A monotonic playback clock and its smoothed position on screen.
pub struct Timeline {
    pub position_ms: f32 = 0.0,
    pub rate: f32 = 0.0,
    pub observed_at: Instant,
    pub start_ms: f32 = 0.0,
    pub movement: f32 = 0.0,
}

impl Timeline {
    pub(super) fn update(&mut self, target: f32, dragging: bool, delta_time: f32) {
        let predicted = self.start_ms - self.rate * delta_time * 1000.0;
        let next = if dragging { target } else { predicted + (target - predicted) * (1.0 - (-delta_time / 0.3).exp()) };
        self.movement += ((next - self.start_ms) * delta_time - self.movement) * (delta_time * 10.0).min(1.0);
        self.start_ms = next;
    }

    pub fn position_now(&self) -> f32 {
        self.position_ms + self.observed_at.elapsed().as_secs_f32() * 1000.0 * self.rate
    }
}
