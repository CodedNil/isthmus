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
    pub fn offset_ms(&self, index: usize) -> f64 {
        self.tracks[..index].iter().map(|track| f64::from(track.queue_span_ms())).sum()
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
            .unwrap_or_else(|| (self.index, f64::from(position_ms)));
        self.timeline.start_ms = -progress - self.offset_ms(origin);
    }

    pub fn update(&mut self, drag_offset_ms: Option<f32>, delta_time: f32) {
        if self.tracks.is_empty() {
            self.timeline.start_ms = 0.0;
            self.timeline.movement = 0.0;
            return;
        }
        let target = -self.timeline.precise_position_now() - self.offset_ms(self.index)
            + f64::from(drag_offset_ms.unwrap_or(0.0));
        self.timeline.update(target, drag_offset_ms.is_some(), delta_time);
    }

    /// Queue entry covering the rendered playhead, including its trailing spacing.
    pub fn span_at_playhead(&self) -> Option<(usize, f32)> {
        let mut start_ms = self.timeline.start_ms;
        self.tracks.iter().enumerate().find_map(|(index, track)| {
            let elapsed = -start_ms;
            let current =
                (elapsed >= 0.0 && elapsed < f64::from(track.queue_span_ms())).then_some((index, elapsed as f32));
            start_ms += f64::from(track.queue_span_ms());
            current
        })
    }
}

/// A monotonic playback clock and its smoothed position on screen.
pub struct Timeline {
    pub position_ms: f32 = 0.0,
    pub rate: f32 = 0.0,
    pub observed_at: Instant,
    // Queue-relative time can span hours; f32 would quantize individual 165 Hz steps.
    pub start_ms: f64 = 0.0,
    pub movement: f32 = 0.0,
}

impl Timeline {
    pub(super) fn update(&mut self, target: f64, dragging: bool, delta_time: f32) {
        let delta_time = f64::from(delta_time);
        let predicted = self.start_ms - f64::from(self.rate) * delta_time * 1000.0;
        let next = if dragging { target } else { predicted + (target - predicted) * (1.0 - (-delta_time / 0.3).exp()) };
        self.movement +=
            (((next - self.start_ms) * delta_time) as f32 - self.movement) * (delta_time as f32 * 10.0).min(1.0);
        self.start_ms = next;
    }

    pub fn position_now(&self) -> f32 {
        self.precise_position_now() as f32
    }

    pub(super) fn precise_position_now(&self) -> f64 {
        f64::from(self.position_ms) + self.observed_at.elapsed().as_secs_f64() * 1000.0 * f64::from(self.rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_refresh_steps_keep_precision_after_a_long_queue() {
        let mut timeline = Timeline { rate: 1.0, start_ms: -100_000_000.0, observed_at: Instant::now(), .. };
        let delta = 1.0 / 165.0;
        let step = f64::from(delta) * 1000.0;
        for _ in 0..1650 {
            let previous = timeline.start_ms;
            timeline.update(previous - step, false, delta);
            assert!(((previous - timeline.start_ms) - step).abs() < 0.000_001);
        }
    }

    #[test]
    fn dragging_and_paused_clock_do_not_drift() {
        let mut timeline = Timeline { observed_at: Instant::now(), start_ms: -100_000_000.0, .. };
        let target = timeline.start_ms;
        for _ in 0..165 {
            timeline.update(target, false, 1.0 / 165.0);
        }
        assert!((timeline.start_ms - target).abs() < f64::EPSILON);
        timeline.update(-1234.567, true, 1.0 / 165.0);
        assert!((timeline.start_ms + 1234.567).abs() < 0.000_001);
    }
}
