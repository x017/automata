//! UI documents: `ui/<name>.ui.ron` screens composed in Oberon's UI editor.
//!
//! The shared script API (`script_api.py`) reads these files and draws them
//! with `ctx.ui` in Play, under `run_scene` and in exported games; scripts
//! reach their widgets by name through `ctx.ui.screen("<name>")`. This
//! module is the editor's side: the data, reading and writing it, and a
//! copy of the script API's layout rules ([`layout`]) so the canvas shows
//! widgets exactly where the game will. A test keeps both copies in step
//! (`tests/fixtures/ui_layout.json`).

use std::path::Path;

use serde::{Deserialize, Serialize};

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NodeKind {
    /// Children top to bottom.
    #[default]
    Column,
    /// Children left to right.
    Row,
    /// Children on top of each other.
    Stack,
    /// Text.
    Label,
    /// A button.
    Button,
    /// A checkbox with its text.
    Toggle,
    /// A horizontal slider.
    Slider,
    /// A progress bar.
    Progress,
    /// An image from the project.
    Image,
    /// Empty room in a layout.
    Space,
}

impl NodeKind {
    /// Every kind, in the editor's Add order.
    pub const ALL: [Self; 10] = [
        Self::Column,
        Self::Row,
        Self::Stack,
        Self::Label,
        Self::Button,
        Self::Toggle,
        Self::Slider,
        Self::Progress,
        Self::Image,
        Self::Space,
    ];

    /// The kind's display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Column => "Column",
            Self::Row => "Row",
            Self::Stack => "Stack",
            Self::Label => "Label",
            Self::Button => "Button",
            Self::Toggle => "Toggle",
            Self::Slider => "Slider",
            Self::Progress => "Progress",
            Self::Image => "Image",
            Self::Space => "Space",
        }
    }

    /// Whether it holds other nodes.
    #[must_use]
    pub fn is_layout(self) -> bool {
        matches!(self, Self::Column | Self::Row | Self::Stack)
    }

    /// Whether `screen.set(name, value)` gives it a value.
    #[must_use]
    pub fn has_value(self) -> bool {
        !matches!(self, Self::Column | Self::Row | Self::Stack | Self::Space)
    }
}

