//! The Oberon visual theme: AUTOMATA black (dark-white sci-fi).
//!
//! One place for every color, size, and spacing the editor uses, so the
//! whole app reads consistently and a re-skin is a single edit.

#![forbid(unsafe_code)]

/// Opaque theme colors. `tiny_skia::Color::from_rgba8` is not `const`, so
/// these are functions rather than associated constants.
pub mod color {
    use tiny_skia::Color;

    macro_rules! c {
        ($name:ident, $r:expr, $g:expr, $b:expr) => {
            /// Theme color.
            pub fn $name() -> Color {
                Color::from_rgba8($r, $g, $b, 255)
            }
        };
    }

    // AUTOMATA black: neutral (untinted) blacks layered by elevation, with
    // the logo's paper white as text and accent. Hue is reserved for
    // meaning: axes, errors, warnings, play state.
    c!(background, 5, 5, 5);
    c!(surface, 12, 12, 12);
    c!(raised, 22, 22, 22);
    c!(hover, 32, 32, 31);
    c!(border, 36, 36, 35);
    c!(text, 242, 240, 235);
    c!(text_dim, 140, 138, 132);
    c!(text_faint, 88, 87, 84);
    c!(accent, 242, 240, 235);
    c!(accent_soft, 214, 211, 204);
    c!(select, 42, 42, 41);
    c!(viewport_bg, 0, 0, 0);
    c!(grid, 38, 38, 37);
    c!(axis_x, 230, 92, 92);
    c!(axis_z, 92, 144, 230);
    c!(danger, 236, 98, 98);
    c!(axis_y, 120, 204, 112);
    c!(field, 20, 20, 20);
    c!(warn, 232, 186, 92);
    c!(success, 128, 208, 150);
    c!(gizmo_hot, 255, 220, 96);
    c!(light_icon, 255, 214, 120);
    c!(camera_icon, 140, 220, 170);
    c!(entity_icon, 200, 197, 190);
    // Text drawn on an accent (paper-white) fill.
    c!(on_accent, 8, 8, 8);

    /// Translucent black for drop shadows under menus.
    #[must_use]
    pub fn shadow() -> Color {
        Color::from_rgba8(0, 0, 0, 120)
    }
}

/// Layout metrics, in logical pixels (scaled by the window's DPI factor).
pub mod layout {
    /// Height of the top toolbar.
    pub const TOOLBAR_H: f32 = 0.0;
    /// Width of the hierarchy panel.
    pub const HIERARCHY_W: f32 = 250.0;
    /// Width of the inspector panel.
    pub const INSPECTOR_W: f32 = 340.0;
    /// Height of the console panel.
    pub const CONSOLE_H: f32 = 150.0;
    /// Default height of the Files panel under the hierarchy.
    pub const FILES_H: f32 = 340.0;
    /// Height of the status bar.
    pub const STATUS_H: f32 = 24.0;
    /// Panel title-bar height.
    pub const TITLEBAR_H: f32 = 28.0;
    /// Standard inner padding.
    pub const PAD: f32 = 10.0;
    /// Corner radius for panels and buttons.
    pub const RADIUS: f32 = 5.0;
    /// Body text size.
    pub const FONT_SIZE: f32 = 13.0;
    /// Small text size (hints, console, ids).
    pub const SMALL_SIZE: f32 = 11.5;
    /// Panel title text size.
    pub const TITLE_SIZE: f32 = 12.0;
    /// List row height (hierarchy, menus).
    pub const ROW_H: f32 = 24.0;
    /// Inspector field height.
    pub const FIELD_H: f32 = 20.0;
    /// Vertical advance per inspector row.
    pub const ROW_STEP: f32 = 25.0;
    /// Width of the inspector property-label column.
    pub const LABEL_W: f32 = 96.0;
    /// Width of popup menus.
    pub const MENU_W: f32 = 210.0;
}
