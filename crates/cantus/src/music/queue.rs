use super::Track;
use core::mem;
use web_time::Instant;

/// The queue, its playback position, and the on-screen position.
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

    /// Timeline offset of an entry, including the spacing before it.
    pub fn offset_ms(&self, index: usize) -> f64 {
        self.tracks[..index].iter().map(|track| f64::from(track.queue_span_ms())).sum()
    }

    /// Merges a snapshot without resetting the playback clock.
    pub fn replace(&mut self, mut tracks: Vec<Track>, index: usize, position_ms: f32) {
        let origin = self
            .tracks
            .get(self.index)
            .map(|track| (track.interaction_id, -self.timeline.start_ms - self.offset_ms(self.index)));
        let mut previous = mem::take(&mut self.tracks);
        for track in &mut tracks {
            // Reports can omit metadata or renumber UIDs; keep metadata by recording and UI state by entry.
            if let Some(old) = previous.iter().find(|old| !track.uri.is_empty() && old.uri == track.uri) {
                if track.duration_ms == 0 {
                    track.duration_ms = old.duration_ms;
                }
                for (new, old) in [(&mut track.name, &old.name), (&mut track.album, &old.album)] {
                    if new.trim().is_empty() {
                        new.clone_from(old);
                    }
                }
                if track.artists.iter().all(|artist| artist.trim().is_empty()) {
                    track.artists.clone_from(&old.artists);
                }
                if track.original_title.as_ref().is_none_or(|title| title.trim().is_empty()) {
                    track.original_title.clone_from(&old.original_title);
                }
                track.isrc = track.isrc.take().or_else(|| old.isrc.clone());
                track.image = track.image.take().or_else(|| old.image.clone());
                track.id = track.id.or(old.id);
            }
        }
        for track in &mut tracks {
            if let Some(index) =
                previous.iter().position(|old| old.uri == track.uri && old.queue_uid == track.queue_uid)
            {
                let old = previous.remove(index);
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

    /// Moves the rendered position to now. `drag` is pointer and scroll drag in ms; scroll eases in.
    pub fn update(&mut self, drag: Option<(f32, f32)>) {
        let now = Instant::now();
        let delta_time = self.timeline.updated_at.replace(now).map_or(0.0, |at| (now - at).as_secs_f32());
        if self.tracks.is_empty() {
            self.timeline.start_ms = 0.0;
            self.timeline.movement = 0.0;
            return;
        }
        let (pointer_ms, scroll_ms) = drag.unwrap_or_default();
        self.timeline.scroll_ms = if drag.is_some() {
            let ease = Timeline::ease(delta_time);
            self.timeline.scroll_ms + (f64::from(scroll_ms) - self.timeline.scroll_ms) * ease
        } else {
            0.0
        };
        let target = -self.timeline.precise_position_now() - self.offset_ms(self.index)
            + f64::from(pointer_ms)
            + self.timeline.scroll_ms;
        self.timeline.update(target, drag.is_some(), delta_time);
    }

    /// The entry under the playhead.
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
    // f64, since f32 is too coarse over hours.
    pub start_ms: f64 = 0.0,
    pub movement: f32 = 0.0,
    /// Eased scroll-wheel drag, in ms.
    pub(super) scroll_ms: f64 = 0.0,
    /// When `start_ms` last advanced.
    pub(crate) updated_at: Option<Instant> = None,
}

impl Timeline {
    /// Fraction of the remaining distance the timeline covers in `delta_time` seconds.
    fn ease(delta_time: f32) -> f64 {
        1.0 - (-f64::from(delta_time) / 0.3).exp()
    }

    pub(super) fn update(&mut self, target: f64, dragging: bool, delta_time: f32) {
        let predicted = self.start_ms - f64::from(self.rate) * f64::from(delta_time) * 1000.0;
        let next = if dragging { target } else { predicted + (target - predicted) * Self::ease(delta_time) };
        let delta_time = f64::from(delta_time);
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
    fn timeline_keeps_precision_without_paused_or_dragged_drift() {
        let mut timeline = Timeline { rate: 1.0, start_ms: -100_000_000.0, observed_at: Instant::now(), .. };
        let delta = 1.0 / 165.0;
        let step = f64::from(delta) * 1000.0;
        for _ in 0..1650 {
            let previous = timeline.start_ms;
            timeline.update(previous - step, false, delta);
            assert!(((previous - timeline.start_ms) - step).abs() < 0.000_001);
        }

        timeline.rate = 0.0;
        let target = timeline.start_ms;
        for _ in 0..165 {
            timeline.update(target, false, delta);
        }
        assert!((timeline.start_ms - target).abs() < f64::EPSILON);
        timeline.update(-1234.567, true, delta);
        assert!((timeline.start_ms + 1234.567).abs() < 0.000_001);
    }
}