// serde's `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(v: &bool) -> bool {
    !*v
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(v: &bool) -> bool {
    *v
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_ten(v: &f32) -> bool {
    (*v - 10.0).abs() < 1e-6
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_one(v: &f32) -> bool {
    (*v - 1.0).abs() < 1e-6
}
fn is_left(v: &String) -> bool {
    v == "left"
}
fn ten() -> f32 {
    10.0
}
fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}
fn left() -> String {
    "left".into()
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// One widget or layout. Fields that don't apply to its kind are ignored;
/// the file only holds those that differ from the defaults.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNode {
    /// What it is.
    pub kind: NodeKind,
    /// The name scripts use: `ctx.ui.screen("hud").set(name, value)`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// A label's, button's or toggle's text; a progress bar's text
    /// (`{value}`, `{max}`, `{percent}` filled in); an image's path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Top-level nodes only: where on the screen (`"top_left"` …
    /// `"bottom_right"`); empty: at `x, y`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub anchor: String,
    /// Top-level nodes: the position, or the margin from the anchor.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub x: f32,
    /// See [`Self::x`].
    #[serde(default, skip_serializing_if = "is_zero")]
    pub y: f32,
    /// Size in UI units; `None`: the kind's own, or from its layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// See [`Self::width`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    /// Layouts: space between children.
    #[serde(default = "ten", skip_serializing_if = "is_ten")]
    pub gap: f32,
    /// Layouts: space inside the edges.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub padding: f32,
    /// Rows: equal cells (0: each child its own width).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub columns: u32,
    /// Layouts: draw a panel behind the children.
    #[serde(default, skip_serializing_if = "is_false")]
    pub panel: bool,
    /// Columns and rows with a panel: its title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Text size; `None`: the theme's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// A label's text, a progress bar's fill, an image's tint, a panel's
    /// fill (sRGB RGBA, 0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 4]>,
    /// Labels: `"left"`, `"center"` or `"right"`.
    #[serde(default = "left", skip_serializing_if = "is_left")]
    pub align: String,
    /// Labels: a `.ttf` / `.otf` in the project; empty: the theme's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// Sliders and progress bars: the value; toggles: on when not 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub value: f32,
    /// Sliders: the smallest value.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub min: f32,
    /// Sliders and progress bars: the largest value.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub max: f32,
    /// Sliders: values snap to this (0: no snapping).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub step: f32,
    /// Off: dimmed, and it never clicks.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Off: not drawn (scripts can show it with `set_visible`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// Layouts: what's inside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiNode>,
}

impl Default for UiNode {
    fn default() -> Self {
        Self::bare(NodeKind::Column)
    }
}

impl UiNode {
    /// A node of `kind` with every field at its default.
    #[must_use]
    pub fn bare(kind: NodeKind) -> Self {
        Self {
            kind,
            name: String::new(),
            text: String::new(),
            anchor: String::new(),
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
            gap: 10.0,
            padding: 0.0,
            columns: 0,
            panel: false,
            title: String::new(),
            size: None,
            color: None,
            align: left(),
            font: String::new(),
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: 0.0,
            enabled: true,
            visible: true,
            children: Vec::new(),
        }
    }

    /// A fresh node of `kind` as the editor's Add makes it: with text to
    /// see and a value to show.
    #[must_use]
    pub fn new(kind: NodeKind) -> Self {
        let mut n = Self::bare(kind);
        match kind {
            NodeKind::Column => {
                n.width = Some(320.0);
                n.padding = 20.0;
                n.panel = true;
            }
            NodeKind::Row => n.columns = 2,
            NodeKind::Stack | NodeKind::Image => {}
            NodeKind::Label => n.text = "Label".into(),
            NodeKind::Button => n.text = "Button".into(),
            NodeKind::Toggle => {
                n.text = "Toggle".into();
                n.value = 1.0;
            }
            NodeKind::Slider => n.value = 0.5,
            NodeKind::Progress => n.value = 0.6,
            NodeKind::Space => n.height = Some(20.0),
        }
        n
    }

    /// The node at `path` below this one (empty path: itself).
    #[must_use]
    pub fn at(&self, path: &[usize]) -> Option<&Self> {
        match path.split_first() {
            None => Some(self),
            Some((&i, rest)) => self.children.get(i)?.at(rest),
        }
    }

    /// What the tree shows for it: its name, else its text, else its kind.
    #[must_use]
    pub fn title_text(&self) -> String {
        if !self.name.is_empty() {
            format!("{} {}", self.kind.label(), self.name)
        } else if !self.text.is_empty() && self.kind != NodeKind::Image {
            format!("{} \u{201c}{}\u{201d}", self.kind.label(), self.text)
        } else {
            self.kind.label().to_string()
        }
    }
}

/// Theme colours and text the document overrides (the rest are the
/// script API's defaults, [`ThemeColors::default`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiThemeOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Labels and button text.
    pub text: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Disabled text and hints.
    pub text_dim: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Panel backgrounds.
    pub panel: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Buttons at rest.
    pub button: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Buttons under the cursor.
    pub hover: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Buttons held down.
    pub press: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Filled parts: progress, sliders, toggles that are on.
    pub accent: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Text on the accent.
    pub on_accent: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Empty bars, slider tracks and toggle boxes.
    pub track: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The thin edge around buttons and panels.
    pub border: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The keyboard focus ring.
    pub focus: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Text height when a widget has no size.
    pub text_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// A `.ttf` / `.otf` for every widget's text.
    pub font: Option<String>,
}

/// The colours widgets are drawn with: the script API's `UiTheme`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeColors {
    /// Labels and button text.
    pub text: [f32; 4],
    /// Disabled text and hints.
    pub text_dim: [f32; 4],
    /// Panel backgrounds.
    pub panel: [f32; 4],
    /// Buttons at rest.
    pub button: [f32; 4],
    /// Buttons under the cursor.
    pub hover: [f32; 4],
    /// Buttons held down.
    pub press: [f32; 4],
    /// Filled parts: progress, sliders, toggles that are on.
    pub accent: [f32; 4],
    /// Text on the accent.
    pub on_accent: [f32; 4],
    /// Empty bars, slider tracks and toggle boxes.
    pub track: [f32; 4],
    /// The thin edge around buttons and panels.
    pub border: [f32; 4],
    /// The keyboard focus ring.
    pub focus: [f32; 4],
    /// Text height when a widget has no size.
    pub text_size: f32,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            text: [0.95, 0.94, 0.92, 1.0],
            text_dim: [0.6, 0.6, 0.6, 1.0],
            panel: [0.05, 0.05, 0.06, 0.9],
            button: [0.15, 0.15, 0.17, 0.94],
            hover: [0.24, 0.24, 0.27, 0.96],
            press: [0.09, 0.09, 0.1, 0.98],
            accent: [0.95, 0.94, 0.92, 1.0],
            on_accent: [0.03, 0.03, 0.03, 1.0],
            track: [0.0, 0.0, 0.0, 0.6],
            border: [1.0, 1.0, 1.0, 0.16],
            focus: [1.0, 0.78, 0.3, 1.0],
            text_size: 20.0,
        }
    }
}

