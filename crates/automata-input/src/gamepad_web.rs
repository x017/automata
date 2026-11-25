//! The browser gamepad backend (emscripten builds).
//!
//! gilrs's web backend needs wasm-bindgen, which an emscripten module
//! (the Pyodide extension) can't use. The page polls the Gamepad API
//! (`navigator.getGamepads()`) once per animation frame instead and feeds
//! the pads in as [`PadSnapshot`]s; [`Gamepads::feed`] turns the changes
//! into [`GamepadEvent`]s. The button and axis types mirror gilrs's, so
//! game code and bindings are the same on every platform.

use std::collections::BTreeMap;

use crate::gamepad::GamepadEvent;

/// Which gamepad: the browser's gamepad index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GamepadId(usize);

impl From<GamepadId> for usize {
    fn from(id: GamepadId) -> Self {
        id.0
    }
}

impl From<usize> for GamepadId {
    fn from(index: usize) -> Self {
        Self(index)
    }
}

/// A gamepad button (the same names as gilrs's `Button`).
#[allow(missing_docs)] // the names say it; see gilrs's `Button`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    C,
    Z,
    LeftTrigger,
    LeftTrigger2,
    RightTrigger,
    RightTrigger2,
    Select,
    Start,
    Mode,
    LeftThumb,
    RightThumb,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Unknown,
}

/// A gamepad axis (the same names as gilrs's `Axis`).
#[allow(missing_docs)] // the names say it; see gilrs's `Axis`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    LeftZ,
    RightStickX,
    RightStickY,
    RightZ,
    DPadX,
    DPadY,
    Unknown,
}

/// The Gamepad API's "standard" mapping: button index → button.
const STANDARD_BUTTONS: [GamepadButton; 17] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::West,
    GamepadButton::North,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger,
    GamepadButton::LeftTrigger2,
    GamepadButton::RightTrigger2,
    GamepadButton::Select,
    GamepadButton::Start,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
    GamepadButton::Mode,
];

/// The standard mapping's axes, and whether to flip them (the browser's
/// "down" is positive; gilrs's — and so the engine's — "up" is).
const STANDARD_AXES: [(GamepadAxis, bool); 4] = [
    (GamepadAxis::LeftStickX, false),
    (GamepadAxis::LeftStickY, true),
    (GamepadAxis::RightStickX, false),
    (GamepadAxis::RightStickY, true),
];

/// One pad as the page saw it this frame (`navigator.getGamepads()`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PadSnapshot {
    /// `Gamepad.index`.
    pub index: usize,
    /// `Gamepad.id`, the device's name.
    pub name: String,
    /// `Gamepad.buttons[i].pressed`, in the standard mapping's order.
    pub buttons: Vec<bool>,
    /// `Gamepad.axes`, in the standard mapping's order (browser signs).
    pub axes: Vec<f32>,
}

/// A connected pad's state, in engine terms.
#[derive(Debug, Clone, Default)]
struct Pad {
    name: String,
    pressed: Vec<GamepadButton>,
    axes: Vec<(GamepadAxis, f32)>,
}

/// Gamepad state for the current frame.
#[derive(Debug, Default)]
pub struct Gamepads {
    pads: BTreeMap<GamepadId, Pad>,
    events: Vec<GamepadEvent>,
}

impl Gamepads {
    /// No pads until the page feeds some.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Like [`new`](Self::new); never fails in the browser.
    #[must_use]
    pub fn try_new() -> Option<Self> {
        Some(Self::new())
    }

    /// Nothing to poll here: the page [`feed`](Self::feed)s the pads.
    pub fn update(&mut self) {}

    /// Takes this frame's pads from the page: pads that appeared or went
    /// away, and buttons and axes that changed, become events.
    pub fn feed(&mut self, snapshots: &[PadSnapshot]) {
        let seen: Vec<GamepadId> = snapshots.iter().map(|s| GamepadId(s.index)).collect();
        let gone: Vec<GamepadId> = self
            .pads
            .keys()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect();
        for id in gone {
            self.pads.remove(&id);
            self.events.push(GamepadEvent::Disconnected { id });
        }
        for snapshot in snapshots {
            self.feed_pad(snapshot);
        }
    }

