"""Typed constants for input devices and graphics quality.

Every enum member is a plain ``str`` subclass, so members work anywhere the
engine accepts a name — ``at.Binding.key(at.Key.SPACE)`` and
``at.Binding.key("space")`` are equivalent — while IDEs autocomplete them
and type checkers catch typos before runtime.
"""

from enum import Enum


class Key(str, Enum):
    """A keyboard key, by physical position (layout-independent)."""

    # Letters.
    A = "a"
    B = "b"
    C = "c"
    D = "d"
    E = "e"
    F = "f"
    G = "g"
    H = "h"
    I = "i"
    J = "j"
    K = "k"
    L = "l"
    M = "m"
    N = "n"
    O = "o"
    P = "p"
    Q = "q"
    R = "r"
    S = "s"
    T = "t"
    U = "u"
    V = "v"
    W = "w"
    X = "x"
    Y = "y"
    Z = "z"

    # Digits (top row).
    DIGIT_0 = "0"
    DIGIT_1 = "1"
    DIGIT_2 = "2"
    DIGIT_3 = "3"
    DIGIT_4 = "4"
    DIGIT_5 = "5"
    DIGIT_6 = "6"
    DIGIT_7 = "7"
    DIGIT_8 = "8"
    DIGIT_9 = "9"

    # Navigation and editing.
    ARROW_LEFT = "arrow_left"
    ARROW_RIGHT = "arrow_right"
    ARROW_UP = "arrow_up"
    ARROW_DOWN = "arrow_down"
    HOME = "home"
    END = "end"
    PAGE_UP = "page_up"
    PAGE_DOWN = "page_down"
    INSERT = "insert"
    DELETE = "delete"

    # Whitespace and control.
    SPACE = "space"
    ENTER = "enter"
    ESCAPE = "escape"
    TAB = "tab"
    BACKSPACE = "backspace"

    # Modifiers (physical side variants).
    SHIFT_LEFT = "shift_left"
    SHIFT_RIGHT = "shift_right"
    CTRL_LEFT = "ctrl_left"
    CTRL_RIGHT = "ctrl_right"
    ALT_LEFT = "alt_left"
    ALT_RIGHT = "alt_right"
    SUPER_LEFT = "super_left"
    SUPER_RIGHT = "super_right"

    # Locks.
    CAPS_LOCK = "caps_lock"
    NUM_LOCK = "num_lock"
    SCROLL_LOCK = "scroll_lock"

    # Function keys.
    F1 = "f1"
    F2 = "f2"
    F3 = "f3"
    F4 = "f4"
    F5 = "f5"
    F6 = "f6"
    F7 = "f7"
    F8 = "f8"
    F9 = "f9"
    F10 = "f10"
    F11 = "f11"
    F12 = "f12"

    # Punctuation.
    MINUS = "minus"
    EQUAL = "equal"
    COMMA = "comma"
    PERIOD = "period"
    SLASH = "slash"
    BACKSLASH = "backslash"
    SEMICOLON = "semicolon"
    QUOTE = "quote"
    BACKQUOTE = "backquote"
    BRACKET_LEFT = "bracket_left"
    BRACKET_RIGHT = "bracket_right"


class MouseButton(str, Enum):
    """A mouse button."""

    LEFT = "left"
    RIGHT = "right"
    MIDDLE = "middle"
    BACK = "back"
    FORWARD = "forward"


class GamepadButton(str, Enum):
    """A standard gamepad button (Xbox/PlayStation layout)."""

    SOUTH = "south"  # A / Cross
    EAST = "east"  # B / Circle
    NORTH = "north"  # Y / Triangle
    WEST = "west"  # X / Square
    C = "c"
    Z = "z"
    LEFT_TRIGGER = "left_trigger"  # LB / L1
    LEFT_TRIGGER2 = "left_trigger2"  # LT / L2 (as a button)
    RIGHT_TRIGGER = "right_trigger"  # RB / R1
    RIGHT_TRIGGER2 = "right_trigger2"  # RT / R2 (as a button)
    SELECT = "select"  # Back / Share
    START = "start"  # Start / Options
    MODE = "mode"  # Guide / PS button
    LEFT_THUMB = "left_thumb"  # L3 (stick click)
    RIGHT_THUMB = "right_thumb"  # R3 (stick click)
    DPAD_UP = "dpad_up"
    DPAD_DOWN = "dpad_down"
    DPAD_LEFT = "dpad_left"
    DPAD_RIGHT = "dpad_right"


class GamepadAxis(str, Enum):
    """A standard gamepad analog axis."""

    LEFT_STICK_X = "left_stick_x"
    LEFT_STICK_Y = "left_stick_y"
    LEFT_Z = "left_z"  # LT analog on some pads
    RIGHT_Z = "right_z"  # RT analog on some pads
    RIGHT_STICK_X = "right_stick_x"
    RIGHT_STICK_Y = "right_stick_y"
    DPAD_X = "dpad_x"
    DPAD_Y = "dpad_y"


class GraphicsQuality(str, Enum):
    """Graphics quality preset for ``Game(graphics=...)``.

    ``LOW`` disables shadows; ``MEDIUM``/``HIGH``/``ULTRA`` raise the
    shadow map resolution (1024/2048/4096). Finer-grained controls
    (render scale, MSAA, per-effect toggles) build on this setting.
    """

    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    ULTRA = "ultra"


__all__ = [
    "GamepadAxis",
    "GamepadButton",
    "GraphicsQuality",
    "Key",
    "MouseButton",
]