impl ThemeColors {
    /// The colour in slot `name` ([`THEME_SLOTS`]).
    #[must_use]
    pub fn slot(&self, name: &str) -> Option<[f32; 4]> {
        Some(match name {
            "text" => self.text,
            "text_dim" => self.text_dim,
            "panel" => self.panel,
            "button" => self.button,
            "hover" => self.hover,
            "press" => self.press,
            "accent" => self.accent,
            "on_accent" => self.on_accent,
            "track" => self.track,
            "border" => self.border,
            "focus" => self.focus,
            _ => return None,
        })
    }
}

/// The theme colour slots, for the editor's Theme section.
pub const THEME_SLOTS: [&str; 11] = [
    "text",
    "text_dim",
    "panel",
    "button",
    "hover",
    "press",
    "accent",
    "on_accent",
    "track",
    "border",
    "focus",
];

impl UiThemeOverrides {
    /// The colour slot `name` ([`THEME_SLOTS`]).
    pub fn slot_mut(&mut self, name: &str) -> Option<&mut Option<[f32; 4]>> {
        Some(match name {
            "text" => &mut self.text,
            "text_dim" => &mut self.text_dim,
            "panel" => &mut self.panel,
            "button" => &mut self.button,
            "hover" => &mut self.hover,
            "press" => &mut self.press,
            "accent" => &mut self.accent,
            "on_accent" => &mut self.on_accent,
            "track" => &mut self.track,
            "border" => &mut self.border,
            "focus" => &mut self.focus,
            _ => return None,
        })
    }

    /// The colours with these overrides applied.
    #[must_use]
    pub fn resolve(&self) -> ThemeColors {
        let d = ThemeColors::default();
        ThemeColors {
            text: self.text.unwrap_or(d.text),
            text_dim: self.text_dim.unwrap_or(d.text_dim),
            panel: self.panel.unwrap_or(d.panel),
            button: self.button.unwrap_or(d.button),
            hover: self.hover.unwrap_or(d.hover),
            press: self.press.unwrap_or(d.press),
            accent: self.accent.unwrap_or(d.accent),
            on_accent: self.on_accent.unwrap_or(d.on_accent),
            track: self.track.unwrap_or(d.track),
            border: self.border.unwrap_or(d.border),
            focus: self.focus.unwrap_or(d.focus),
            text_size: self.text_size.unwrap_or(d.text_size),
        }
    }
}

/// A UI document: one screen (or HUD) and its widgets.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiDoc {
    /// The screen size it's laid out for; scaled to fit any screen.
    pub design_size: (f32, f32),
    /// A HUD: shown from the start, under every screen. Otherwise a
    /// screen, shown with `ctx.ui.screens.show(name)`.
    pub hud: bool,
    /// As a screen: blocks what's under it.
    pub modal: bool,
    /// As a screen: sets `ctx.ui.screens.paused` while shown.
    pub pause: bool,
    /// As a screen: Escape / the pad's B closes it.
    pub escape: bool,
    /// Theme overrides.
    #[serde(skip_serializing_if = "is_default_theme")]
    pub theme: UiThemeOverrides,
    /// The top-level nodes, drawn in order.
    pub nodes: Vec<UiNode>,
}

fn is_default_theme(t: &UiThemeOverrides) -> bool {
    *t == UiThemeOverrides::default()
}

impl Default for UiDoc {
    fn default() -> Self {
        Self {
            design_size: (1280.0, 720.0),
            hud: false,
            modal: true,
            pause: false,
            escape: true,
            theme: UiThemeOverrides::default(),
            nodes: Vec::new(),
        }
    }
}

/// The file's first lines: what it is and how scripts use it.
const HEADER: &str = "// A UI screen made in Oberon's UI editor. Scripts reach its widgets by\n\
                      // name: ctx.ui.screen(\"NAME\").set(\"score\", 10)\n";

