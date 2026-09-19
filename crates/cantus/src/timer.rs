//! A single in-memory countdown timer, started from the launcher.

use crate::render::launcher::similarity;
use jiff::Zoned;
use std::{mem, time::Duration};
use web_time::Instant;

/// Words which clear the running timer instead of starting one.
const CANCEL_WORDS: [&str; 5] = ["cancel", "stop", "end", "clear", "remove"];
/// Similarity below which a word is not treated as a timer command word.
const TRIGGER: f32 = 0.7;
/// Longest accepted timer; the float seconds must also stay finite.
const MAX_SECONDS: f64 = 86_400.0;

/// What the launcher asks of the timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Start(Duration),
    Cancel,
}

impl Request {
    /// Row name shown by the launcher.
    pub fn label(&self) -> String {
        match self {
            Self::Start(duration) => format!("Start {} Timer", describe(*duration)),
            Self::Cancel => "Cancel Timer".to_owned(),
        }
    }

    /// Action label for the row.
    pub const fn action(&self) -> &'static str {
        match self {
            Self::Start(_) => "Start",
            Self::Cancel => "Cancel",
        }
    }
}

/// One countdown timer; starting a new one replaces the running one.
#[derive(Default)]
pub struct Timer {
    start: Option<Zoned>,
    duration: Duration,
    deadline: Option<Instant>,
}

impl Timer {
    pub fn apply(&mut self, request: Request) {
        match request {
            Request::Start(duration) => {
                self.start = Some(Zoned::now());
                self.duration = duration;
                self.deadline = Some(Instant::now() + duration);
            }
            Request::Cancel => {
                self.start = None;
                self.deadline = None;
            }
        }
    }

    /// Readout for the weather pill, reporting completion once the deadline passes.
    pub fn readout(&self) -> Option<String> {
        let start = self.start.as_ref()?;
        let end = start.checked_add(self.duration).ok()?;
        Some(if self.expired() {
            format!("{} Timer Complete at {}", describe(self.duration), clock(&end))
        } else {
            // Remaining first, since that is what you glance for. The deadline is the useful
            // context; the start time is not.
            format!("{} · ends {}", countdown(self.remaining()), clock(&end))
        })
    }

    pub fn expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Remaining time, zero once the deadline passes.
    fn remaining(&self) -> Duration {
        self.deadline.map_or(Duration::ZERO, |deadline| deadline.saturating_duration_since(Instant::now()))
    }
}

/// Parses a launcher query into a timer request and how confidently it matched.
pub fn parse(query: &str) -> Option<(Request, f32)> {
    let words = tokens(query);
    let closest = |target: &str| words.iter().map(|word| similarity(word, target)).fold(0.0, f32::max);
    // A misspelling such as `time` still names the timer, so match the trigger loosely.
    let command = closest("timer");
    if command < TRIGGER {
        return None;
    }
    let cancel = CANCEL_WORDS.iter().map(|word| closest(word)).fold(0.0, f32::max);
    if cancel >= TRIGGER {
        return Some((Request::Cancel, cancel));
    }
    let mut seconds = 0.0;
    let mut number = None;
    for word in &words {
        if let Ok(value) = word.parse::<f64>() {
            number = Some(value);
        } else if let Some(unit) = unit(word) {
            seconds += number.take().unwrap_or(1.0) * unit;
        }
    }
    // A trailing number with no unit reads as minutes, so `timer 30` works.
    seconds += number.unwrap_or(0.0) * 60.0;
    (seconds > 0.0).then(|| (Request::Start(Duration::from_secs_f64(seconds.min(MAX_SECONDS))), command))
}

/// Splits a query into lowercase number and unit tokens, so `1h30m` reads as `1 h 30 m`.
fn tokens(query: &str) -> Vec<String> {
    let numeric = |c: char| c.is_ascii_digit() || c == '.';
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in query.chars() {
        if !c.is_alphanumeric() && c != '.' {
            if !current.is_empty() {
                tokens.push(mem::take(&mut current).to_lowercase());
            }
            continue;
        }
        if current.chars().next().is_some_and(|first| numeric(first) != numeric(c)) {
            tokens.push(mem::take(&mut current).to_lowercase());
        }
        current.push(c);
    }
    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }
    tokens
}

/// Seconds in a unit name, or `None` when the word is not a unit.
fn unit(word: &str) -> Option<f64> {
    match word {
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(3600.0),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60.0),
        "s" | "sec" | "secs" | "second" | "seconds" => Some(1.0),
        _ => None,
    }
}

/// Lowercase twelve-hour clock, such as `6:15pm`.
fn clock(time: &Zoned) -> String {
    time.strftime("%I:%M%p").to_string().trim_start_matches('0').to_lowercase()
}

/// Remaining time as `15:32`, gaining an hours field only when needed.
fn countdown(remaining: Duration) -> String {
    let total = remaining.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// Human description, such as `30 Minute` or `2 Hour`.
fn describe(duration: Duration) -> String {
    let (hours, minutes) = (duration.as_secs() / 3600, duration.as_secs() / 60 % 60);
    match (hours, minutes) {
        (0, minutes) => format!("{minutes} Minute"),
        (hours, 0) => format!("{hours} Hour"),
        (hours, minutes) => format!("{hours} Hour {minutes} Minute"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        for (query, expected) in [
            ("timer 30m", Some(Request::Start(Duration::from_mins(30)))),
            ("set a 30m timer", Some(Request::Start(Duration::from_mins(30)))),
            ("set a 2 hour timer", Some(Request::Start(Duration::from_hours(2)))),
            ("timer 1h30m", Some(Request::Start(Duration::from_mins(90)))),
            ("timer 45s", Some(Request::Start(Duration::from_secs(45)))),
            ("timer 30", Some(Request::Start(Duration::from_mins(30)))),
            ("TIMER 5 MINS", Some(Request::Start(Duration::from_mins(5)))),
            ("timer 9000 hours", Some(Request::Start(Duration::from_hours(24)))),
            // The trigger matches loosely, so a misspelling still starts a timer.
            ("30m time", Some(Request::Start(Duration::from_mins(30)))),
            ("set a 2 hour timr", Some(Request::Start(Duration::from_hours(2)))),
            ("cancel timer", Some(Request::Cancel)),
            ("stop the timer", Some(Request::Cancel)),
            // Without a duration, or without the trigger, this is not a timer command.
            ("", None),
            ("timer", None),
            ("what time is it", None),
            ("30m", None),
            ("firefox", None),
        ] {
            assert_eq!(parse(query).map(|(request, _)| request), expected, "{query}");
        }
    }

    #[test]
    fn formats_readout() {
        assert_eq!(describe(Duration::from_mins(30)), "30 Minute");
        assert_eq!(describe(Duration::from_hours(2)), "2 Hour");
        assert_eq!(describe(Duration::from_mins(90)), "1 Hour 30 Minute");
        assert_eq!(countdown(Duration::from_secs(932)), "15:32");
        assert_eq!(countdown(Duration::from_mins(65)), "1:05:00");
    }
}
