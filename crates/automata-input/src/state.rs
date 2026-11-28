//! Input state collection and queries.

use std::collections::HashSet;

use crate::keys::{ElementState, InputEvent, KeyCode, ModifiersState, MouseButton, TouchPhase};

/// A snapshot of keyboard and mouse input for the current frame.
///
/// `pressed`/`button_pressed` report held inputs; `just_pressed`/
/// `just_released` report transitions since the previous frame and are
/// cleared by [`InputState::end_frame`]. OS key repeat does not re-trigger
/// `just_pressed`: a transition is recorded only when the underlying state
/// actually changes.
///
/// Physical key positions ([`KeyCode`]) are used rather than logical
/// characters, so `W` always means "the key in the W position" regardless of
/// keyboard layout — the right default for game controls.
///
/// Touch screens drive the mouse: the first finger down is the left button,
/// held where it touches, so menus and click-to-act games work on phones
/// and tablets unchanged. Every finger is also listed in
/// [`touches`](InputState::touches); without a touch screen the left mouse
/// button stands in as finger 0, so touch controls can be tried on desktop.
#[derive(Debug, Default)]
pub struct InputState {
    pressed_keys: HashSet<KeyCode>,
    just_pressed_keys: HashSet<KeyCode>,
    just_released_keys: HashSet<KeyCode>,
    pressed_buttons: HashSet<MouseButton>,
    just_pressed_buttons: HashSet<MouseButton>,
    just_released_buttons: HashSet<MouseButton>,
    cursor_position: Option<(f64, f64)>,
    scroll_delta: (f64, f64),
    mouse_delta: (f64, f64),
    modifiers: ModifiersState,
    /// The finger acting as the left button, if one is down.
    touch: Option<u64>,
    /// Fingers down this frame, and those that lifted this frame.
    touches: Vec<Touch>,
}

/// One finger on the screen (see [`InputState::touches`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Touch {
    /// A small number for the finger, stable while it's down: the lowest
    /// not in use when it landed (0 for the first finger).
    pub finger: u32,
    /// Where it is, in physical pixels like the cursor.
    pub position: (f64, f64),
    /// Where it landed.
    pub start: (f64, f64),
    /// It landed this frame.
    pub just_started: bool,
    /// It lifted (or was cancelled) this frame; it's gone next frame.
    pub ended: bool,
    /// It's the finger driving the left mouse button (or the mouse itself).
    pub primary: bool,
    /// The platform's id for the finger; `None` for the mouse stand-in.
    source: Option<u64>,
}

impl Touch {
    /// Whether this is the left mouse button standing in for a finger.
    #[must_use]
    pub fn is_mouse(&self) -> bool {
        self.source.is_none()
    }
}

impl InputState {
    /// Creates an empty input state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------
    // Event ingestion (used by the window runtime)
    // ------------------------------------------------------------------

    /// Feeds one input event from the platform driver into the state.
    pub fn handle_event(&mut self, event: &InputEvent) {
        match *event {
            InputEvent::Key { key, state } => self.handle_key(key, state),
            InputEvent::MouseButton { button, state } => self.handle_mouse_button(button, state),
            InputEvent::CursorMoved { x, y } => self.handle_cursor_moved(x, y),
            InputEvent::Scroll { x, y } => self.handle_scroll(x, y),
            InputEvent::MouseMotion { dx, dy } => self.handle_mouse_motion(dx, dy),
            InputEvent::Touch { id, phase, x, y } => self.handle_touch(id, phase, x, y),
            InputEvent::Modifiers(state) => self.handle_modifiers(state),
            InputEvent::FocusLost => self.release_all(),
        }
    }

    /// Lets go of every held key and button (reported as just released),
    /// as when the window loses focus and their releases would be missed.
    pub fn release_all(&mut self) {
        self.just_released_keys.extend(self.pressed_keys.drain());
        self.just_released_buttons
            .extend(self.pressed_buttons.drain());
        self.modifiers = ModifiersState::empty();
        self.touch = None;
        for t in &mut self.touches {
            t.ended = true;
        }
    }

