use crate::render::{HELD_PRESSURE, RIPPLE_COUNT, RipplePulse};
use isthmus::{Quad, glam::Vec2};
use isthmus_sdf::Shape;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    mem,
};

pub fn key(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

struct Widget {
    contains: Box<dyn Fn(Vec2) -> bool>,
    id: Active,
}

#[derive(Clone, Copy)]
enum Pointer {
    Outside(Vec2),
    Hovering(Vec2),
    Held { position: Vec2, origin: Vec2, dragging: bool },
}

#[derive(Clone, Copy)]
enum ButtonEvent {
    Press(Vec2),
    Release { id: Active, position: Vec2, drag_origin: Vec2, was_dragging: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Active {
    Widget(u64),
    Drag(u64),
}

#[derive(Default)]
pub struct Interaction {
    pub enabled: bool = true,
    pointer: Pointer = Pointer::Outside(Vec2::ZERO),
    pressure: f32,
    pub ripples: [RipplePulse; RIPPLE_COUNT],
    events: Vec<ButtonEvent>,
    hot: Option<Active>,
    active: Option<Active>,
    scroll: i32,
    held_seconds: f32,
    previous_held_seconds: f32,
    widgets: Vec<Widget>,
    pub input_regions: Vec<Quad>,
}

#[derive(Clone, Copy, Default)]
pub struct Response {
    pub hovered: bool,
    pub clicked: bool,
    pub held: bool,
    pub scroll: i32,
    pub held_seconds: f32,
    previous_held_seconds: f32,
}

#[derive(Clone, Copy)]
pub enum InputEvent {
    Enter(Vec2),
    Motion(Vec2),
    Leave,
    Press,
    Release,
    Cancel,
    Scroll(i32),
}

impl Response {
    pub fn held_for(&self, seconds: f32) -> bool {
        self.held && self.previous_held_seconds < seconds && self.held_seconds >= seconds
    }
}

impl Interaction {
    pub fn begin_frame(&mut self, delta_time: f32, time: f32) {
        self.input_regions.clear();
        self.hot = self.pointer().and_then(|pointer| self.hit_test(pointer));
        self.widgets.clear();
        self.previous_held_seconds = self.held_seconds;
        if self.down() {
            self.held_seconds += delta_time;
        } else {
            self.held_seconds = 0.0;
            self.previous_held_seconds = 0.0;
        }
        let target_pressure = match self.pointer {
            Pointer::Outside(_) => 0.0,
            Pointer::Hovering(_) => 1.0,
            Pointer::Held { .. } => HELD_PRESSURE,
        };
        self.pressure += (target_pressure - self.pressure).clamp(-5.0 * delta_time, 5.0 * delta_time);
        for event in &self.events {
            let ButtonEvent::Press(origin) = event else {
                continue;
            };
            let pulse = RipplePulse { origin: *origin, start_time: time };
            if let Some(ripple) = self.ripples.iter_mut().min_by(|a, b| a.start_time.total_cmp(&b.start_time)) {
                *ripple = pulse;
            }
        }
    }

    pub fn end_frame(&mut self) {
        if !self.down() {
            self.active = None;
        }
        self.events.clear();
        self.scroll = 0;
    }

    pub fn interact<S: Fn(Vec2) -> f32 + Copy + 'static>(&mut self, identity: impl Hash, shape: Shape<S>) -> Response {
        self.interact_with(shape, Active::Widget(key(identity)))
    }

    pub fn drag<S: Fn(Vec2) -> f32 + Copy + 'static>(&mut self, key: u64, shape: Shape<S>) -> Response {
        self.interact_with(shape, Active::Drag(key))
    }

    pub fn pointer_in<S: Fn(Vec2) -> f32 + Copy>(&self, shape: Shape<S>) -> bool {
        self.pointer().is_some_and(|pointer| shape.contains(pointer))
    }

    pub fn input_region(&mut self, quad: impl Into<Quad>) {
        if self.enabled {
            self.input_regions.push(quad.into());
        }
    }

    pub const fn pressure(&self) -> f32 {
        if self.enabled { self.pressure } else { 0.0 }
    }

    pub const fn mouse_pos(&self) -> Vec2 {
        match self.pointer {
            Pointer::Outside(position) | Pointer::Hovering(position) | Pointer::Held { position, .. } => position,
        }
    }

    pub const fn dragging(&self) -> bool {
        matches!(self.pointer, Pointer::Held { dragging: true, .. })
    }

    /// Captured drag displacement and release state, even when its widget is no longer drawn.
    pub fn drag_motion(&self) -> Option<(Vec2, bool)> {
        if !self.enabled {
            return None;
        }
        if matches!(self.active, Some(Active::Drag(_)))
            && self.dragging()
            && let Pointer::Held { position, origin, .. } = self.pointer
        {
            return Some((position - origin, false));
        }
        self.events.iter().rev().find_map(|event| match event {
            ButtonEvent::Release { id: Active::Drag(_), position, drag_origin, was_dragging: true } => {
                Some((*position - *drag_origin, true))
            }
            _ => None,
        })
    }

    pub fn apply(&mut self, event: InputEvent) {
        match event {
            InputEvent::Enter(position) => {
                if !self.down() {
                    self.pointer = Pointer::Hovering(position);
                }
                self.apply(InputEvent::Motion(position));
            }
            InputEvent::Motion(position) => {
                self.hot = self.hit_test(position);
                if matches!(self.active, Some(Active::Widget(_))) && self.hot.is_none() {
                    self.apply(InputEvent::Cancel);
                }
                self.pointer = match self.pointer {
                    Pointer::Held { origin, dragging, .. } => Pointer::Held {
                        position,
                        origin,
                        dragging: dragging
                            || matches!(self.active, Some(Active::Drag(_)))
                                && (position - origin).abs().max_element() >= 2.0,
                    },
                    Pointer::Hovering(_) => Pointer::Hovering(position),
                    Pointer::Outside(_) => Pointer::Outside(position),
                };
            }
            InputEvent::Leave => {
                if !matches!(self.active, Some(Active::Drag(_))) {
                    self.apply(InputEvent::Cancel);
                    self.pointer = Pointer::Outside(self.mouse_pos());
                }
            }
            InputEvent::Press => {
                let position = self.mouse_pos();
                self.pointer = Pointer::Held { position, origin: position, dragging: false };
                self.hot = self.widgets.iter().rev().find(|widget| (widget.contains)(position)).map(|widget| widget.id);
                self.active = self.hot;
                self.events.push(ButtonEvent::Press(position));
                self.held_seconds = 0.0;
                self.previous_held_seconds = 0.0;
            }
            InputEvent::Release => {
                if let Pointer::Held { origin, dragging, position } = self.pointer
                    && let Some(id) = self.active.take()
                    && (matches!(id, Active::Drag(_))
                        || self.widgets.iter().any(|widget| widget.id == id && (widget.contains)(position)))
                {
                    self.events.push(ButtonEvent::Release {
                        id,
                        position,
                        drag_origin: origin,
                        was_dragging: dragging,
                    });
                }
                let position = self.mouse_pos();
                self.pointer = Pointer::Hovering(position);
                self.hot = self.hit_test(position);
            }
            InputEvent::Cancel => {
                self.pointer = Pointer::Hovering(self.mouse_pos());
                self.active = None;
                self.held_seconds = 0.0;
                self.previous_held_seconds = 0.0;
            }
            InputEvent::Scroll(direction) => self.scroll = self.scroll.saturating_add(direction),
        }
    }

    fn hit_test(&self, point: Vec2) -> Option<Active> {
        self.widgets
            .iter()
            .rev()
            .find(|widget| (!self.down() || self.active == Some(widget.id)) && (widget.contains)(point))
            .map(|widget| widget.id)
    }

    fn interact_with<S: Fn(Vec2) -> f32 + Copy + 'static>(&mut self, shape: Shape<S>, id: Active) -> Response {
        if !self.enabled {
            return Response::default();
        }
        debug_assert!(!self.widgets.iter().any(|widget| widget.id == id), "duplicate widget identity");
        self.widgets.push(Widget { contains: Box::new(move |point| shape.contains(point)), id });
        let dragging = matches!(id, Active::Drag(_));
        if !dragging && self.active == Some(id) && !self.pointer_in(shape) {
            self.apply(InputEvent::Cancel);
        }
        let active = self.active == Some(id);
        let hovered = self.pointer().is_some()
            && if dragging && active { shape.contains(self.mouse_pos()) } else { self.hot == Some(id) };
        Response {
            hovered,
            clicked: self.events.iter().any(
                |event| matches!(event, ButtonEvent::Release { id: target, was_dragging: false, .. } if *target == id),
            ),
            held: active && self.down(),
            scroll: if hovered { mem::take(&mut self.scroll) } else { 0 },
            held_seconds: self.held_seconds,
            previous_held_seconds: self.previous_held_seconds,
        }
    }

    const fn pointer(&self) -> Option<Vec2> {
        if !self.enabled {
            return None;
        }
        match self.pointer {
            Pointer::Outside(_) => None,
            Pointer::Hovering(position) | Pointer::Held { position, .. } => Some(position),
        }
    }

    const fn down(&self) -> bool {
        matches!(self.pointer, Pointer::Held { .. })
    }
}
