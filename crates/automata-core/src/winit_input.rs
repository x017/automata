//! Translates winit's window events into the engine's [`InputEvent`]s (the
//! desktop driver's half of `automata_input`'s platform-independent types).
//! Oberon uses [`key_code`] too, so its Play knows the same keys.

use automata_input::{ElementState, InputEvent, KeyCode, ModifiersState, MouseButton, TouchPhase};
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode as WK, PhysicalKey};

/// Declares the two-way key table once.
macro_rules! key_table {
    ($($ours:ident => $theirs:ident,)*) => {
        /// The engine key for a winit key code (`None`: one games can't
        /// bind, such as media keys).
        #[must_use]
        pub fn key_code(code: WK) -> Option<KeyCode> {
            match code {
                $( WK::$theirs => Some(KeyCode::$ours), )*
                _ => None,
            }
        }

        /// The winit key code for an engine key (the inverse of
        /// [`key_code`]).
        #[cfg(test)]
        fn winit_key(key: KeyCode) -> Option<WK> {
            match key {
                $( KeyCode::$ours => Some(WK::$theirs), )*
                _ => None,
            }
        }
    };
}

key_table! {
    KeyA => KeyA, KeyB => KeyB, KeyC => KeyC, KeyD => KeyD, KeyE => KeyE,
    KeyF => KeyF, KeyG => KeyG, KeyH => KeyH, KeyI => KeyI, KeyJ => KeyJ,
    KeyK => KeyK, KeyL => KeyL, KeyM => KeyM, KeyN => KeyN, KeyO => KeyO,
    KeyP => KeyP, KeyQ => KeyQ, KeyR => KeyR, KeyS => KeyS, KeyT => KeyT,
    KeyU => KeyU, KeyV => KeyV, KeyW => KeyW, KeyX => KeyX, KeyY => KeyY,
    KeyZ => KeyZ,
    Digit0 => Digit0, Digit1 => Digit1, Digit2 => Digit2, Digit3 => Digit3,
    Digit4 => Digit4, Digit5 => Digit5, Digit6 => Digit6, Digit7 => Digit7,
    Digit8 => Digit8, Digit9 => Digit9,
    ArrowLeft => ArrowLeft, ArrowRight => ArrowRight, ArrowUp => ArrowUp,
    ArrowDown => ArrowDown, Home => Home, End => End, PageUp => PageUp,
    PageDown => PageDown, Insert => Insert, Delete => Delete,
    Space => Space, Enter => Enter, Escape => Escape, Tab => Tab,
    Backspace => Backspace,
    ShiftLeft => ShiftLeft, ShiftRight => ShiftRight,
    ControlLeft => ControlLeft, ControlRight => ControlRight,
    AltLeft => AltLeft, AltRight => AltRight,
    SuperLeft => SuperLeft, SuperRight => SuperRight,
    CapsLock => CapsLock, NumLock => NumLock, ScrollLock => ScrollLock,
    F1 => F1, F2 => F2, F3 => F3, F4 => F4, F5 => F5, F6 => F6,
    F7 => F7, F8 => F8, F9 => F9, F10 => F10, F11 => F11, F12 => F12,
    Minus => Minus, Equal => Equal, Comma => Comma, Period => Period,
    Slash => Slash, Backslash => Backslash, Semicolon => Semicolon,
    Quote => Quote, Backquote => Backquote, BracketLeft => BracketLeft,
    BracketRight => BracketRight,
    Numpad0 => Numpad0, Numpad1 => Numpad1, Numpad2 => Numpad2,
    Numpad3 => Numpad3, Numpad4 => Numpad4, Numpad5 => Numpad5,
    Numpad6 => Numpad6, Numpad7 => Numpad7, Numpad8 => Numpad8,
    Numpad9 => Numpad9, NumpadAdd => NumpadAdd,
    NumpadSubtract => NumpadSubtract, NumpadMultiply => NumpadMultiply,
    NumpadDivide => NumpadDivide, NumpadDecimal => NumpadDecimal,
    NumpadEnter => NumpadEnter,
    PrintScreen => PrintScreen, Pause => Pause, ContextMenu => ContextMenu,
    BrowserBack => BrowserBack,
}