    /// Records a key press or release.
    pub fn handle_key(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => {
                if self.pressed_keys.insert(key) {
                    self.just_pressed_keys.insert(key);
                }
            }
            ElementState::Released => {
                if self.pressed_keys.remove(&key) {
                    self.just_released_keys.insert(key);
                }
            }
        }
    }

    /// Records a mouse button press or release. With no finger down, the
    /// left button also acts as finger 0 in [`touches`](Self::touches).
    pub fn handle_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        self.set_button(button, state);
        if button != MouseButton::Left {
            return;
        }
        let mouse = self.touches.iter().position(|t| t.is_mouse() && !t.ended);
        match (state, mouse) {
            (ElementState::Pressed, None) if !self.touches.iter().any(|t| !t.is_mouse()) => {
                let at = self.cursor_position.unwrap_or_default();
                self.add_touch(None, at, true);
            }
            (ElementState::Released, Some(i)) => self.touches[i].ended = true,
            _ => {}
        }
    }

    /// Lists a new finger in the lowest free slot.
    fn add_touch(&mut self, source: Option<u64>, at: (f64, f64), primary: bool) {
        // One of 0..=len is always free.
        let finger = (0..=u32::try_from(self.touches.len()).unwrap_or(u32::MAX))
            .find(|f| !self.touches.iter().any(|t| t.finger == *f))
            .unwrap_or_default();
        self.touches.push(Touch {
            finger,
            position: at,
            start: at,
            just_started: true,
            ended: false,
            primary,
            source,
        });
    }

    /// The live (not lifted) touch from `source`.
    fn live_touch(&mut self, source: Option<u64>) -> Option<&mut Touch> {
        self.touches
            .iter_mut()
            .find(|t| t.source == source && !t.ended)
    }

    fn set_button(&mut self, button: MouseButton, state: ElementState) {
        match state {
            ElementState::Pressed => {
                if self.pressed_buttons.insert(button) {
                    self.just_pressed_buttons.insert(button);
                }
            }
            ElementState::Released => {
                if self.pressed_buttons.remove(&button) {
                    self.just_released_buttons.insert(button);
                }
            }
        }
    }

    /// Records the cursor position, in physical pixels relative to the
    /// window's top-left corner.
    pub fn handle_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_position = Some((x, y));
        if let Some(t) = self.live_touch(None) {
            t.position = (x, y);
        }
    }

    /// Records a touch, in physical pixels like the cursor. The first
    /// finger down presses the left button and moves the cursor until it
    /// lifts; fingers that land while it's down are ignored.
    pub fn handle_touch(&mut self, id: u64, phase: TouchPhase, x: f64, y: f64) {
        self.track_touch(id, phase, (x, y));
        match phase {
            TouchPhase::Started if self.touch.is_none() => {
                self.touch = Some(id);
                self.cursor_position = Some((x, y));
                self.set_button(MouseButton::Left, ElementState::Pressed);
            }
            TouchPhase::Moved if self.touch == Some(id) => {
                self.cursor_position = Some((x, y));
            }
            TouchPhase::Ended | TouchPhase::Cancelled if self.touch == Some(id) => {
                self.touch = None;
                self.cursor_position = Some((x, y));
                self.set_button(MouseButton::Left, ElementState::Released);
            }
            _ => {}
        }
    }

    /// Keeps [`touches`](Self::touches) up to date for one touch event.
    fn track_touch(&mut self, id: u64, phase: TouchPhase, at: (f64, f64)) {
        match phase {
            TouchPhase::Started => {
                if self.live_touch(Some(id)).is_none() {
                    // A real finger replaces the mouse stand-in.
                    for t in &mut self.touches {
                        t.ended |= t.is_mouse();
                    }
                    let primary = self.touch.is_none();
                    self.add_touch(Some(id), at, primary);
                }
            }
            TouchPhase::Moved => {
                if let Some(t) = self.live_touch(Some(id)) {
                    t.position = at;
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if let Some(t) = self.live_touch(Some(id)) {
                    t.position = at;
                    t.ended = true;
                }
            }
        }
    }

    /// Accumulates scroll input. Line-based and pixel-based deltas are added
    /// as-is; consumers that care about units should normalize by preference.
    pub fn handle_scroll(&mut self, x: f64, y: f64) {
        self.scroll_delta.0 += x;
        self.scroll_delta.1 += y;
    }

    /// Accumulates raw mouse movement.
    /// Unlike the cursor position it keeps reporting at the screen edge and
    /// while the cursor is locked, which is what mouse-look needs.
    pub fn handle_mouse_motion(&mut self, dx: f64, dy: f64) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Records the currently held modifier keys.
    pub fn handle_modifiers(&mut self, state: ModifiersState) {
        self.modifiers = state;
    }

    /// Clears per-frame state. Called by the main loop after each update.
    pub fn end_frame(&mut self) {
        self.just_pressed_keys.clear();
        self.just_released_keys.clear();
        self.just_pressed_buttons.clear();
        self.just_released_buttons.clear();
        self.scroll_delta = (0.0, 0.0);
        self.mouse_delta = (0.0, 0.0);
        self.touches.retain(|t| !t.ended);
        for t in &mut self.touches {
            t.just_started = false;
        }
    }

    // ------------------------------------------------------------------
    // Queries (used by game logic during the update)
    // ------------------------------------------------------------------

    /// Whether `key` is currently held down.
    #[must_use]
    pub fn pressed(&self, key: KeyCode) -> bool {
        self.pressed_keys.contains(&key)
    }

    /// Whether `key` transitioned to pressed since the previous frame.
    #[must_use]
    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed_keys.contains(&key)
    }

    /// Whether `key` was released since the previous frame.
    #[must_use]
    pub fn just_released(&self, key: KeyCode) -> bool {
        self.just_released_keys.contains(&key)
    }

    /// Whether `button` is currently held down.
    #[must_use]
    pub fn button_pressed(&self, button: MouseButton) -> bool {
        self.pressed_buttons.contains(&button)
    }

    /// Whether `button` transitioned to pressed since the previous frame.
    #[must_use]
    pub fn button_just_pressed(&self, button: MouseButton) -> bool {
        self.just_pressed_buttons.contains(&button)
    }

    /// Whether `button` was released since the previous frame.
    #[must_use]
    pub fn button_just_released(&self, button: MouseButton) -> bool {
        self.just_released_buttons.contains(&button)
    }

    /// The cursor position in physical pixels, if it is inside the window.
    #[must_use]
    pub fn cursor_position(&self) -> Option<(f64, f64)> {
        self.cursor_position
    }

    /// The scroll delta accumulated since the previous frame.
    #[must_use]
    pub fn scroll_delta(&self) -> (f64, f64) {
        self.scroll_delta
    }

    /// Raw mouse movement accumulated since the previous frame (`+x` right,
    /// `+y` down), in device units (roughly pixels).
    #[must_use]
    pub fn mouse_delta(&self) -> (f64, f64) {
        self.mouse_delta
    }

    /// The currently held modifier keys.
    #[must_use]
    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }

    /// The fingers on the screen this frame, plus those that lifted this
    /// frame (`ended`), in the order they landed. Without a touch screen
    /// the held left mouse button is listed as finger 0.
    #[must_use]
    pub fn touches(&self) -> &[Touch] {
        &self.touches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingers(input: &InputState) -> Vec<(u32, bool, bool, bool)> {
        input
            .touches()
            .iter()
            .map(|t| (t.finger, t.just_started, t.ended, t.primary))
            .collect()
    }

    #[test]
    fn every_finger_is_listed_with_its_own_slot() {
        let mut input = InputState::new();
        input.handle_touch(70, TouchPhase::Started, 10.0, 10.0);
        input.handle_touch(71, TouchPhase::Started, 50.0, 60.0);
        assert_eq!(
            fingers(&input),
            [(0, true, false, true), (1, true, false, false)]
        );
        input.end_frame();
        input.handle_touch(71, TouchPhase::Moved, 55.0, 66.0);
        input.handle_touch(70, TouchPhase::Ended, 12.0, 11.0);
        let t = input.touches()[1];
        assert_eq!((t.position, t.start), ((55.0, 66.0), (50.0, 60.0)));
        assert_eq!(
            fingers(&input),
            [(0, false, true, true), (1, false, false, false)]
        );
        input.end_frame();
        // Slot 0 is free again; finger 1 keeps its number.
        input.handle_touch(72, TouchPhase::Started, 1.0, 1.0);
        assert_eq!(
            fingers(&input),
            [(1, false, false, false), (0, true, false, true)]
        );
        assert!(!input.touches()[0].is_mouse());
        input.handle_touch(71, TouchPhase::Cancelled, 0.0, 0.0);
        input.end_frame();
        assert_eq!(fingers(&input), [(0, false, false, true)]);
    }

    #[test]
    fn the_left_mouse_button_stands_in_for_a_finger() {
        let mut input = InputState::new();
        input.handle_cursor_moved(30.0, 40.0);
        input.handle_mouse_button(MouseButton::Right, ElementState::Pressed);
        assert_eq!(input.touches(), []);
        input.handle_mouse_button(MouseButton::Left, ElementState::Pressed);
        assert_eq!(fingers(&input), [(0, true, false, true)]);
        assert!(input.touches()[0].is_mouse());
        input.end_frame();
        input.handle_cursor_moved(35.0, 41.0);
        assert_eq!(input.touches()[0].position, (35.0, 41.0));
        assert_eq!(input.touches()[0].start, (30.0, 40.0));
        input.handle_mouse_button(MouseButton::Left, ElementState::Released);
        assert_eq!(fingers(&input), [(0, false, true, true)]);
        input.end_frame();
        assert_eq!(input.touches(), []);
        // A finger's emulated left button doesn't add a mouse finger.
        input.handle_touch(9, TouchPhase::Started, 5.0, 5.0);
        assert_eq!(input.touches().len(), 1);
        assert!(!input.touches()[0].is_mouse());
    }

    #[test]
    fn driver_events_feed_the_state_and_losing_focus_lets_go() {
        let mut input = InputState::new();
        for e in [
            InputEvent::Key {
                key: KeyCode::KeyW,
                state: ElementState::Pressed,
            },
            InputEvent::MouseButton {
                button: MouseButton::Right,
                state: ElementState::Pressed,
            },
            InputEvent::CursorMoved { x: 10.0, y: 20.0 },
            InputEvent::Scroll { x: 0.0, y: 1.0 },
            InputEvent::MouseMotion { dx: 3.0, dy: -1.0 },
            InputEvent::Modifiers(ModifiersState::ALT),
        ] {
            input.handle_event(&e);
        }
        assert!(input.pressed(KeyCode::KeyW) && input.button_pressed(MouseButton::Right));
        assert_eq!(input.cursor_position(), Some((10.0, 20.0)));
        assert_eq!(input.scroll_delta(), (0.0, 1.0));
        assert_eq!(input.mouse_delta(), (3.0, -1.0));
        assert!(input.modifiers().alt_key());
        input.end_frame();
        input.handle_event(&InputEvent::FocusLost);
        assert!(!input.pressed(KeyCode::KeyW) && input.just_released(KeyCode::KeyW));
        assert!(input.button_just_released(MouseButton::Right));
        assert!(!input.modifiers().alt_key());
        input.handle_event(&InputEvent::Touch {
            id: 4,
            phase: TouchPhase::Started,
            x: 1.0,
            y: 2.0,
        });
        assert!(
            input.button_pressed(MouseButton::Left),
            "a finger is the left button"
        );
    }

    #[test]
    fn key_press_is_pressed_and_just_pressed() {
        let mut input = InputState::new();
        input.handle_key(KeyCode::KeyW, ElementState::Pressed);

        assert!(input.pressed(KeyCode::KeyW));
        assert!(input.just_pressed(KeyCode::KeyW));
        assert!(!input.just_released(KeyCode::KeyW));
        assert!(!input.pressed(KeyCode::KeyA));
    }

    #[test]
    fn repeated_press_without_release_is_not_just_pressed_again() {
        let mut input = InputState::new();
        input.handle_key(KeyCode::KeyW, ElementState::Pressed);
        input.end_frame();

        // OS key repeat: another Pressed without an intervening Released.
        input.handle_key(KeyCode::KeyW, ElementState::Pressed);
        assert!(input.pressed(KeyCode::KeyW));
        assert!(
            !input.just_pressed(KeyCode::KeyW),
            "key repeat must not re-trigger just_pressed"
        );
    }

    #[test]
    fn key_release_clears_pressed_and_marks_just_released() {
        let mut input = InputState::new();
        input.handle_key(KeyCode::Space, ElementState::Pressed);
        input.end_frame();
        input.handle_key(KeyCode::Space, ElementState::Released);

        assert!(!input.pressed(KeyCode::Space));
        assert!(input.just_released(KeyCode::Space));
        assert!(!input.just_pressed(KeyCode::Space));
    }

    #[test]
    fn end_frame_clears_only_per_frame_state() {
        let mut input = InputState::new();
        input.handle_key(KeyCode::ShiftLeft, ElementState::Pressed);
        input.handle_mouse_button(MouseButton::Left, ElementState::Pressed);
        input.handle_scroll(1.0, -2.0);
        input.end_frame();

        assert!(input.pressed(KeyCode::ShiftLeft), "held keys persist");
        assert!(
            input.button_pressed(MouseButton::Left),
            "held buttons persist"
        );
        assert!(!input.just_pressed(KeyCode::ShiftLeft));
        assert!(!input.button_just_pressed(MouseButton::Left));
        assert_eq!(input.scroll_delta(), (0.0, 0.0));
    }

    #[test]
    fn mouse_button_transitions_are_tracked() {
        let mut input = InputState::new();
        input.handle_mouse_button(MouseButton::Right, ElementState::Pressed);

        assert!(input.button_pressed(MouseButton::Right));
        assert!(input.button_just_pressed(MouseButton::Right));
        assert!(!input.button_pressed(MouseButton::Middle));

        input.end_frame();
        input.handle_mouse_button(MouseButton::Right, ElementState::Released);
        assert!(input.button_just_released(MouseButton::Right));
    }

    #[test]
    fn scroll_deltas_accumulate_within_a_frame() {
        let mut input = InputState::new();
        input.handle_scroll(0.0, 1.0);
        input.handle_scroll(0.0, 2.5);
        assert_eq!(input.scroll_delta(), (0.0, 3.5));
    }

    #[test]
    fn cursor_position_is_tracked() {
        let mut input = InputState::new();
        assert_eq!(input.cursor_position(), None);

        input.handle_cursor_moved(10.0, 20.5);
        assert_eq!(input.cursor_position(), Some((10.0, 20.5)));

        input.end_frame();
        assert_eq!(
            input.cursor_position(),
            Some((10.0, 20.5)),
            "cursor position persists across frames"
        );
    }

    #[test]
    fn the_first_finger_is_the_left_button_where_it_touches() {
        let mut input = InputState::new();
        input.handle_touch(7, TouchPhase::Started, 100.0, 50.0);
        assert!(input.button_just_pressed(MouseButton::Left));
        assert_eq!(input.cursor_position(), Some((100.0, 50.0)));
        input.end_frame();

        // A second finger neither moves the cursor nor lifts the button.
        input.handle_touch(8, TouchPhase::Started, 400.0, 300.0);
        input.handle_touch(8, TouchPhase::Ended, 400.0, 300.0);
        input.handle_touch(7, TouchPhase::Moved, 120.0, 60.0);
        assert!(input.button_pressed(MouseButton::Left));
        assert!(!input.button_just_pressed(MouseButton::Left));
        assert_eq!(input.cursor_position(), Some((120.0, 60.0)));

        input.handle_touch(7, TouchPhase::Ended, 125.0, 62.0);
        assert!(input.button_just_released(MouseButton::Left));
        assert!(!input.button_pressed(MouseButton::Left));
        input.end_frame();

        // Once it lifts, the next finger down takes over.
        input.handle_touch(8, TouchPhase::Started, 10.0, 10.0);
        assert!(input.button_just_pressed(MouseButton::Left));
        input.handle_touch(8, TouchPhase::Cancelled, 10.0, 10.0);
        assert!(!input.button_pressed(MouseButton::Left));
    }

    #[test]
    fn a_tap_inside_one_frame_still_reads_as_a_press_and_release() {
        let mut input = InputState::new();
        input.handle_touch(1, TouchPhase::Started, 5.0, 5.0);
        input.handle_touch(1, TouchPhase::Ended, 5.0, 5.0);
        assert!(input.button_just_pressed(MouseButton::Left));
        assert!(input.button_just_released(MouseButton::Left));
    }

    #[test]
    fn modifiers_are_tracked() {
        let mut input = InputState::new();
        assert!(!input.modifiers().shift_key());

        input.handle_modifiers(ModifiersState::SHIFT | ModifiersState::CONTROL);
        assert!(input.modifiers().shift_key());
        assert!(input.modifiers().control_key());
        assert!(!input.modifiers().alt_key());
    }
}
