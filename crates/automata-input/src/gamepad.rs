//! Gamepad support: connection tracking, button/axis state, and events.
//!
//! Wraps [gilrs](https://docs.rs/gilrs). The main loop feeds the
//! [`Gamepads`] once per frame via [`Gamepads::update`]; game code queries
//! buttons/axes or matches on [`GamepadEvent`]. Headless environments simply
//! see no connected gamepads.

use std::collections::HashSet;

pub use gilrs::{Axis as GamepadAxis, Button as GamepadButton, GamepadId};
use gilrs::{EventType, Gilrs};

/// A gamepad event for the engine's event stream.
#[derive(Debug, Clone, PartialEq)]
pub enum GamepadEvent {
    /// A gamepad was connected.
    Connected {
        /// Which gamepad.
        id: GamepadId,
        /// Human-readable name (e.g. "Xbox Wireless Controller").
        name: String,
    },
    /// A gamepad was disconnected.
    Disconnected {
        /// Which gamepad.
        id: GamepadId,
    },
    /// A button changed state.
    Button {
        /// Which gamepad.
        id: GamepadId,
        /// Which button.
        button: GamepadButton,
        /// Whether the button is now pressed.
        pressed: bool,
    },
    /// An axis moved (analog stick or trigger).
    Axis {
        /// Which gamepad.
        id: GamepadId,
        /// Which axis.
        axis: GamepadAxis,
        /// Current value (-1.0..=1.0 for sticks, 0.0..=1.0 for triggers).
        value: f32,
    },
}

/// Gamepad state for the current frame.
///
/// Tracks connected gamepads and provides per-button/axis queries. Events
/// collected since the last [`update`](Self::update) are drained by the
/// engine into the event stream.
pub struct Gamepads {
    gilrs: Gilrs,
    connected: HashSet<GamepadId>,
    events: Vec<GamepadEvent>,
}

impl std::fmt::Debug for Gamepads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gamepads")
            .field("connected", &self.connected.len())
            .field("pending_events", &self.events.len())
            .finish_non_exhaustive()
    }
}

impl Default for Gamepads {
    fn default() -> Self {
        Self::new()
    }
}

impl Gamepads {
    /// Initialise gamepad support. On platforms or sandboxes without
    /// gamepad devices this yields an empty manager (no connected pads).
    ///
    /// # Panics
    ///
    /// Panics only if the platform's gamepad backend fails to initialise
    /// (never observed on Linux/Windows/macOS).
    #[must_use]
    pub fn new() -> Self {
        let gilrs = Gilrs::new().expect("gilrs init");
        let mut this = Self {
            gilrs,
            connected: HashSet::new(),
            events: Vec::new(),
        };
        // Record gamepads that were already connected at startup.
        for (id, _) in this.gilrs.gamepads() {
            this.connected.insert(id);
        }
        this
    }

    /// Drain gilrs' event queue. Called once per frame by the engine.
    pub fn update(&mut self) {
        while let Some(event) = self.gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    self.connected.insert(event.id);
                    let name = self.gilrs.gamepad(event.id).name().to_string();
                    self.events
                        .push(GamepadEvent::Connected { id: event.id, name });
                }
                EventType::Disconnected => {
                    self.connected.remove(&event.id);
                    self.events
                        .push(GamepadEvent::Disconnected { id: event.id });
                }
                EventType::ButtonPressed(button, _) => {
                    self.events.push(GamepadEvent::Button {
                        id: event.id,
                        button,
                        pressed: true,
                    });
                }
                EventType::ButtonReleased(button, _) => {
                    self.events.push(GamepadEvent::Button {
                        id: event.id,
                        button,
                        pressed: false,
                    });
                }
                EventType::AxisChanged(axis, value, _) => {
                    self.events.push(GamepadEvent::Axis {
                        id: event.id,
                        axis,
                        value,
                    });
                }
                _ => {}
            }
        }
    }

    /// Drain the events collected since the last update.
    pub fn drain_events(&mut self) -> Vec<GamepadEvent> {
        std::mem::take(&mut self.events)
    }

    /// The currently connected gamepad ids.
    pub fn connected_ids(&self) -> impl Iterator<Item = GamepadId> + '_ {
        self.connected.iter().copied()
    }

    /// The first connected gamepad, if any (convenience for single-player).
    #[must_use]
    pub fn primary(&self) -> Option<GamepadId> {
        self.connected
            .iter()
            .copied()
            .min_by_key(|id| format!("{id:?}"))
    }

    /// True if any gamepad is connected.
    #[must_use]
    pub fn any_connected(&self) -> bool {
        !self.connected.is_empty()
    }

    /// Human-readable name of a connected gamepad, if any.
    #[must_use]
    pub fn name(&self, id: GamepadId) -> Option<String> {
        self.connected
            .contains(&id)
            .then(|| self.gilrs.gamepad(id).name().to_string())
    }

    /// Is `button` pressed on gamepad `id`?
    #[must_use]
    pub fn pressed(&self, id: GamepadId, button: GamepadButton) -> bool {
        self.connected.contains(&id) && self.gilrs.gamepad(id).is_pressed(button)
    }

    /// Is `button` pressed on *any* connected gamepad?
    #[must_use]
    pub fn pressed_any(&self, button: GamepadButton) -> bool {
        self.connected.iter().any(|&id| self.pressed(id, button))
    }

    /// Current value of `axis` on gamepad `id` (0.0 if disconnected).
    #[must_use]
    pub fn axis(&self, id: GamepadId, axis: GamepadAxis) -> f32 {
        if !self.connected.contains(&id) {
            return 0.0;
        }
        self.gilrs.gamepad(id).value(axis)
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

    #[test]
    fn fresh_manager_queries_are_safe() {
        let gamepads = Gamepads::new();
        // No hardware in CI/sandboxes; queries must still be safe.
        assert!(!gamepads.any_connected() || gamepads.primary().is_some());
        assert!((-1.0..=1.0).contains(&gamepads.axis_any(GamepadAxis::LeftStickX)));
    }

    #[test]
    fn events_start_empty() {
        let mut gamepads = Gamepads::new();
        assert_eq!(gamepads.drain_events().len(), 0);
    }
}