impl UiDoc {
    /// A new screen: a centred panel with a title and two buttons.
    #[must_use]
    pub fn template_menu() -> Self {
        let mut panel = UiNode::new(NodeKind::Column);
        panel.anchor = "center".into();
        panel.width = Some(360.0);
        panel.padding = 32.0;
        panel.gap = 12.0;
        let mut title = UiNode::new(NodeKind::Label);
        title.text = "MENU".into();
        title.size = Some(32.0);
        title.align = "center".into();
        let mut play = UiNode::new(NodeKind::Button);
        play.name = "play".into();
        play.text = "Play".into();
        play.height = Some(52.0);
        let mut quit = UiNode::new(NodeKind::Button);
        quit.name = "quit".into();
        quit.text = "Quit".into();
        quit.height = Some(52.0);
        panel.children = vec![title, play, quit];
        Self {
            pause: true,
            nodes: vec![panel],
            ..Self::default()
        }
    }

    /// A new HUD: a score and a health bar in the top-left corner.
    #[must_use]
    pub fn template_hud() -> Self {
        let mut col = UiNode::bare(NodeKind::Column);
        col.anchor = "top_left".into();
        col.x = 24.0;
        col.y = 24.0;
        col.width = Some(280.0);
        col.gap = 8.0;
        let mut score = UiNode::new(NodeKind::Label);
        score.name = "score".into();
        score.text = "Score 0".into();
        score.size = Some(24.0);
        let mut health = UiNode::new(NodeKind::Progress);
        health.name = "health".into();
        health.value = 100.0;
        health.max = 100.0;
        health.text = "{value} / {max}".into();
        col.children = vec![score, health];
        Self {
            hud: true,
            modal: false,
            nodes: vec![col],
            ..Self::default()
        }
    }

    /// Reads a document.
    ///
    /// # Errors
    /// The file can't be read or isn't a UI document.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses a document's text.
    ///
    /// # Errors
    /// It isn't a UI document.
    pub fn parse(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    /// The document as the file holds it, with a header naming the screen.
    #[must_use]
    pub fn to_ron(&self, name: &str) -> String {
        let config = ron::ser::PrettyConfig::new()
            .indentor("    ".into())
            .separate_tuple_members(false);
        let body = ron::ser::to_string_pretty(self, config).unwrap_or_default();
        format!("{}{body}\n", HEADER.replace("NAME", name))
    }

    /// Writes the document.
    ///
    /// # Errors
    /// The file can't be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_ron(&screen_name(path)))
    }

    /// The node at `path` (indices from the top level down).
    #[must_use]
    pub fn node(&self, path: &[usize]) -> Option<&UiNode> {
        let (&first, rest) = path.split_first()?;
        self.nodes.get(first)?.at(rest)
    }

    /// The node at `path`, to change.
    pub fn node_mut(&mut self, path: &[usize]) -> Option<&mut UiNode> {
        let (&first, rest) = path.split_first()?;
        let mut node = self.nodes.get_mut(first)?;
        for &i in rest {
            node = node.children.get_mut(i)?;
        }
        Some(node)
    }

    /// The list `path`'s node lives in, and its index there.
    pub fn siblings_mut(&mut self, path: &[usize]) -> Option<(&mut Vec<UiNode>, usize)> {
        let (&last, parent) = path.split_last()?;
        if parent.is_empty() {
            return Some((&mut self.nodes, last));
        }
        Some((&mut self.node_mut(parent)?.children, last))
    }

    /// Every widget name and how often it's used (scripts need them unique).
    #[must_use]
    pub fn names(&self) -> std::collections::BTreeMap<String, usize> {
        fn walk(nodes: &[UiNode], out: &mut std::collections::BTreeMap<String, usize>) {
            for n in nodes {
                if !n.name.is_empty() {
                    *out.entry(n.name.clone()).or_insert(0) += 1;
                }
                walk(&n.children, out);
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(&self.nodes, &mut out);
        out
    }
}

/// Whether `path` is a UI document (`*.ui.ron`).
#[must_use]
pub fn is_ui_doc_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".ui.ron"))
}

/// The screen name scripts use for the document at `path`: its path
/// under the `ui/` folder without `.ui.ron` (`ui/menus/main.ui.ron` →
/// `menus/main`), or just its file name outside one.
#[must_use]
pub fn screen_name(path: &Path) -> String {
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("screen");
    let stem = if file.to_ascii_lowercase().ends_with(".ui.ron") {
        &file[..file.len() - ".ui.ron".len()]
    } else {
        file
    };
    let mut parts = vec![stem.to_string()];
    let mut dir = path.parent();
    while let Some(d) = dir {
        let Some(name) = d.file_name().and_then(|n| n.to_str()) else {
            break;
        };
        if name == "ui" {
            parts.reverse();
            return parts.join("/");
        }
        parts.push(name.to_string());
        dir = d.parent();
    }
    stem.to_string()
}

