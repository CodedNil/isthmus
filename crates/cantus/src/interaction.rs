use isthmus::glam::Vec2;
use isthmus_2d::{Rect, Sdf};
use std::{
    hash::{DefaultHasher, Hash, Hasher as _},
    mem,
};

/// A widget's outline, as a point-containment test.
type Outline = Box<dyn Fn(Vec2) -> bool>;

/// Drag pixels added per scroll notch; scrolling down drags left.
const SCROLL_DRAG_STEP: f32 = 200.0;
/// Pointer travel in pixels before a press counts as a drag.
const DRAG_THRESHOLD: f32 = 2.0;

pub fn key(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Copy)]
pub enum InputEvent {
    Enter(Vec2),
    Motion(Vec2),
    Leave,
    Press,
    Release,
    /// Ends the press without a click.
    Cancel,
    Scroll(i32),
}

/// What a widget learns about the pointer this frame.
#[derive(Clone, Copy, Default)]
pub struct Response {
    pub hovered: bool,
    /// The button came up over the widget.
    pub clicked: bool,
    /// Wheel notches over the widget; positive is down.
    pub scroll: i32,
}

/// A press that moved or scrolled enough to count as a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    /// Pointer travel since the press.
    pub offset: Vec2,
    /// Horizontal travel from the scroll wheel.
    pub scroll: f32,
    /// The button came up this frame.
    pub released: bool,
}

/// A button press; the widget it landed on captures the pointer until release.
struct Press {
    origin: Vec2,
    target: Option<u64>,
    dragged: bool,
    scroll: f32,
    up: bool,
}

#[derive(Default)]
pub struct Interaction {
    /// False when another surface owns input.
    pub enabled: bool = true,
    /// Rects where the surface takes pointer input.
    pub input_regions: Vec<Rect>,
    position: Vec2,
    inside: bool,
    press: Option<Press>,
    /// Widget under the pointer, from the last frame.
    hot: Option<u64>,
    scroll: i32,
    /// This frame's widgets in drawing order; the last hit is on top.
    widgets: Vec<(u64, Outline)>,
}

impl Interaction {
    pub fn begin_frame(&mut self) {
        self.input_regions.clear();
        self.hot = self
            .pointer()
            .and_then(|point| self.hit_test(point))
            .filter(|id| self.held().is_none_or(|press| press.target == Some(*id)));
        self.widgets.clear();
    }

    /// Clears one-frame state.
    pub fn end_frame(&mut self) {
        if self.press.as_ref().is_some_and(|press| press.up) {
            self.press = None;
        }
        self.scroll = 0;
    }

    pub fn interact(&mut self, id: impl Hash, shape: impl Sdf + 'static) -> Response {
        if !self.enabled {
            return Response::default();
        }
        let id = key(id);
        debug_assert!(!self.widgets.iter().any(|(other, _)| *other == id), "duplicate widget identity");
        self.widgets.push((id, Box::new(move |point| shape.contains(point))));
        // A held widget tests its current shape; others use last frame's hit.
        let hovered = self.pointer().is_some_and(|point| {
            if self.held().is_some_and(|press| press.target == Some(id)) {
                shape.contains(point)
            } else {
                self.hot == Some(id)
            }
        });
        Response {
            hovered,
            clicked: self
                .press
                .as_ref()
                .is_some_and(|press| press.up && press.target == Some(id) && shape.contains(self.position)),
            scroll: if hovered { mem::take(&mut self.scroll) } else { 0 },
        }
    }

    /// The widget's drag, including one that just ended.
    pub fn drag_of(&self, id: impl Hash) -> Option<Drag> {
        let press =
            self.press.as_ref().filter(|press| self.enabled && press.target == Some(key(id)) && press.dragged)?;
        Some(Drag { offset: self.position - press.origin, scroll: press.scroll, released: press.up })
    }

    pub fn dragging(&self) -> bool {
        self.held().is_some_and(|press| press.dragged)
    }

    pub fn pointer_in(&self, shape: impl Sdf) -> bool {
        self.pointer().is_some_and(|point| shape.contains(point))
    }

    pub fn input_region(&mut self, rect: Rect) {
        if self.enabled && !rect.is_empty() {
            self.input_regions.push(rect);
        }
    }

