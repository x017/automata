//! Input state collection and queries.

use std::collections::HashSet;

use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

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
#[derive(Debug, Default)]
pub struct InputState {
    pressed_keys: HashSet<KeyCode>,
    just_pressed_keys: HashSet<KeyCode>,
    just_released_keys: HashSet<KeyCode>,
    pressed_buttons: HashSet<MouseButton>,
    just_pressed_buttons: HashSet<MouseButton>,
    just_released_buttons: HashSet<MouseButton>,
    cursor_position: Option<(f64, f64)>,
    scroll_delta: (f32, f32),
    modifiers: ModifiersState,
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

    /// Feeds a winit window event into the state.
    ///
    /// Events that carry no input information (resize, redraw, focus, ...)
    /// are ignored. Keyboard events without a physical key code (IME input,
    /// dead keys) are ignored as well.
    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.handle_key(code, event.state);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_button(*button, *state);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position.x, position.y);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                    MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                };
                self.handle_scroll(x, y);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.handle_modifiers(modifiers.state());
            }
            _ => {}
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
                if self.pressed_keys.remove(key) {
                    self.just_released_keys.insert(key);
                }
            }
        }
    }

    /// Records a mouse button press or release.
    pub fn handle_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        match state {
            ElementState::Pressed => {
                if self.pressed_buttons.insert(button) {
                    self.just_pressed_buttons.insert(button);
                }
            }
            ElementState::Released => {
                if self.pressed_buttons.remove(button) {
                    self.just_released_buttons.insert(button);
                }
            }
        }
    }

    /// Records the cursor position, in physical pixels relative to the
    /// window's top-left corner.
    pub fn handle_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_position = Some((x, y));
    }

    /// Accumulates scroll input. Line-based and pixel-based deltas are added
    /// as-is; consumers that care about units should normalize by preference.
    pub fn handle_scroll(&mut self, x: f32, y: f32) {
        self.scroll_delta.0 += x;
        self.scroll_delta.1 += y;
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
    pub fn scroll_delta(&self) -> (f32, f32) {
        self.scroll_delta
    }

    /// The currently held modifier keys.
    #[must_use]
    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(input.button_pressed(MouseButton::Left), "held buttons persist");
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
    fn modifiers_are_tracked() {
        let mut input = InputState::new();
        assert!(!input.modifiers().shift_key());

        input.handle_modifiers(ModifiersState::SHIFT | ModifiersState::CONTROL);
        assert!(input.modifiers().shift_key());
        assert!(input.modifiers().control_key());
        assert!(!input.modifiers().alt_key());
    }
}