/// The nine anchors, as fractions of the free space (the script API's
/// `ANCHORS`).
pub const ANCHORS: [(&str, f32, f32); 9] = [
    ("top_left", 0.0, 0.0),
    ("top", 0.5, 0.0),
    ("top_right", 1.0, 0.0),
    ("left", 0.0, 0.5),
    ("center", 0.5, 0.5),
    ("right", 1.0, 0.5),
    ("bottom_left", 0.0, 1.0),
    ("bottom", 0.5, 1.0),
    ("bottom_right", 1.0, 1.0),
];

/// An anchor's fractions, accepting the script API's aliases.
#[must_use]
pub fn anchor_fractions(name: &str) -> Option<(f32, f32)> {
    let key = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    let key = match key.as_str() {
        "centre" | "middle" => "center",
        "top_center" => "top",
        "bottom_center" => "bottom",
        "center_left" => "left",
        "center_right" => "right",
        k => k,
    };
    ANCHORS
        .iter()
        .find(|(n, ..)| *n == key)
        .map(|&(_, fx, fy)| (fx, fy))
}

/// A box in UI units: x, y (top-left), width, height.
pub type UiRect = [f32; 4];

/// Where [`layout`] put one node.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The node's path in the document.
    pub path: Vec<usize>,
    /// What it is.
    pub kind: NodeKind,
    /// Its box in UI units (a label: its slot).
    pub rect: UiRect,
    /// Labels: where the text's anchor point is (x by `align`, y at the
    /// middle of the text), like the `ctx.hud.text` call the game makes.
    pub text_at: Option<(f32, f32)>,
    /// The text size it's drawn with.
    pub text_size: f32,
}

/// The screen a document is laid out on: UI scale and size in UI units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Screen pixels per UI unit.
    pub scale: f32,
    /// The screen in UI units.
    pub size: (f32, f32),
}

impl Frame {
    /// The frame for a `screen` in pixels (`ctx.ui.scale` / `ctx.ui.size`
    /// with the player's UI scale at 1).
    #[must_use]
    pub fn new(doc: &UiDoc, screen: (f32, f32)) -> Self {
        let (dw, dh) = doc.design_size;
        let mut s = 1.0;
        if dw > 0.0 && dh > 0.0 {
            s = (screen.0 / dw).min(screen.1 / dh);
        }
        let s = s.max(0.05);
        Self {
            scale: s,
            size: (screen.0 / s, screen.1 / s),
        }
    }

    /// `ctx.ui.place`: the top-left of a `w`×`h` box at `anchor`.
    #[must_use]
    pub fn place(&self, (fx, fy): (f32, f32), w: f32, h: f32, (mx, my): (f32, f32)) -> (f32, f32) {
        (
            mx + (self.size.0 - 2.0 * mx - w) * fx,
            my + (self.size.1 - 2.0 * my - h) * fy,
        )
    }
}

/// The script API's `_Layout`, step for step.
struct Lay {
    kind: NodeKind,
    x: f32,
    y: f32,
    w: Option<f32>,
    h: Option<f32>,
    gap: f32,
    pad: f32,
    columns: Option<u32>,
    top: f32,
    used_w: f32,
    used_h: f32,
    count: u32,
}

impl Lay {
    fn inner_w(&self) -> Option<f32> {
        self.w.map(|w| (w - 2.0 * self.pad).max(0.0))
    }

    fn inner_h(&self) -> Option<f32> {
        self.h.map(|h| (h - 2.0 * self.pad - self.top).max(0.0))
    }

    fn cell_w(&self) -> Option<f32> {
        let n = self.columns.filter(|&c| c > 0)?;
        #[allow(clippy::cast_precision_loss)]
        let n = n as f32;
        self.inner_w().map(|iw| (iw - self.gap * (n - 1.0)) / n)
    }

    fn peek(&self, w: Option<f32>, h: Option<f32>) -> (f32, f32, Option<f32>, Option<f32>) {
        let gap = if self.count > 0 { self.gap } else { 0.0 };
        let (x0, y0) = (self.x + self.pad, self.y + self.pad + self.top);
        match self.kind {
            NodeKind::Column => (x0, y0 + self.used_h + gap, w.or(self.inner_w()), h),
            NodeKind::Row => (
                x0 + self.used_w + gap,
                y0,
                w.or(self.cell_w()),
                h.or(self.inner_h()),
            ),
            _ => (x0, y0, w.or(self.inner_w()), h.or(self.inner_h())),
        }
    }

