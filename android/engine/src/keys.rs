//! SDL scancodes and mouse buttons to the engine's own input types.
//!
//! Scancodes are physical positions, like the desktop driver's winit key
//! codes: `KeyCode::KeyW` is the key where W sits on a US layout.

use automata_input::{KeyCode, ModifiersState, MouseButton};
use sdl3::keyboard::{Mod, Scancode};
use sdl3::mouse::MouseButton as SdlButton;

/// The engine key for an SDL scancode, if the engine has one.
#[allow(clippy::too_many_lines)] // one flat table
pub(crate) fn key(code: Scancode) -> Option<KeyCode> {
    use KeyCode as K;
    use Scancode as S;
    Some(match code {
        S::A => K::KeyA,
        S::B => K::KeyB,
        S::C => K::KeyC,
        S::D => K::KeyD,
        S::E => K::KeyE,
        S::F => K::KeyF,
        S::G => K::KeyG,
        S::H => K::KeyH,
        S::I => K::KeyI,
        S::J => K::KeyJ,
        S::K => K::KeyK,
        S::L => K::KeyL,
        S::M => K::KeyM,
        S::N => K::KeyN,
        S::O => K::KeyO,
        S::P => K::KeyP,
        S::Q => K::KeyQ,
        S::R => K::KeyR,
        S::S => K::KeyS,
        S::T => K::KeyT,
        S::U => K::KeyU,
        S::V => K::KeyV,
        S::W => K::KeyW,
        S::X => K::KeyX,
        S::Y => K::KeyY,
        S::Z => K::KeyZ,
        S::_0 => K::Digit0,
        S::_1 => K::Digit1,
        S::_2 => K::Digit2,
        S::_3 => K::Digit3,
        S::_4 => K::Digit4,
        S::_5 => K::Digit5,
        S::_6 => K::Digit6,
        S::_7 => K::Digit7,
        S::_8 => K::Digit8,
        S::_9 => K::Digit9,
        S::Left => K::ArrowLeft,
        S::Right => K::ArrowRight,
        S::Up => K::ArrowUp,
        S::Down => K::ArrowDown,
        S::Home => K::Home,
        S::End => K::End,
        S::PageUp => K::PageUp,
        S::PageDown => K::PageDown,
        S::Insert => K::Insert,
        S::Delete => K::Delete,
        S::Space => K::Space,
        S::Return => K::Enter,
        S::Escape => K::Escape,
        S::Tab => K::Tab,
        S::Backspace => K::Backspace,
        S::LShift => K::ShiftLeft,
        S::RShift => K::ShiftRight,
        S::LCtrl => K::ControlLeft,
        S::RCtrl => K::ControlRight,
        S::LAlt => K::AltLeft,
        S::RAlt => K::AltRight,
        S::LGui => K::SuperLeft,
        S::RGui => K::SuperRight,
        S::CapsLock => K::CapsLock,
        S::NumLockClear => K::NumLock,
        S::ScrollLock => K::ScrollLock,
        S::F1 => K::F1,
        S::F2 => K::F2,
        S::F3 => K::F3,
        S::F4 => K::F4,
        S::F5 => K::F5,
        S::F6 => K::F6,
        S::F7 => K::F7,
        S::F8 => K::F8,
        S::F9 => K::F9,
        S::F10 => K::F10,
        S::F11 => K::F11,
        S::F12 => K::F12,
        S::Minus => K::Minus,
        S::Equals => K::Equal,
        S::Comma => K::Comma,
        S::Period => K::Period,
        S::Slash => K::Slash,
        S::Backslash => K::Backslash,
        S::Semicolon => K::Semicolon,
        S::Apostrophe => K::Quote,
        S::Grave => K::Backquote,
        S::LeftBracket => K::BracketLeft,
        S::RightBracket => K::BracketRight,
        S::Kp0 => K::Numpad0,
        S::Kp1 => K::Numpad1,
        S::Kp2 => K::Numpad2,
        S::Kp3 => K::Numpad3,
        S::Kp4 => K::Numpad4,
        S::Kp5 => K::Numpad5,
        S::Kp6 => K::Numpad6,
        S::Kp7 => K::Numpad7,
        S::Kp8 => K::Numpad8,
        S::Kp9 => K::Numpad9,
        S::KpPlus => K::NumpadAdd,
        S::KpMinus => K::NumpadSubtract,
        S::KpMultiply => K::NumpadMultiply,
        S::KpDivide => K::NumpadDivide,
        S::KpPeriod => K::NumpadDecimal,
        S::KpEnter => K::NumpadEnter,
        S::PrintScreen => K::PrintScreen,
        S::Pause => K::Pause,
        S::Application | S::Menu => K::ContextMenu,
        // Android's Back button / gesture.
        S::AcBack => K::BrowserBack,
        _ => return None,
    })
}

/// The engine button for an SDL mouse button.
pub(crate) fn mouse_button(button: SdlButton) -> Option<MouseButton> {
    Some(match button {
        SdlButton::Left => MouseButton::Left,
        SdlButton::Right => MouseButton::Right,
        SdlButton::Middle => MouseButton::Middle,
        SdlButton::X1 => MouseButton::Back,
        SdlButton::X2 => MouseButton::Forward,
        SdlButton::Unknown => return None,
    })
}

/// The engine modifier set for SDL's.
pub(crate) fn modifiers(keymod: Mod) -> ModifiersState {
    ModifiersState::empty()
        .with(
            ModifiersState::SHIFT,
            keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        )
        .with(
            ModifiersState::CONTROL,
            keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        )
        .with(
            ModifiersState::ALT,
            keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
        )
        .with(
            ModifiersState::SUPER,
            keymod.intersects(Mod::LGUIMOD | Mod::RGUIMOD),
        )
}