fn element_state(state: winit::event::ElementState) -> ElementState {
    match state {
        winit::event::ElementState::Pressed => ElementState::Pressed,
        winit::event::ElementState::Released => ElementState::Released,
    }
}

fn mouse_button(button: winit::event::MouseButton) -> MouseButton {
    use winit::event::MouseButton as B;
    match button {
        B::Left => MouseButton::Left,
        B::Right => MouseButton::Right,
        B::Middle => MouseButton::Middle,
        B::Back => MouseButton::Back,
        B::Forward => MouseButton::Forward,
        B::Other(n) => MouseButton::Other(n),
    }
}

fn modifiers(state: winit::keyboard::ModifiersState) -> ModifiersState {
    ModifiersState::empty()
        .with(ModifiersState::SHIFT, state.shift_key())
        .with(ModifiersState::CONTROL, state.control_key())
        .with(ModifiersState::ALT, state.alt_key())
        .with(ModifiersState::SUPER, state.super_key())
}

fn touch_phase(phase: winit::event::TouchPhase) -> TouchPhase {
    use winit::event::TouchPhase as P;
    match phase {
        P::Started => TouchPhase::Started,
        P::Moved => TouchPhase::Moved,
        P::Ended => TouchPhase::Ended,
        P::Cancelled => TouchPhase::Cancelled,
    }
}

/// The input event a window event carries, if any. Keys without a physical
/// code (IME input, dead keys) and keys games can't bind are dropped.
#[must_use]
pub fn input_event(event: &WindowEvent) -> Option<InputEvent> {
    Some(match event {
        WindowEvent::KeyboardInput { event, .. } => {
            let PhysicalKey::Code(code) = event.physical_key else {
                return None;
            };
            InputEvent::Key {
                key: key_code(code)?,
                state: element_state(event.state),
            }
        }
        WindowEvent::MouseInput { state, button, .. } => InputEvent::MouseButton {
            button: mouse_button(*button),
            state: element_state(*state),
        },
        WindowEvent::CursorMoved { position, .. } => InputEvent::CursorMoved {
            x: position.x,
            y: position.y,
        },
        WindowEvent::MouseWheel { delta, .. } => {
            let (x, y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (f64::from(*x), f64::from(*y)),
                MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
            };
            InputEvent::Scroll { x, y }
        }
        WindowEvent::ModifiersChanged(m) => InputEvent::Modifiers(modifiers(m.state())),
        WindowEvent::Touch(touch) => InputEvent::Touch {
            id: touch.id,
            phase: touch_phase(touch.phase),
            x: touch.location.x,
            y: touch.location.y,
        },
        WindowEvent::Focused(false) => InputEvent::FocusLost,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_key_has_a_winit_key_and_back() {
        for &key in KeyCode::ALL {
            let theirs = winit_key(key).unwrap_or_else(|| panic!("{key:?} has no winit key"));
            assert_eq!(key_code(theirs), Some(key));
        }
        assert_eq!(key_code(WK::MediaPlayPause), None);
    }

    #[test]
    fn buttons_modifiers_and_touches_translate() {
        assert_eq!(
            mouse_button(winit::event::MouseButton::Other(9)),
            MouseButton::Other(9)
        );
        let m = modifiers(
            winit::keyboard::ModifiersState::SHIFT | winit::keyboard::ModifiersState::SUPER,
        );
        assert!(m.shift_key() && m.super_key() && !m.control_key());
        assert_eq!(
            touch_phase(winit::event::TouchPhase::Cancelled),
            TouchPhase::Cancelled
        );
        assert_eq!(
            input_event(&WindowEvent::Focused(false)),
            Some(InputEvent::FocusLost)
        );
        assert_eq!(input_event(&WindowEvent::Focused(true)), None);
    }
}