    fn next(&mut self, w: Option<f32>, h: Option<f32>, dw: f32, dh: f32) -> UiRect {
        let gap = if self.count > 0 { self.gap } else { 0.0 };
        self.count += 1;
        let (x0, y0) = (self.x + self.pad, self.y + self.pad + self.top);
        match self.kind {
            NodeKind::Column => {
                let cw = w.or(self.inner_w()).unwrap_or(dw);
                let ch = h.unwrap_or(dh);
                let r = [x0, y0 + self.used_h + gap, cw, ch];
                self.used_h += gap + ch;
                self.used_w = self.used_w.max(cw);
                r
            }
            NodeKind::Row => {
                let cw = w.or(self.cell_w()).unwrap_or(dw);
                let ch = h.or(self.inner_h()).unwrap_or(dh);
                let r = [x0 + self.used_w + gap, y0, cw, ch];
                self.used_w += gap + cw;
                self.used_h = self.used_h.max(ch);
                r
            }
            _ => {
                let cw = w.or(self.inner_w()).unwrap_or(dw);
                let ch = h.or(self.inner_h()).unwrap_or(dh);
                self.used_w = self.used_w.max(cw);
                self.used_h = self.used_h.max(ch);
                [x0, y0, cw, ch]
            }
        }
    }

    fn size(&self) -> (f32, f32) {
        (
            self.w.unwrap_or(self.used_w + 2.0 * self.pad),
            self.h.unwrap_or(self.used_h + 2.0 * self.pad + self.top),
        )
    }
}

/// A label's estimated width (the script API can't measure text on every
/// runner, so neither do we).
#[allow(clippy::cast_precision_loss)]
fn estimate(text: &str, size: f32) -> (f32, f32) {
    let lines: Vec<&str> = text.split('\n').collect();
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    (longest as f32 * size * 0.55, size * lines.len() as f32)
}

/// A widget's own size outside a layout, or what it asks a layout for:
/// `(width, height, default width, default height)`.
#[allow(clippy::cast_precision_loss)]
fn wants(node: &UiNode, text_size: f32) -> (Option<f32>, Option<f32>, f32, f32) {
    let size = node.size.unwrap_or(text_size);
    match node.kind {
        NodeKind::Label => {
            let (w, h) = estimate(&node.text, size);
            (None, None, w, h)
        }
        NodeKind::Toggle => {
            let bx = size * 1.2;
            let est = bx + size * 0.5 + node.text.chars().count() as f32 * size * 0.55;
            (None, None, est, bx)
        }
        NodeKind::Button => (node.width, node.height, 160.0, 44.0),
        NodeKind::Slider => (node.width, node.height, 200.0, 24.0),
        NodeKind::Progress => (node.width, node.height, 200.0, 18.0),
        NodeKind::Image => (node.width, node.height, 64.0, 64.0),
        NodeKind::Space | NodeKind::Column | NodeKind::Row | NodeKind::Stack => {
            (node.width, node.height, 0.0, 0.0)
        }
    }
}

fn label_x(node: &UiNode, x: f32, w: f32) -> f32 {
    match node.align.to_ascii_lowercase().as_str() {
        "center" | "centre" => x + w / 2.0,
        "right" => x + w,
        _ => x,
    }
}

/// Lays `doc` out on `frame` the way the script API draws it: every
/// visible node's box in UI units, parents before their children.
#[must_use]
pub fn layout(doc: &UiDoc, frame: &Frame) -> Vec<Placed> {
    let text_size = doc.theme.text_size.unwrap_or(20.0);
    let mut out = Vec::new();
    for (i, node) in doc.nodes.iter().enumerate() {
        if !node.visible {
            continue;
        }
        let anchor = (!node.anchor.is_empty())
            .then(|| anchor_fractions(&node.anchor))
            .flatten();
        let margin = (node.x, node.y);
        if node.kind.is_layout() {
            let start = out.len();
            let (w, h) = lay_out(
                node,
                &[i],
                0.0,
                0.0,
                node.width,
                node.height,
                text_size,
                &mut out,
            );
            if let Some(a) = anchor {
                let (x, y) = frame.place(a, w, h, margin);
                shift(&mut out[start..], x, y);
            } else {
                shift(&mut out[start..], node.x, node.y);
            }
            continue;
        }
        let (ww, wh, dw, dh) = wants(node, text_size);
        let (bw, bh) = (ww.unwrap_or(dw), wh.unwrap_or(dh));
        let (x, y) = match anchor {
            Some(a) => frame.place(a, bw, bh, margin),
            None => margin,
        };
        out.push(widget(node, vec![i], [x, y, bw, bh], text_size));
    }
    out
}

