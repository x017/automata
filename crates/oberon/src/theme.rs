//! The Oberon visual theme: dark-white sci-fi.
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

    c!(background, 13, 14, 18);
    c!(surface, 22, 24, 30);
    c!(raised, 32, 35, 43);
    c!(hover, 40, 44, 54);
    c!(border, 52, 56, 68);
    c!(text, 232, 234, 240);
    c!(text_dim, 150, 156, 170);
    c!(accent, 96, 156, 255);
    c!(accent_soft, 150, 195, 255);
    c!(select, 43, 70, 110);
    c!(viewport_bg, 9, 10, 13);
    c!(grid, 44, 48, 58);
    c!(axis_x, 224, 96, 100);
    c!(axis_z, 96, 148, 224);
    c!(danger, 224, 96, 100);
    c!(axis_y, 128, 204, 112);
    c!(field, 17, 18, 23);
    c!(warn, 236, 190, 92);
    c!(gizmo_hot, 255, 220, 96);
    c!(light_icon, 255, 214, 120);
    c!(camera_icon, 140, 220, 170);

    /// Translucent black for drop shadows under menus.
    pub fn shadow() -> Color {
        Color::from_rgba8(0, 0, 0, 120)
    }
}

/// Layout metrics, in logical pixels (scaled by the window's DPI factor).
pub mod layout {
    /// Height of the top toolbar.
    pub const TOOLBAR_H: f32 = 44.0;
    /// Width of the hierarchy panel.
    pub const HIERARCHY_W: f32 = 250.0;
    /// Width of the inspector panel.
    pub const INSPECTOR_W: f32 = 340.0;
    /// Height of the console panel.
    pub const CONSOLE_H: f32 = 150.0;
    /// Height of the status bar.
    pub const STATUS_H: f32 = 24.0;
    /// Panel title-bar height.
    pub const TITLEBAR_H: f32 = 28.0;
    /// Standard inner padding.
    pub const PAD: f32 = 10.0;
    /// Corner radius for panels and buttons.
    pub const RADIUS: f32 = 4.0;
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