    fn feed_pad(&mut self, snapshot: &PadSnapshot) {
        let id = GamepadId(snapshot.index);
        let next = Pad {
            name: snapshot.name.clone(),
            pressed: STANDARD_BUTTONS
                .iter()
                .zip(&snapshot.buttons)
                .filter(|(_, down)| **down)
                .map(|(button, _)| *button)
                .collect(),
            axes: STANDARD_AXES
                .iter()
                .zip(&snapshot.axes)
                .map(|((axis, flip), value)| (*axis, if *flip { -value } else { *value }))
                .collect(),
        };
        let previous = match self.pads.get(&id) {
            Some(pad) => pad.clone(),
            None => {
                self.events.push(GamepadEvent::Connected {
                    id,
                    name: next.name.clone(),
                });
                Pad::default()
            }
        };
        for &button in &STANDARD_BUTTONS {
            let (was, is) = (
                previous.pressed.contains(&button),
                next.pressed.contains(&button),
            );
            if was != is {
                self.events.push(GamepadEvent::Button {
                    id,
                    button,
                    pressed: is,
                });
            }
        }
        for &(axis, value) in &next.axes {
            let old = previous
                .axes
                .iter()
                .find(|(a, _)| *a == axis)
                .map_or(0.0, |(_, v)| *v);
            if (old - value).abs() > f32::EPSILON {
                self.events.push(GamepadEvent::Axis { id, axis, value });
            }
        }
        self.pads.insert(id, next);
    }

    /// Drain the events collected since the last update.
    pub fn drain_events(&mut self) -> Vec<GamepadEvent> {
        std::mem::take(&mut self.events)
    }

    /// The currently connected gamepad ids.
    pub fn connected_ids(&self) -> impl Iterator<Item = GamepadId> + '_ {
        self.pads.keys().copied()
    }

    /// The first connected gamepad, if any (convenience for single-player).
    #[must_use]
    pub fn primary(&self) -> Option<GamepadId> {
        self.pads.keys().next().copied()
    }

    /// True if any gamepad is connected.
    #[must_use]
    pub fn any_connected(&self) -> bool {
        !self.pads.is_empty()
    }

    /// Human-readable name of a connected gamepad, if any.
    #[must_use]
    pub fn name(&self, id: GamepadId) -> Option<String> {
        self.pads.get(&id).map(|pad| pad.name.clone())
    }

    /// Is `button` pressed on gamepad `id`?
    #[must_use]
    pub fn pressed(&self, id: GamepadId, button: GamepadButton) -> bool {
        self.pads
            .get(&id)
            .is_some_and(|pad| pad.pressed.contains(&button))
    }

    /// Is `button` pressed on *any* connected gamepad?
    #[must_use]
    pub fn pressed_any(&self, button: GamepadButton) -> bool {
        self.pads.values().any(|pad| pad.pressed.contains(&button))
    }

    /// Current value of `axis` on gamepad `id` (0.0 if disconnected).
    #[must_use]
    pub fn axis(&self, id: GamepadId, axis: GamepadAxis) -> f32 {
        self.pads
            .get(&id)
            .and_then(|pad| pad.axes.iter().find(|(a, _)| *a == axis))
            .map_or(0.0, |(_, value)| *value)
    }

    /// Current value of `axis` on the first connected gamepad, if any.
    #[must_use]
    pub fn axis_any(&self, axis: GamepadAxis) -> f32 {
        self.primary().map_or(0.0, |id| self.axis(id, axis))
    }

    /// Left stick position on gamepad `id`.
    #[must_use]
    pub fn left_stick(&self, id: GamepadId) -> (f32, f32) {
        (
            self.axis(id, GamepadAxis::LeftStickX),
            self.axis(id, GamepadAxis::LeftStickY),
        )
    }

    /// Right stick position on gamepad `id`.
    #[must_use]
    pub fn right_stick(&self, id: GamepadId) -> (f32, f32) {
        (
            self.axis(id, GamepadAxis::RightStickX),
            self.axis(id, GamepadAxis::RightStickY),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(buttons: &[usize], axes: [f32; 4]) -> PadSnapshot {
        let mut pressed = vec![false; 17];
        for &b in buttons {
            pressed[b] = true;
        }
        PadSnapshot {
            index: 0,
            name: "Pad".into(),
            buttons: pressed,
            axes: axes.to_vec(),
        }
    }

    #[test]
    fn snapshots_become_state_and_events() {
        let mut pads = Gamepads::new();
        pads.feed(&[pad(&[0], [0.0, -1.0, 0.0, 0.0])]);
        let id = GamepadId::from(0);
        assert_eq!(
            pads.drain_events(),
            vec![
                GamepadEvent::Connected {
                    id,
                    name: "Pad".into()
                },
                GamepadEvent::Button {
                    id,
                    button: GamepadButton::South,
                    pressed: true
                },
                GamepadEvent::Axis {
                    id,
                    axis: GamepadAxis::LeftStickY,
                    value: 1.0
                },
            ]
        );
        assert!(pads.pressed_any(GamepadButton::South));
        assert!(
            (pads.left_stick(id).1 - 1.0).abs() < f32::EPSILON,
            "up is positive"
        );
        pads.feed(&[pad(&[], [0.0, -1.0, 0.0, 0.0])]);
        assert_eq!(
            pads.drain_events(),
            vec![GamepadEvent::Button {
                id,
                button: GamepadButton::South,
                pressed: false
            }]
        );
        pads.feed(&[]);
        assert_eq!(pads.drain_events(), vec![GamepadEvent::Disconnected { id }]);
        assert!(!pads.any_connected());
    }
}