fn shift(placed: &mut [Placed], dx: f32, dy: f32) {
    for p in placed {
        p.rect[0] += dx;
        p.rect[1] += dy;
        if let Some((x, y)) = p.text_at.as_mut() {
            *x += dx;
            *y += dy;
        }
    }
}

fn widget(node: &UiNode, path: Vec<usize>, rect: UiRect, text_size: f32) -> Placed {
    let size = node.size.unwrap_or(text_size);
    let text_at = (node.kind == NodeKind::Label).then(|| {
        let lines = node.text.split('\n').count().max(1);
        #[allow(clippy::cast_precision_loss)]
        let h = size * lines as f32;
        (label_x(node, rect[0], rect[2]), rect[1] + h / 2.0)
    });
    Placed {
        path,
        kind: node.kind,
        rect,
        text_at,
        text_size: size,
    }
}

/// Lays a layout node out with its top-left at `(x, y)`; returns its size.
#[allow(clippy::too_many_arguments)]
fn lay_out(
    node: &UiNode,
    path: &[usize],
    x: f32,
    y: f32,
    w: Option<f32>,
    h: Option<f32>,
    text_size: f32,
    out: &mut Vec<Placed>,
) -> (f32, f32) {
    let me = out.len();
    out.push(Placed {
        path: path.to_vec(),
        kind: node.kind,
        rect: [x, y, 0.0, 0.0],
        text_at: None,
        text_size,
    });
    let titled = node.kind != NodeKind::Stack && !node.title.is_empty();
    let mut lay = Lay {
        kind: node.kind,
        x,
        y,
        w,
        h,
        gap: if node.kind == NodeKind::Stack {
            0.0
        } else {
            node.gap
        },
        pad: node.padding,
        columns: (node.kind == NodeKind::Row && node.columns > 0).then_some(node.columns),
        top: if titled { text_size + 12.0 } else { 0.0 },
        used_w: 0.0,
        used_h: 0.0,
        count: 0,
    };
    for (i, child) in node.children.iter().enumerate() {
        if !child.visible {
            continue;
        }
        let mut cpath = path.to_vec();
        cpath.push(i);
        if child.kind.is_layout() {
            let (cx, cy, cw, ch) = lay.peek(child.width, child.height);
            let (sw, sh) = lay_out(child, &cpath, cx, cy, cw, ch, text_size, out);
            lay.next(Some(sw), Some(sh), sw, sh);
            continue;
        }
        let rect = match child.kind {
            NodeKind::Space => {
                if lay.kind == NodeKind::Row {
                    lay.next(Some(child.width.unwrap_or(0.0)), child.height, 0.0, 0.0)
                } else {
                    lay.next(child.width, Some(child.height.unwrap_or(0.0)), 0.0, 0.0)
                }
            }
            NodeKind::Image if child.text.is_empty() => {
                // The game leaves the room an image would take.
                let (w, h) = (child.width.unwrap_or(64.0), child.height.unwrap_or(64.0));
                lay.next(Some(w), Some(h), 0.0, 0.0)
            }
            _ => {
                let (ww, wh, dw, dh) = wants(child, text_size);
                lay.next(ww, wh, dw, dh)
            }
        };
        out.push(widget(child, cpath, rect, text_size));
    }
    let (sw, sh) = lay.size();
    out[me].rect = [x, y, sw, sh];
    (sw, sh)
}