    /// Last known pointer position.
    pub const fn mouse_pos(&self) -> Vec2 {
        self.position
    }

    pub const fn hovering(&self) -> bool {
        self.inside
    }

    /// The button is down anywhere.
    pub fn pressed(&self) -> bool {
        self.held().is_some()
    }

    /// Press position while held on a widget.
    pub fn press_origin(&self) -> Option<Vec2> {
        self.held().filter(|press| press.target.is_some()).map(|press| press.origin)
    }

    pub fn apply(&mut self, event: InputEvent) {
        match event {
            InputEvent::Enter(position) => {
                self.inside = true;
                self.apply(InputEvent::Motion(position));
            }
            InputEvent::Motion(position) => {
                self.position = position;
                if let Some(press) = self.press.as_mut().filter(|press| !press.up) {
                    press.dragged |= (position - press.origin).abs().max_element() >= DRAG_THRESHOLD;
                }
            }
            InputEvent::Leave => self.inside = false,
            InputEvent::Press => {
                let target = self.hit_test(self.position);
                self.press = Some(Press { origin: self.position, target, dragged: false, scroll: 0.0, up: false });
            }
            InputEvent::Release => {
                if let Some(press) = &mut self.press {
                    press.up = true;
                }
            }
            InputEvent::Cancel => self.press = None,
            InputEvent::Scroll(direction) => match self.press.as_mut().filter(|press| !press.up) {
                Some(press) if press.target.is_some() => {
                    press.scroll -= direction as f32 * SCROLL_DRAG_STEP;
                    press.dragged = true;
                }
                _ => self.scroll = self.scroll.saturating_add(direction),
            },
        }
    }

    /// The press still held.
    fn held(&self) -> Option<&Press> {
        self.press.as_ref().filter(|press| !press.up)
    }

    fn hit_test(&self, point: Vec2) -> Option<u64> {
        self.widgets.iter().rev().find(|(_, contains)| contains(point)).map(|(id, _)| *id)
    }

    /// Pointer position while over the surface or held.
    fn pointer(&self) -> Option<Vec2> {
        (self.enabled && (self.inside || self.held().is_some())).then_some(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use isthmus_2d::Shape;

    fn square() -> Shape<impl Fn(Vec2) -> f32 + Copy> {
        Shape::rectangle(Rect::new(Vec2::ZERO, Vec2::splat(10.0)))
    }

    /// One 10px widget, id 7, with the pointer pressed on it.
    fn pressed() -> Interaction {
        let mut interaction = Interaction::default();
        interaction.begin_frame();
        interaction.interact(7, square());
        interaction.apply(InputEvent::Enter(Vec2::splat(5.0)));
        interaction.apply(InputEvent::Press);
        interaction
    }

    #[test]
    fn scrolling_while_held_adds_to_the_drag() {
        let mut interaction = pressed();
        interaction.apply(InputEvent::Scroll(1));
        interaction.apply(InputEvent::Scroll(1));
        interaction.apply(InputEvent::Motion(Vec2::new(8.0, 5.0)));
        // Scrolling down drags left.
        let drag = Drag { offset: Vec2::new(3.0, 0.0), scroll: -2.0 * SCROLL_DRAG_STEP, released: false };
        assert_eq!(interaction.drag_of(7), Some(drag));
        interaction.apply(InputEvent::Release);
        assert_eq!(interaction.drag_of(7), Some(Drag { released: true, ..drag }));
    }

    #[test]
    fn a_press_is_a_click_or_a_drag_and_lasts_one_frame_after_release() {
        for (offset, dragged) in [(1.0, false), (20.0, true)] {
            let mut interaction = pressed();
            interaction.apply(InputEvent::Motion(Vec2::new(5.0 + offset, 5.0)));
            interaction.apply(InputEvent::Release);

            let drag = Drag { offset: Vec2::new(offset, 0.0), scroll: 0.0, released: true };
            assert_eq!(interaction.drag_of(7), dragged.then_some(drag));
            interaction.begin_frame();
            assert_eq!(interaction.interact(7, square()).clicked, !dragged);
            interaction.end_frame();
            assert!(interaction.drag_of(7).is_none());
            assert!(interaction.hovering() && !interaction.pressed());
        }
    }
}