/// A progress bar's text with `{value}`, `{max}` and `{percent}` filled
/// in (the script API's `_ui_format`).
#[must_use]
pub fn progress_text(text: &str, value: f32, max: f32) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let num = |v: f32| {
        if v.fract() == 0.0 {
            format!("{v:.0}")
        } else {
            format!("{v:.1}")
        }
    };
    let k = if max <= 0.0 {
        0.0
    } else {
        (value / max).clamp(0.0, 1.0)
    };
    text.replace("{value}", &num(value))
        .replace("{max}", &num(max))
        .replace("{percent}", &format!("{:.0}", (k * 100.0).round()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn documents_round_trip_and_leave_defaults_out() {
        let doc = UiDoc::template_menu();
        let text = doc.to_ron("menu");
        assert!(text.starts_with("// A UI screen made in Oberon's UI editor"));
        assert!(text.contains("ctx.ui.screen(\"menu\")"));
        assert!(text.contains("kind: Button"));
        assert!(!text.contains("enabled"), "defaults stay out: {text}");
        assert_eq!(UiDoc::parse(&text).unwrap(), doc);
        // A hand-written minimal file fills in the rest.
        let min = UiDoc::parse("(nodes: [(kind: Label, text: \"Hi\")])").unwrap();
        assert_eq!(min.design_size, (1280.0, 720.0));
        assert!(min.modal && min.escape && !min.hud);
        assert_eq!(min.nodes[0].max, 1.0);
        assert!(UiDoc::parse("(nodes: [(kind: Buton)])").is_err());
    }

    #[test]
    fn screen_names_come_from_the_ui_folder() {
        assert_eq!(screen_name(Path::new("/g/ui/hud.ui.ron")), "hud");
        assert_eq!(
            screen_name(Path::new("/g/ui/menus/Main.ui.ron")),
            "menus/Main"
        );
        assert_eq!(screen_name(Path::new("/g/other/x.ui.ron")), "x");
        assert!(is_ui_doc_path(Path::new("a/HUD.UI.RON")));
        assert!(!is_ui_doc_path(Path::new("a/level.ron")));
    }

    #[test]
    fn anchored_columns_land_like_the_script_api() {
        let doc = UiDoc::template_menu();
        let frame = Frame::new(&doc, (800.0, 600.0));
        assert!((frame.scale - 0.625).abs() < 1e-6);
        assert!((frame.size.1 - 960.0).abs() < 1e-3);
        let placed = layout(&doc, &frame);
        let panel = &placed[0];
        // 360 wide, 32 padding; a 32 px title, then two 52 px buttons.
        let h = 32.0 * 2.0 + 32.0 + 12.0 + 52.0 + 12.0 + 52.0;
        assert_eq!(
            panel.rect,
            [(1280.0 - 360.0) / 2.0, (960.0 - h) / 2.0, 360.0, h]
        );
        let play = &placed[2];
        assert_eq!(play.rect[2], 296.0, "buttons fill the column");
        assert_eq!(placed[1].text_at.unwrap().0, 640.0, "centred title");
    }

    /// What the script API would report for every named widget of the
    /// cross-check fixture: buttons, toggles and sliders by the rect they
    /// take clicks in (pixels), labels by their text's point and size.
    fn fixture_positions(doc: &UiDoc) -> String {
        let mut out = String::from("{\n");
        let screens = [(800.0, 600.0), (1920.0, 1080.0), (720.0, 1280.0)];
        for (si, &(w, h)) in screens.iter().enumerate() {
            let frame = Frame::new(doc, (w, h));
            let s = frame.scale;
            let mut rows = Vec::new();
            for p in layout(doc, &frame) {
                let node = doc.node(&p.path).unwrap();
                if node.name.is_empty() {
                    continue;
                }
                let nums: Vec<f32> = match p.kind {
                    NodeKind::Button | NodeKind::Slider => p.rect.iter().map(|v| v * s).collect(),
                    NodeKind::Toggle => {
                        let [x, y, rw, _] = p.rect;
                        vec![x * s, y * s, rw * s, p.text_size * 1.2 * s]
                    }
                    NodeKind::Label => {
                        let (x, y) = p.text_at.unwrap();
                        vec![x * s, y * s, p.text_size * s]
                    }
                    _ => continue,
                };
                let nums: Vec<String> = nums.iter().map(|v| format!("{v:.2}")).collect();
                rows.push(format!("    \"{}\": [{}]", node.name, nums.join(", ")));
            }
            rows.sort();
            let _ = writeln!(out, "  \"{w:.0}x{h:.0}\": {{");
            out += &rows.join(",\n");
            out += "\n  }";
            out.push_str(if si + 1 < screens.len() { ",\n" } else { "\n" });
        }
        out.push_str("}\n");
        out
    }

    #[test]
    fn layout_matches_the_shared_fixture() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let doc = UiDoc::load(&dir.join("ui_layout.ui.ron")).unwrap();
        let got = fixture_positions(&doc);
        let path = dir.join("ui_layout.json");
        if std::env::var_os("OBERON_UPDATE_FIXTURES").is_some() {
            std::fs::write(&path, &got).unwrap();
        }
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            got, want,
            "the layout moved; if that's intended, rerun with OBERON_UPDATE_FIXTURES=1 \
             and check the script API agrees (test_ui_documents.py)"
        );
        assert!(!got.contains("hidden"), "invisible nodes take no room");
    }

    #[test]
    fn progress_text_fills_in_numbers() {
        assert_eq!(progress_text("{value} / {max}", 80.0, 100.0), "80 / 100");
        assert_eq!(progress_text("{percent}%", 0.333, 1.0), "33%");
        assert_eq!(progress_text("HP {value}", 2.5, 10.0), "HP 2.5");
    }
}
