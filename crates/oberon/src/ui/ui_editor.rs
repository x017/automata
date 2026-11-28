//! The UI editor: a `ui/<name>.ui.ron` document opens as a canvas showing
//! the screen the way the game draws it, its widget tree, and the selected
//! widget's properties, instead of as text.
//!
//! The tab's text stays the source of truth: every change rewrites it as
//! one undoable edit, so Ctrl+Z, the unsaved dot, Ctrl+S, reloading after
//! an outside change and "Edit as Text" all work as they do for scripts.

use std::collections::HashSet;
use std::path::PathBuf;

use tiny_skia::Color;

use super::menus::{MenuItem, UiAction};
use super::{ButtonStyle, CursorHint, EditTarget, LogLevel, Rect, TextEdit, Ui};
use crate::canvas::Canvas;
use crate::code::{Key, Mods};
use crate::theme::{color, layout};
use crate::uidoc::{self, NodeKind, ThemeColors, UiDoc, UiNode, THEME_SLOTS};

/// Screen sizes the canvas can preview.
pub const PREVIEWS: [(f32, f32, &str); 4] = [
    (1280.0, 720.0, "1280 × 720"),
    (1920.0, 1080.0, "1920 × 1080"),
    (800.0, 600.0, "800 × 600"),
    (720.0, 1280.0, "720 × 1280 portrait"),
];

const ALIGNS: [&str; 3] = ["left", "center", "right"];
pub(super) const TOOLBAR_H: f32 = 36.0;
pub(super) const STATUS_H: f32 = 22.0;
pub(super) const ROW_H: f32 = 26.0;
pub(super) const LABEL_W: f32 = 92.0;

/// A screen option's check box: its label, whether it's on, and how to flip it.
type ScreenFlag = (&'static str, bool, fn(&mut UiDoc));

/// The UI editor's state (shared by every UI document tab).
#[derive(Debug, Default)]
pub struct UiEditor {
    /// Documents shown as text instead.
    source: HashSet<PathBuf>,
    /// The last text parsed and what it gave.
    parsed: Option<(String, Result<UiDoc, String>)>,
    /// The selected node (`None`: the screen itself).
    pub selected: Option<Vec<usize>>,
    /// Which document `selected` belongs to.
    selected_for: Option<PathBuf>,
    /// Index into [`PREVIEWS`].
    pub preview: usize,
    /// The language the canvas shows string-table keys in (`None`: the
    /// tables' first language; `Some("")`: the keys themselves).
    pub language: Option<String>,
    drag: Option<NodeDrag>,
    clipboard: Option<UiNode>,
    tree_scroll: f32,
    props_scroll: f32,
}

/// Dragging a top-level node on the canvas: it moves by its margin and is
/// written once, on release.
#[derive(Debug, Clone)]
struct NodeDrag {
    path: Vec<usize>,
    start: (f32, f32),
    from: (f32, f32),
    /// Canvas pixels per UI unit.
    k: f32,
    /// How the margin moves the node: +1, -1 or 0 per axis (by anchor).
    sign: (f32, f32),
    moved: bool,
}

impl NodeDrag {
    fn margin(&self, p: (f32, f32)) -> (f32, f32) {
        let dx = ((p.0 - self.start.0) / self.k).round();
        let dy = ((p.1 - self.start.1) / self.k).round();
        (
            self.from.0 + dx * self.sign.0,
            self.from.1 + dy * self.sign.1,
        )
    }
}

/// A property typed into a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiProp {
    /// The widget's name (what scripts use).
    Name,
    /// Label / button text, a progress bar's format, an image's file.
    Text,
    /// Margin from the anchor (or position), x.
    X,
    /// Margin from the anchor (or position), y.
    Y,
    /// Width (empty: automatic).
    Width,
    /// Height (empty: automatic).
    Height,
    /// Space between a layout's children.
    Gap,
    /// Space inside a layout's edge.
    Padding,
    /// A row's equal columns.
    Columns,
    /// A layout's panel title.
    Title,
    /// Text size (empty: the theme's).
    Size,
    /// A label's colour (empty: the theme's).
    Color,
    /// A label's font file.
    Font,
    /// Toggle / slider / progress value.
    Value,
    /// Slider / progress minimum.
    Min,
    /// Slider / progress maximum.
    Max,
    /// Slider step (0: smooth).
    Step,
    /// The document's design width.
    DesignW,
    /// The document's design height.
    DesignH,
    /// The theme's text size.
    TextSize,
    /// The theme's font.
    ThemeFont,
    /// A theme colour ([`THEME_SLOTS`] index).
    Theme(usize),
}

/// A UI editor command (from its menus and toolbar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiDocAction {
    /// Add a node: into the selected layout, after the selected widget, or
    /// at the top level.
    Add(NodeKind),
    /// Set the selected node's anchor (0: none, else `ANCHORS[i - 1]`).
    Anchor(usize),
    /// Set the selected label's alignment.
    Align(usize),
    /// Preview on `PREVIEWS[i]`.
    Preview(usize),
    /// Show string-table keys as written (0) or in language `i - 1`.
    Language(usize),
}

/// One row of the widget tree.
struct TreeRow {
    path: Vec<usize>,
    depth: usize,
    title: String,
    shown: bool,
    kind: NodeKind,
}

fn tree_rows(doc: &UiDoc) -> Vec<TreeRow> {
    fn walk(nodes: &[UiNode], path: &[usize], shown: bool, out: &mut Vec<TreeRow>) {
        for (i, n) in nodes.iter().enumerate() {
            let mut p = path.to_vec();
            p.push(i);
            let shown = shown && n.visible;
            out.push(TreeRow {
                path: p.clone(),
                depth: path.len(),
                title: n.title_text(),
                shown,
                kind: n.kind,
            });
            walk(&n.children, &p, shown, out);
        }
    }
    let mut out = Vec::new();
    walk(&doc.nodes, &[], true, &mut out);
    out
}

pub(super) fn rgba(c: [f32; 4]) -> Color {
    let f = |v: f32| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    Color::from_rgba(f(c[0]), f(c[1]), f(c[2]), f(c[3])).unwrap_or(Color::BLACK)
}

fn dim(c: [f32; 4]) -> [f32; 4] {
    [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, c[3]]
}

/// `#rrggbb` (with alpha when it isn't opaque).
#[must_use]
pub fn hex(c: [f32; 4]) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c[3] < 0.999 {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            b(c[0]),
            b(c[1]),
            b(c[2]),
            b(c[3])
        )
    } else {
        format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
    }
}

/// A number without trailing zeros.
#[must_use]
pub fn num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// Gives every name in `node` (and below) a `_2`, `_3`… suffix where `doc`
/// already uses it, so a pasted copy doesn't clash.
fn rename_clashes(doc: &UiDoc, node: &mut UiNode) {
    fn walk(n: &mut UiNode, used: &mut std::collections::BTreeMap<String, usize>) {
        if !n.name.is_empty() && used.contains_key(&n.name) {
            let base = n.name.trim_end_matches(|c: char| c.is_ascii_digit());
            let base = base.strip_suffix('_').unwrap_or(base).to_string();
            let mut i = 2;
            while used.contains_key(&format!("{base}_{i}")) {
                i += 1;
            }
            n.name = format!("{base}_{i}");
        }
        if !n.name.is_empty() {
            used.insert(n.name.clone(), 1);
        }
        for c in &mut n.children {
            walk(c, used);
        }
    }
    let mut used = doc.names();
    walk(node, &mut used);
}

/// Draws one laid-out node the way the script API does.
#[allow(clippy::too_many_lines)]
fn draw_node(
    canvas: &mut Canvas,
    p: &uidoc::Placed,
    node: &UiNode,
    theme: &ThemeColors,
    origin: (f32, f32),
    k: f32,
) {
    let [x, y, w, h] = p.rect;
    let (x, y, w, h) = (origin.0 + x * k, origin.1 + y * k, w * k, h * k);
    let boxed = |canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, fill: [f32; 4]| {
        canvas.fill_rect(x, y, w, h, rgba(theme.border));
        canvas.fill_rect(
            x + k,
            y + k,
            (w - 2.0 * k).max(0.0),
            (h - 2.0 * k).max(0.0),
            rgba(fill),
        );
    };
    let center_text =
        |canvas: &mut Canvas, cx: f32, cy: f32, size: f32, text: &str, c: [f32; 4]| {
            let tw = canvas.measure_text(size, text);
            canvas.text(cx - tw / 2.0, cy - size * 0.62, size, text, rgba(c));
        };
    let text_c = if node.enabled {
        theme.text
    } else {
        theme.text_dim
    };
    match p.kind {
        NodeKind::Column | NodeKind::Row | NodeKind::Stack => {
            if node.panel {
                boxed(canvas, x, y, w, h, theme.panel);
            }
            if node.kind != NodeKind::Stack && !node.title.is_empty() {
                let size = theme.text_size * k;
                canvas.text(
                    x + 14.0 * k,
                    y + 10.0 * k - size * 0.12,
                    size,
                    &node.title,
                    rgba(theme.text_dim),
                );
            }
        }
        NodeKind::Label => {
            let size = p.text_size * k;
            let (ax, ay) = p.text_at.unwrap_or((p.rect[0], p.rect[1]));
            let (ax, ay) = (origin.0 + ax * k, origin.1 + ay * k);
            let lines: Vec<&str> = node.text.split('\n').collect();
            #[allow(clippy::cast_precision_loss)]
            let top = ay - size * lines.len() as f32 / 2.0;
            let c = node.color.unwrap_or(theme.text);
            for (i, line) in lines.iter().enumerate() {
                let tw = canvas.measure_text(size, line);
                let lx = match node.align.as_str() {
                    "center" => ax - tw / 2.0,
                    "right" => ax - tw,
                    _ => ax,
                };
                #[allow(clippy::cast_precision_loss)]
                canvas.text(lx, top + i as f32 * size - size * 0.12, size, line, rgba(c));
            }
        }
        NodeKind::Button => {
            let fill = if node.enabled {
                theme.button
            } else {
                dim(theme.button)
            };
            boxed(canvas, x, y, w, h, fill);
            let size = node
                .size
                .unwrap_or_else(|| theme.text_size.min(p.rect[3] * 0.5))
                * k;
            center_text(canvas, x + w / 2.0, y + h / 2.0, size, &node.text, text_c);
        }
        NodeKind::Toggle => {
            let size = p.text_size * k;
            let b = size * 1.2;
            canvas.fill_rect(x, y, b, b, rgba(theme.border));
            canvas.fill_rect(x + k, y + k, b - 2.0 * k, b - 2.0 * k, rgba(theme.track));
            if node.value > 0.5 {
                let inner = b * 0.6;
                let accent = if node.enabled {
                    theme.accent
                } else {
                    dim(theme.accent)
                };
                canvas.fill_rect(
                    x + (b - inner) / 2.0,
                    y + (b - inner) / 2.0,
                    inner,
                    inner,
                    rgba(accent),
                );
            }
            canvas.text(
                x + b + size * 0.5,
                y + b / 2.0 - size * 0.62,
                size,
                &node.text,
                rgba(text_c),
            );
        }
        NodeKind::Slider => {
            let span = node.max - node.min;
            let v = node
                .value
                .clamp(node.min.min(node.max), node.max.max(node.min));
            let t = if span.abs() > f32::EPSILON {
                (v - node.min) / span
            } else {
                0.0
            };
            let knob = h * 0.6;
            let cy = y + h / 2.0;
            let track = h / 4.0;
            canvas.fill_rect(x, cy - track / 2.0, w, track, rgba(theme.track));
            let accent = if node.enabled {
                theme.accent
            } else {
                dim(theme.accent)
            };
            let fill = (w - knob) * t + knob / 2.0;
            canvas.fill_rect(x, cy - track / 2.0, fill, track, rgba(accent));
            let kx = x + (w - knob) * t;
            canvas.fill_rect(kx, y, knob, h, rgba(theme.border));
            canvas.fill_rect(kx + k, y + k, knob - 2.0 * k, h - 2.0 * k, rgba(accent));
        }
        NodeKind::Progress => {
            boxed(canvas, x, y, w, h, theme.track);
            let t = if node.max > 0.0 {
                (node.value / node.max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if t > 0.0 {
                let fw = (w - 4.0 * k) * t;
                canvas.fill_rect(
                    x + 2.0 * k,
                    y + 2.0 * k,
                    fw,
                    h - 4.0 * k,
                    rgba(node.color.unwrap_or(theme.accent)),
                );
            }
            let text = uidoc::progress_text(&node.text, node.value, node.max);
            if !text.is_empty() {
                let size = node
                    .size
                    .unwrap_or_else(|| theme.text_size.min(p.rect[3] * 0.75))
                    * k;
                center_text(
                    canvas,
                    x + w / 2.0 + k,
                    y + h / 2.0 + k,
                    size,
                    &text,
                    [0.0, 0.0, 0.0, 0.85],
                );
                center_text(canvas, x + w / 2.0, y + h / 2.0, size, &text, theme.text);
            }
        }
        NodeKind::Image => {
            let c = rgba([theme.border[0], theme.border[1], theme.border[2], 0.9]);
            canvas.fill_rect(x, y, w, h, rgba([0.0, 0.0, 0.0, 0.25]));
            canvas.stroke_rect(x, y, w, h, c, 1.0);
            canvas.line(x, y, x + w, y + h, c, 1.0);
            canvas.line(x + w, y, x, y + h, c, 1.0);
            if !node.text.is_empty() {
                let name = node.text.rsplit(['/', '\\']).next().unwrap_or(&node.text);
                canvas.text_clipped(
                    x + 4.0,
                    y + 4.0,
                    layout::SMALL_SIZE,
                    name,
                    rgba(theme.text),
                    w - 8.0,
                );
            }
        }
        NodeKind::Space => {}
    }
}

impl Ui {
    /// Whether the active tab is a UI document shown in the UI editor.
    #[must_use]
    pub fn ui_editor_shown(&self) -> bool {
        self.code
            .current()
            .is_some_and(|t| uidoc::is_ui_doc_path(&t.path) && !self.uied.source.contains(&t.path))
    }

    /// The UI document the editor shows, as last parsed (None while it
    /// doesn't parse or no UI tab is visible).
    #[must_use]
    pub fn ui_editor_doc(&self) -> Option<&UiDoc> {
        if !self.ui_editor_shown() {
            return None;
        }
        match &self.uied.parsed {
            Some((_, Ok(doc))) => Some(doc),
            _ => None,
        }
    }

    /// Selects a node of the visible UI document (`None`: the screen).
    pub fn select_ui_node(&mut self, path: Option<Vec<usize>>) {
        self.uied.selected = path;
    }

    /// Previews the UI document on `PREVIEWS[i]`.
    pub fn set_ui_preview(&mut self, i: usize) {
        self.ui_doc_action(UiDocAction::Preview(i));
    }

    /// Shows string-table keys on the canvas in `language` (`""`: the keys
    /// as written; `None`: the tables' first language).
    pub fn set_ui_language(&mut self, language: Option<&str>) {
        self.uied.language = language.map(str::to_string);
    }

    /// The language the canvas shows keys in (`None`: the keys).
    #[must_use]
    pub fn ui_language(&self) -> Option<String> {
        self.ui_preview_language()
    }

    /// Shows the active UI document as text (`true`) or in the editor.
    pub fn set_ui_source(&mut self, text: bool) {
        let Some(path) = self.code.current().map(|t| t.path.clone()) else {
            return;
        };
        if text {
            self.uied.source.insert(path);
        } else {
            self.uied.source.remove(&path);
        }
        self.uied.drag = None;
    }

    /// Re-parses the tab's text when it changed and keeps the selection
    /// valid.
    fn refresh_ui_doc(&mut self, idx: usize) {
        let tab = &self.code.tabs[idx];
        let text = tab.buf.text();
        if self.uied.parsed.as_ref().map(|(t, _)| t) != Some(&text) {
            let doc = UiDoc::parse(&text);
            self.uied.parsed = Some((text, doc));
        }
        if self.uied.selected_for.as_ref() != Some(&tab.path) {
            self.uied.selected_for = Some(tab.path.clone());
            self.uied.selected = None;
            self.uied.tree_scroll = 0.0;
            self.uied.props_scroll = 0.0;
        }
        let valid = match (&self.uied.parsed, &self.uied.selected) {
            (Some((_, Ok(doc))), Some(sel)) => doc.node(sel).is_some(),
            _ => true,
        };
        if !valid {
            self.uied.selected = None;
        }
    }

    /// Changes the visible UI document: `f` edits a copy (and may move the
    /// selection); a real change rewrites the tab's text as one undo step.
    pub fn edit_ui_doc(&mut self, f: impl FnOnce(&mut UiDoc, &mut Option<Vec<usize>>)) {
        let Some(idx) = self.code.active.filter(|_| self.ui_editor_shown()) else {
            return;
        };
        self.refresh_ui_doc(idx);
        let Some((_, Ok(before))) = &self.uied.parsed else {
            return;
        };
        let mut doc = before.clone();
        let mut sel = self.uied.selected.clone();
        f(&mut doc, &mut sel);
        self.uied.selected = sel;
        if doc == *before {
            return;
        }
        let tab = &mut self.code.tabs[idx];
        let text = doc.to_ron(&uidoc::screen_name(&tab.path));
        tab.buf.set_text(&text);
        self.uied.parsed = Some((text, Ok(doc)));
    }

    /// Runs a UI editor menu command.
    pub(super) fn ui_doc_action(&mut self, action: UiDocAction) {
        match action {
            UiDocAction::Add(kind) => self.add_ui_node(UiNode::new(kind)),
            UiDocAction::Anchor(i) => {
                let anchor = i
                    .checked_sub(1)
                    .and_then(|i| uidoc::ANCHORS.get(i))
                    .map_or("", |a| a.0)
                    .to_string();
                self.edit_ui_doc(|doc, sel| {
                    if let Some(n) = sel.as_deref().and_then(|p| doc.node_mut(p)) {
                        n.anchor = anchor;
                    }
                });
            }
            UiDocAction::Align(i) => {
                let align = ALIGNS.get(i).copied().unwrap_or("left").to_string();
                self.edit_ui_doc(|doc, sel| {
                    if let Some(n) = sel.as_deref().and_then(|p| doc.node_mut(p)) {
                        n.align = align;
                    }
                });
            }
            UiDocAction::Preview(i) => self.uied.preview = i.min(PREVIEWS.len() - 1),
            UiDocAction::Language(i) => {
                self.uied.language = Some(
                    i.checked_sub(1)
                        .and_then(|l| self.strings.table.languages.get(l).cloned())
                        .unwrap_or_default(),
                );
            }
        }
    }

    /// Adds `node` into the selected layout, after the selected widget, or
    /// (nothing selected) at the top level, centred; selects it.
    fn add_ui_node(&mut self, mut node: UiNode) {
        self.edit_ui_doc(|doc, sel| {
            rename_clashes(doc, &mut node);
            let into_layout = sel
                .as_deref()
                .and_then(|p| doc.node(p))
                .is_some_and(|n| n.kind.is_layout());
            let path = match sel.clone() {
                Some(p) if into_layout => {
                    let parent = doc.node_mut(&p).expect("checked");
                    parent.children.push(node);
                    let mut p = p;
                    p.push(parent.children.len() - 1);
                    p
                }
                Some(p) => {
                    if p.len() == 1 && node.anchor.is_empty() {
                        node.anchor = "center".into();
                    }
                    let Some((list, i)) = doc.siblings_mut(&p) else {
                        return;
                    };
                    list.insert(i + 1, node);
                    let mut p = p;
                    *p.last_mut().expect("non-empty") += 1;
                    p
                }
                None => {
                    if node.anchor.is_empty() {
                        node.anchor = "center".into();
                    }
                    doc.nodes.push(node);
                    vec![doc.nodes.len() - 1]
                }
            };
            *sel = Some(path);
        });
    }

    fn delete_ui_node(&mut self) {
        self.edit_ui_doc(|doc, sel| {
            let Some(p) = sel.clone() else {
                return;
            };
            let Some((list, i)) = doc.siblings_mut(&p) else {
                return;
            };
            list.remove(i);
            // Select the next sibling, else the previous, else the parent.
            *sel = if i < list.len() {
                Some(p)
            } else if i > 0 {
                let mut p = p;
                *p.last_mut().expect("non-empty") -= 1;
                Some(p)
            } else {
                Some(p[..p.len() - 1].to_vec()).filter(|p| !p.is_empty())
            };
        });
    }

    fn move_ui_node(&mut self, down: bool) {
        self.edit_ui_doc(|doc, sel| {
            let Some(p) = sel.clone() else {
                return;
            };
            let Some((list, i)) = doc.siblings_mut(&p) else {
                return;
            };
            let j = if down { i + 1 } else { i.wrapping_sub(1) };
            if j >= list.len() {
                return;
            }
            list.swap(i, j);
            let mut p = p;
            *p.last_mut().expect("non-empty") = j;
            *sel = Some(p);
        });
    }

    fn selected_ui_node(&self) -> Option<UiNode> {
        let doc = self.ui_editor_doc()?;
        doc.node(self.uied.selected.as_deref()?).cloned()
    }

    /// Moves a top-level node by its margin (arrow keys).
    fn nudge_ui_node(&mut self, dx: f32, dy: f32) {
        self.edit_ui_doc(|doc, sel| {
            let Some(p) = sel.as_deref().filter(|p| p.len() == 1) else {
                return;
            };
            let Some(n) = doc.node_mut(p) else {
                return;
            };
            let (sx, sy) = margin_signs(&n.anchor);
            n.x += dx * sx;
            n.y += dy * sy;
        });
    }

    /// A key while the UI editor has the keyboard; false for keys it
    /// leaves to the app (Ctrl+P, F1, …).
    pub(super) fn ui_editor_key(&mut self, key: Key, m: Mods) -> bool {
        let step = if m.shift { 10.0 } else { 1.0 };
        match (key, m.ctrl) {
            // Saving, undo and closing work on the file's text.
            (Key::Char('s' | 'z' | 'y' | 'w'), true) => self.code.key(key, m),
            (Key::Char('d'), true) => {
                if let Some(n) = self.selected_ui_node() {
                    if self.uied.selected.as_ref().is_some_and(|p| !p.is_empty()) {
                        self.add_ui_node(n);
                    }
                }
                true
            }
            (Key::Char('c' | 'x'), true) => {
                if let Some(n) = self.selected_ui_node() {
                    self.uied.clipboard = Some(n);
                    if key == Key::Char('x') {
                        self.delete_ui_node();
                    }
                }
                true
            }
            (Key::Char('v'), true) => {
                if let Some(n) = self.uied.clipboard.clone() {
                    self.add_ui_node(n);
                }
                true
            }
            (Key::Delete | Key::Backspace, false) => {
                self.delete_ui_node();
                true
            }
            (Key::Escape, false) => {
                self.uied.selected = None;
                true
            }
            (Key::Up | Key::Down, false) if m.alt => {
                self.move_ui_node(key == Key::Down);
                true
            }
            (Key::Left, false) => {
                self.nudge_ui_node(-step, 0.0);
                true
            }
            (Key::Right, false) => {
                self.nudge_ui_node(step, 0.0);
                true
            }
            (Key::Up, false) => {
                self.nudge_ui_node(0.0, -step);
                true
            }
            (Key::Down, false) => {
                self.nudge_ui_node(0.0, step);
                true
            }
            (Key::Tab, false) => {
                self.cycle_ui_selection(!m.shift);
                true
            }
            (_, true) => false,
            _ => !matches!(key, Key::F3 | Key::F4 | Key::F12),
        }
    }

    /// Tab / Shift+Tab: the next or previous widget in the tree.
    fn cycle_ui_selection(&mut self, forward: bool) {
        let Some(doc) = self.ui_editor_doc() else {
            return;
        };
        let rows = tree_rows(doc);
        if rows.is_empty() {
            return;
        }
        let at = self
            .uied
            .selected
            .as_ref()
            .and_then(|s| rows.iter().position(|r| &r.path == s));
        let n = rows.len();
        let i = match (at, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        self.uied.selected = Some(rows[i].path.clone());
    }

    /// Applies a typed property.
    pub(super) fn commit_ui_prop(&mut self, prop: UiProp, text: &str) {
        let raw = text.to_string();
        let trimmed = text.trim().to_string();
        let number = super::eval_number(&trimmed);
        let wants_number = !matches!(
            prop,
            UiProp::Name | UiProp::Text | UiProp::Title | UiProp::Font | UiProp::ThemeFont
        ) && !matches!(prop, UiProp::Color | UiProp::Theme(_));
        let optional = matches!(
            prop,
            UiProp::Width | UiProp::Height | UiProp::Size | UiProp::TextSize
        );
        if wants_number && number.is_none() && !(optional && trimmed.is_empty()) {
            self.log(
                LogLevel::Warn,
                format!("\u{201c}{trimmed}\u{201d} isn't a number"),
            );
            return;
        }
        let colour = match prop {
            UiProp::Color | UiProp::Theme(_) if !trimmed.is_empty() => {
                let Some(c) = super::picker::parse_hex(&trimmed) else {
                    self.log(
                        LogLevel::Warn,
                        format!("\u{201c}{trimmed}\u{201d} isn't a colour: try #ff8800"),
                    );
                    return;
                };
                Some(c)
            }
            _ => None,
        };
        let v = number.unwrap_or(0.0);
        let opt = number.filter(|v| *v > 0.0);
        self.edit_ui_doc(|doc, sel| {
            match prop {
                UiProp::DesignW => doc.design_size.0 = v.max(1.0),
                UiProp::DesignH => doc.design_size.1 = v.max(1.0),
                UiProp::TextSize => doc.theme.text_size = opt,
                UiProp::ThemeFont => {
                    doc.theme.font = Some(trimmed.clone()).filter(|f| !f.is_empty());
                }
                UiProp::Theme(i) => {
                    if let Some(slot) = THEME_SLOTS.get(i).and_then(|s| doc.theme.slot_mut(s)) {
                        *slot = colour;
                    }
                }
                _ => {}
            }
            let Some(n) = sel.as_deref().and_then(|p| doc.node_mut(p)) else {
                return;
            };
            match prop {
                UiProp::Name => n.name.clone_from(&trimmed),
                UiProp::Text => n.text = raw.replace("\\n", "\n"),
                UiProp::Title => n.title.clone_from(&raw),
                UiProp::Font => n.font.clone_from(&trimmed),
                UiProp::X => n.x = v,
                UiProp::Y => n.y = v,
                UiProp::Width => n.width = opt,
                UiProp::Height => n.height = opt,
                UiProp::Gap => n.gap = v.max(0.0),
                UiProp::Padding => n.padding = v.max(0.0),
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                UiProp::Columns => n.columns = v.clamp(0.0, 64.0).round() as u32,
                UiProp::Size => n.size = opt,
                UiProp::Color => n.color = colour,
                UiProp::Value => n.value = v,
                UiProp::Min => n.min = v,
                UiProp::Max => n.max = v,
                UiProp::Step => n.step = v.max(0.0),
                UiProp::DesignW
                | UiProp::DesignH
                | UiProp::TextSize
                | UiProp::ThemeFont
                | UiProp::Theme(_) => {}
            }
        });
    }

    // ----- drawing ---------------------------------------------------------

    /// The UI editor in the center area.
    pub(super) fn ui_doc_view(&mut self, canvas: &mut Canvas, rect: Rect, idx: usize) {
        self.code.check_disk();
        self.drain_code_events();
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::background());
        if self.p.pressed {
            self.code.focused = rect.contains(self.p.x, self.p.y);
        }
        if idx >= self.code.tabs.len() {
            return;
        }
        self.refresh_ui_doc(idx);
        let path = self.code.tabs[idx].path.clone();
        let parsed = self.uied.parsed.as_ref().map(|(_, d)| d.clone());
        let toolbar = Rect::new(rect.x, rect.y, rect.w, TOOLBAR_H);
        let status = Rect::new(rect.x, rect.y + rect.h - STATUS_H, rect.w, STATUS_H);
        let body = Rect::new(
            rect.x,
            toolbar.y + toolbar.h,
            rect.w,
            (status.y - toolbar.y - toolbar.h).max(0.0),
        );
        let screen = uidoc::screen_name(&path);
        match parsed {
            Some(Ok(doc)) => {
                self.ui_toolbar(canvas, toolbar, &doc, &screen);
                let (tw, pw) = if body.w < 980.0 {
                    (170.0, 250.0)
                } else {
                    (210.0, 280.0)
                };
                let tree = Rect::new(body.x, body.y, tw, body.h);
                let props = Rect::new(body.x + body.w - pw, body.y, pw, body.h);
                let area = Rect::new(tree.x + tw, body.y, (body.w - tw - pw).max(0.0), body.h);
                self.ui_canvas(canvas, area, &doc);
                self.ui_tree(canvas, tree, &doc);
                self.ui_props(canvas, props, &doc, &screen);
                self.ui_status(canvas, status, &doc, &screen, idx);
            }
            Some(Err(e)) => {
                self.ui_broken(canvas, body, &e);
                canvas.fill_rect(status.x, status.y, status.w, status.h, color::surface());
            }
            None => {}
        }
    }

    fn ui_toolbar(&mut self, canvas: &mut Canvas, r: Rect, doc: &UiDoc, screen: &str) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, color::border());
        let y = r.y + 5.0;
        let h = r.h - 10.0;
        let mut x = r.x + 8.0;
        let add = Rect::new(x, y, 80.0, h);
        let clicked = self.button(canvas, add, "+ Add     ", ButtonStyle::Accent);
        menu_caret(canvas, add, color::on_accent());
        if clicked {
            let items = NodeKind::ALL
                .iter()
                .map(|&k| MenuItem::ui(k.label(), "", UiAction::UiDoc(UiDocAction::Add(k)), true))
                .collect();
            self.open_menu(add.x, add.y + add.h + 2.0, items);
        }
        x += add.w + 6.0;
        let has = self.uied.selected.is_some();
        let style = if has {
            ButtonStyle::Normal
        } else {
            ButtonStyle::Disabled
        };
        for (label, w) in [
            ("Duplicate", 78.0),
            ("Delete", 62.0),
            ("Up", 36.0),
            ("Down", 48.0),
        ] {
            let b = Rect::new(x, y, w, h);
            if self.button(canvas, b, label, style) {
                match label {
                    "Duplicate" => {
                        if let Some(n) = self.selected_ui_node() {
                            self.add_ui_node(n);
                        }
                    }
                    "Delete" => self.delete_ui_node(),
                    "Up" => self.move_ui_node(false),
                    _ => self.move_ui_node(true),
                }
            }
            x += w + 4.0;
        }
        x += 10.0;
        let preview = PREVIEWS[self.uied.preview.min(PREVIEWS.len() - 1)];
        let label = format!("Preview {}", preview.2);
        let pwid = canvas.measure_text(layout::FONT_SIZE, &label) + 40.0;
        let pb = Rect::new(x, y, pwid, h);
        let clicked = self.button(canvas, pb, &format!("{label}     "), ButtonStyle::Ghost);
        menu_caret(canvas, pb, color::text_dim());
        if clicked {
            let items = PREVIEWS
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mark = if i == self.uied.preview {
                        "✓ "
                    } else {
                        "   "
                    };
                    MenuItem::ui(
                        format!("{mark}{}", p.2),
                        "",
                        UiAction::UiDoc(UiDocAction::Preview(i)),
                        true,
                    )
                })
                .collect();
            self.open_menu(pb.x, pb.y + pb.h + 2.0, items);
        }
        // The language keys show in, when the project has string tables.
        let mut left_end = pb.x + pb.w;
        if !self.strings.table.languages.is_empty() {
            let shown = self.ui_preview_language();
            // Language names in their own script: Persian or Arabic ones
            // are joined and put in drawing order.
            let name = match &shown {
                Some(l) => automata_renderer::bidi::visual(self.strings.table.name(l)),
                None => "Keys".to_string(),
            };
            let label = format!("Text {name}");
            let lw = canvas.measure_text(layout::FONT_SIZE, &label) + 40.0;
            let lb = Rect::new(left_end + 6.0, y, lw, h);
            let clicked = self.button(canvas, lb, &format!("{label}     "), ButtonStyle::Ghost);
            menu_caret(canvas, lb, color::text_dim());
            if clicked {
                let mark = |on: bool| if on { "✓ " } else { "   " };
                let table = &self.strings.table;
                let mut items = vec![MenuItem::ui(
                    format!("{}Keys (as written)", mark(shown.is_none())),
                    "",
                    UiAction::UiDoc(UiDocAction::Language(0)),
                    true,
                )];
                for (i, code) in table.languages.iter().enumerate() {
                    let on = shown.as_deref() == Some(code.as_str());
                    items.push(MenuItem::ui(
                        format!(
                            "{}{} ({code})",
                            mark(on),
                            automata_renderer::bidi::visual(table.name(code))
                        ),
                        "",
                        UiAction::UiDoc(UiDocAction::Language(i + 1)),
                        true,
                    ));
                }
                self.open_menu(lb.x, lb.y + lb.h + 2.0, items);
            }
            left_end = lb.x + lb.w;
        }
        // Right: how scripts reach it, and the text view.
        let tb = Rect::new(r.x + r.w - 104.0, y, 96.0, h);
        if self.button(canvas, tb, "Edit as Text", ButtonStyle::Ghost) {
            self.set_ui_source(true);
        }
        let kind = if doc.hud { "HUD" } else { "screen" };
        let hint = format!("{kind} · ctx.ui.screen(\"{screen}\")");
        let hw = canvas.measure_text(layout::SMALL_SIZE, &hint);
        if tb.x - hw - 16.0 > left_end {
            canvas.text(
                tb.x - hw - 12.0,
                r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
                layout::SMALL_SIZE,
                &hint,
                color::text_dim(),
            );
        }
    }

    /// The language the canvas shows string-table keys in, or `None` for
    /// the keys as written (also when the project has no string tables).
    fn ui_preview_language(&self) -> Option<String> {
        let languages = &self.strings.table.languages;
        match self.uied.language.as_deref() {
            Some("") => None,
            Some(l) if self.strings.table.language_index(l).is_some() => Some(l.to_string()),
            _ => languages.first().cloned(),
        }
    }

    /// The canvas: the preview screen with the document drawn on it; click
    /// selects, dragging a top-level node moves it.
    fn ui_canvas(&mut self, canvas: &mut Canvas, area: Rect, doc: &UiDoc) {
        canvas.fill_rect(area.x, area.y, area.w, area.h, color::viewport_bg());
        let (pw, ph, label) = PREVIEWS[self.uied.preview.min(PREVIEWS.len() - 1)];
        let fit = ((area.w - 40.0) / pw).min((area.h - 56.0) / ph).max(0.01);
        let screen = Rect::new(
            (area.x + (area.w - pw * fit) / 2.0).round(),
            (area.y + 28.0 + (area.h - 28.0 - ph * fit) / 2.0).round(),
            pw * fit,
            ph * fit,
        );
        let frame = uidoc::Frame::new(doc, (pw, ph));
        let k = fit * frame.scale;
        canvas.text(
            screen.x,
            screen.y - 20.0,
            layout::SMALL_SIZE,
            &format!(
                "{label} · designed for {} × {} · {}%",
                num(doc.design_size.0),
                num(doc.design_size.1),
                (frame.scale * 100.0).round()
            ),
            color::text_faint(),
        );

        // A live drag shows the node where it will land.
        let mut view = doc.clone();
        if let Some(d) = &self.uied.drag {
            if d.moved {
                let (mx, my) = d.margin((self.p.x, self.p.y));
                if let Some(n) = view.node_mut(&d.path) {
                    n.x = mx;
                    n.y = my;
                }
            }
        }
        if let Some(lang) = self.ui_preview_language() {
            uidoc::translate(&mut view, &self.strings.table, &lang);
        }
        uidoc::for_display(&mut view);
        let placed = uidoc::layout(&view, &frame);
        let theme = view.theme.resolve();

        // The game's backdrop: a dim gradient so panels read as overlays.
        canvas.fill_rect(
            screen.x,
            screen.y,
            screen.w,
            screen.h,
            Color::from_rgba8(44, 52, 62, 255),
        );
        canvas.fill_rect(
            screen.x,
            screen.y + screen.h * 0.62,
            screen.w,
            screen.h * 0.38,
            Color::from_rgba8(36, 40, 44, 255),
        );
        let prev = canvas.set_clip(Some((screen.x, screen.y, screen.w, screen.h)));
        for p in &placed {
            if let Some(node) = view.node(&p.path) {
                draw_node(canvas, p, node, &theme, (screen.x, screen.y), k);
            }
        }
        let rect_of = |p: &uidoc::Placed| {
            Rect::new(
                screen.x + p.rect[0] * k,
                screen.y + p.rect[1] * k,
                p.rect[2] * k,
                p.rect[3] * k,
            )
        };
        // Hover and selection outlines.
        let inside = self.hot(screen) && self.hot(area);
        let hit = inside
            .then(|| {
                placed.iter().rev().find(|p| {
                    p.rect[2] > 0.0 && p.rect[3] > 0.0 && rect_of(p).contains(self.p.x, self.p.y)
                })
            })
            .flatten();
        if let Some(h) = hit {
            if Some(&h.path) != self.uied.selected.as_ref() && self.uied.drag.is_none() {
                let r = rect_of(h);
                canvas.stroke_rect(r.x, r.y, r.w, r.h, color::accent_soft(), 1.0);
            }
        }
        if let Some(sel) = &self.uied.selected {
            if let Some(p) = placed.iter().find(|p| &p.path == sel) {
                let r = rect_of(p);
                canvas.stroke_rect(
                    r.x - 1.0,
                    r.y - 1.0,
                    r.w + 2.0,
                    r.h + 2.0,
                    color::accent(),
                    2.0,
                );
            }
        }
        canvas.restore_clip(prev);
        canvas.stroke_rect(
            screen.x - 0.5,
            screen.y - 0.5,
            screen.w + 1.0,
            screen.h + 1.0,
            color::border(),
            1.0,
        );

        // Mouse.
        if self.p.pressed && self.hot(area) && self.menu.is_none() {
            let target = hit.map(|h| h.path.clone());
            self.uied.selected.clone_from(&target);
            self.uied.drag = target.filter(|p| p.len() == 1).and_then(|path| {
                let n = doc.node(&path)?;
                Some(NodeDrag {
                    sign: margin_signs(&n.anchor),
                    from: (n.x, n.y),
                    path,
                    start: (self.p.x, self.p.y),
                    k,
                    moved: false,
                })
            });
        }
        let pointer = (self.p.x, self.p.y);
        if let Some(d) = self.uied.drag.as_mut() {
            if (pointer.0 - d.start.0).abs() + (pointer.1 - d.start.1).abs() > 3.0 {
                d.moved = true;
            }
            if d.moved {
                self.cursor = CursorHint::Move;
            }
        }
        if !self.p.down {
            if let Some(d) = self.uied.drag.take() {
                if d.moved {
                    let (mx, my) = d.margin(pointer);
                    self.edit_ui_doc(|doc, _| {
                        if let Some(n) = doc.node_mut(&d.path) {
                            n.x = mx;
                            n.y = my;
                        }
                    });
                }
            }
        }
    }

    fn ui_tree(&mut self, canvas: &mut Canvas, r: Rect, doc: &UiDoc) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x + r.w - 1.0, r.y, 1.0, r.h, color::border());
        canvas.text(
            r.x + 12.0,
            r.y + 9.0,
            layout::SMALL_SIZE,
            "WIDGETS",
            color::text_faint(),
        );
        let list = Rect::new(r.x, r.y + 30.0, r.w - 1.0, r.h - 30.0);
        let rows = tree_rows(doc);
        let total = (rows.len() + 1) as f32 * ROW_H;
        if self.hot(list) && self.p.scroll != 0.0 {
            self.uied.tree_scroll -= self.p.scroll * ROW_H * 3.0;
        }
        self.uied.tree_scroll = self.uied.tree_scroll.clamp(0.0, (total - list.h).max(0.0));
        let prev = canvas.set_clip(Some((list.x, list.y, list.w, list.h)));
        let clip = self.clip;
        self.clip = (list.y.max(clip.0), (list.y + list.h).min(clip.1));
        let mut y = list.y - self.uied.tree_scroll;
        // The screen itself, then every node.
        let entries = std::iter::once(None).chain(rows.iter().map(Some));
        for row in entries {
            let rr = Rect::new(list.x + 4.0, y, list.w - 8.0, ROW_H - 2.0);
            y += ROW_H;
            if !self.row_visible(rr) {
                continue;
            }
            let path = row.map(|r| r.path.clone());
            let selected = self.uied.selected == path;
            let hot = self.hot(rr);
            if selected {
                canvas.fill_rounded(rr.x, rr.y, rr.w, rr.h, 4.0, color::select());
            } else if hot {
                canvas.fill_rounded(rr.x, rr.y, rr.w, rr.h, 4.0, color::hover());
            }
            let (indent, title, c, kind) = match row {
                None => (0.0, "Screen".to_string(), color::text(), None),
                Some(t) => {
                    #[allow(clippy::cast_precision_loss)]
                    let indent = (t.depth + 1) as f32 * 14.0;
                    let c = if t.shown {
                        color::text()
                    } else {
                        color::text_faint()
                    };
                    (indent, t.title.clone(), c, Some(t.kind))
                }
            };
            let ix = rr.x + 8.0 + indent;
            draw_kind_glyph(canvas, kind, ix + 6.0, rr.y + rr.h / 2.0);
            canvas.text_clipped(
                ix + 18.0,
                rr.y + 4.0,
                layout::FONT_SIZE,
                &title,
                c,
                rr.x + rr.w - ix - 22.0,
            );
            if hot && self.p.pressed {
                self.uied.selected = path;
            }
        }
        self.clip = clip;
        canvas.restore_clip(prev);
    }

    #[allow(clippy::too_many_lines)]
    fn ui_props(&mut self, canvas: &mut Canvas, r: Rect, doc: &UiDoc, screen: &str) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y, 1.0, r.h, color::border());
        let x = r.x + 12.0;
        let w = r.w - 24.0;
        if self.hot(r) && self.p.scroll != 0.0 {
            self.uied.props_scroll =
                (self.uied.props_scroll - self.p.scroll * ROW_H * 3.0).max(0.0);
        }
        let prev = canvas.set_clip(Some((r.x, r.y, r.w, r.h)));
        let clip = self.clip;
        self.clip = (r.y.max(clip.0), (r.y + r.h).min(clip.1));
        let top = r.y + 10.0 - self.uied.props_scroll;
        let mut y = top;
        let node = self
            .uied
            .selected
            .as_deref()
            .and_then(|p| doc.node(p).map(|n| (p.len(), n.clone())));
        match node {
            None => {
                y = self.section_head(canvas, x, y, "Screen");
                canvas.text_clipped(
                    x,
                    y + 4.0,
                    layout::SMALL_SIZE,
                    &format!("ctx.ui.screen(\"{screen}\")"),
                    color::accent_soft(),
                    w,
                );
                y += ROW_H;
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Design W",
                    UiProp::DesignW,
                    &num(doc.design_size.0),
                    "",
                );
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Design H",
                    UiProp::DesignH,
                    &num(doc.design_size.1),
                    "",
                );
                let flags: [ScreenFlag; 4] = [
                    ("HUD", doc.hud, |d| d.hud = !d.hud),
                    ("Modal", doc.modal, |d| d.modal = !d.modal),
                    ("Pause", doc.pause, |d| d.pause = !d.pause),
                    ("Escape", doc.escape, |d| d.escape = !d.escape),
                ];
                for (label, on, flip) in flags {
                    let (ny, clicked) = self.prop_check(canvas, x, y, w, label, on);
                    y = ny;
                    if clicked {
                        self.edit_ui_doc(|d, _| flip(d));
                    }
                }
                let hint = if doc.hud {
                    "Shown from the start, under every screen."
                } else {
                    "Shown with ctx.ui.screens.show(name)."
                };
                canvas.text_clipped(x, y + 2.0, layout::SMALL_SIZE, hint, color::text_faint(), w);
                y += ROW_H;
                y = self.section_head(canvas, x, y, "Theme");
                let defaults = ThemeColors::default();
                let size = doc.theme.text_size.map(num).unwrap_or_default();
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Text size",
                    UiProp::TextSize,
                    &size,
                    &num(defaults.text_size),
                );
                let font = doc.theme.font.clone().unwrap_or_default();
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Font",
                    UiProp::ThemeFont,
                    &font,
                    "built-in",
                );
                let resolved = doc.theme.resolve();
                for (i, slot) in THEME_SLOTS.iter().enumerate() {
                    let set = doc.theme.clone().slot_mut(slot).and_then(|s| *s);
                    let shown = resolved.slot(slot).unwrap_or([0.0; 4]);
                    let default = defaults.slot(slot).unwrap_or([0.0; 4]);
                    let label = slot.replace('_', " ");
                    y = self.prop_colour(
                        canvas,
                        x,
                        y,
                        w,
                        &label,
                        UiProp::Theme(i),
                        set,
                        shown,
                        &hex(default),
                    );
                }
            }
            Some((depth, n)) => {
                y = self.section_head(canvas, x, y, n.kind.label());
                let dup = !n.name.is_empty() && doc.names().get(&n.name).copied().unwrap_or(0) > 1;
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Name",
                    UiProp::Name,
                    &n.name,
                    "for scripts",
                );
                if dup {
                    canvas.text_clipped(
                        x + LABEL_W,
                        y - 4.0,
                        layout::SMALL_SIZE,
                        "Used twice: scripts need it unique",
                        color::warn(),
                        w - LABEL_W,
                    );
                    y += 16.0;
                }
                let text_label = match n.kind {
                    NodeKind::Image => Some("Image file"),
                    NodeKind::Label | NodeKind::Button | NodeKind::Toggle | NodeKind::Progress => {
                        Some("Text")
                    }
                    _ => None,
                };
                if let Some(label) = text_label {
                    let hint = if n.kind == NodeKind::Progress {
                        "{value} / {max}"
                    } else {
                        ""
                    };
                    y = self.prop_field(
                        canvas,
                        x,
                        y,
                        w,
                        label,
                        UiProp::Text,
                        &n.text.replace('\n', "\\n"),
                        hint,
                    );
                }
                if depth == 1 {
                    y = self.section_head(canvas, x, y, "Place");
                    let anchor = if n.anchor.is_empty() {
                        "none"
                    } else {
                        n.anchor.as_str()
                    };
                    let (ny, clicked) = self.prop_menu(canvas, x, y, w, "Anchor", anchor);
                    if clicked {
                        let mut items = vec![MenuItem::ui(
                            "none",
                            "",
                            UiAction::UiDoc(UiDocAction::Anchor(0)),
                            true,
                        )];
                        items.extend(uidoc::ANCHORS.iter().enumerate().map(|(i, a)| {
                            MenuItem::ui(a.0, "", UiAction::UiDoc(UiDocAction::Anchor(i + 1)), true)
                        }));
                        self.open_menu(x + LABEL_W, ny, items);
                    }
                    y = ny;
                    let (xl, yl) = if n.anchor.is_empty() {
                        ("X", "Y")
                    } else {
                        ("Margin X", "Margin Y")
                    };
                    y = self.prop_field(canvas, x, y, w, xl, UiProp::X, &num(n.x), "");
                    y = self.prop_field(canvas, x, y, w, yl, UiProp::Y, &num(n.y), "");
                }
                y = self.section_head(canvas, x, y, "Size");
                let opt = |v: Option<f32>| v.map(num).unwrap_or_default();
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Width",
                    UiProp::Width,
                    &opt(n.width),
                    "auto",
                );
                y = self.prop_field(
                    canvas,
                    x,
                    y,
                    w,
                    "Height",
                    UiProp::Height,
                    &opt(n.height),
                    "auto",
                );
                if n.kind.is_layout() {
                    y = self.section_head(canvas, x, y, "Layout");
                    if n.kind != NodeKind::Stack {
                        y = self.prop_field(canvas, x, y, w, "Gap", UiProp::Gap, &num(n.gap), "");
                    }
                    y = self.prop_field(
                        canvas,
                        x,
                        y,
                        w,
                        "Padding",
                        UiProp::Padding,
                        &num(n.padding),
                        "",
                    );
                    if n.kind == NodeKind::Row {
                        let cols = if n.columns == 0 {
                            String::new()
                        } else {
                            n.columns.to_string()
                        };
                        y = self.prop_field(
                            canvas,
                            x,
                            y,
                            w,
                            "Columns",
                            UiProp::Columns,
                            &cols,
                            "by content",
                        );
                    }
                    let (ny, clicked) = self.prop_check(canvas, x, y, w, "Panel", n.panel);
                    y = ny;
                    if clicked {
                        self.edit_selected(|n| n.panel = !n.panel);
                    }
                    if n.kind != NodeKind::Stack {
                        y = self.prop_field(canvas, x, y, w, "Title", UiProp::Title, &n.title, "");
                    }
                }
                if matches!(
                    n.kind,
                    NodeKind::Label | NodeKind::Button | NodeKind::Toggle | NodeKind::Progress
                ) {
                    y = self.section_head(canvas, x, y, "Text");
                    let theme = doc.theme.resolve();
                    y = self.prop_field(
                        canvas,
                        x,
                        y,
                        w,
                        "Size",
                        UiProp::Size,
                        &opt(n.size),
                        &num(theme.text_size),
                    );
                    if n.kind == NodeKind::Label {
                        let (ny, clicked) = self.prop_menu(canvas, x, y, w, "Align", &n.align);
                        if clicked {
                            let items = ALIGNS
                                .iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    MenuItem::ui(
                                        *a,
                                        "",
                                        UiAction::UiDoc(UiDocAction::Align(i)),
                                        true,
                                    )
                                })
                                .collect();
                            self.open_menu(x + LABEL_W, ny, items);
                        }
                        y = ny;
                        y = self.prop_colour(
                            canvas,
                            x,
                            y,
                            w,
                            "Colour",
                            UiProp::Color,
                            n.color,
                            n.color.unwrap_or(theme.text),
                            "theme",
                        );
                        y = self.prop_field(
                            canvas,
                            x,
                            y,
                            w,
                            "Font",
                            UiProp::Font,
                            &n.font,
                            "theme",
                        );
                    }
                    if n.kind == NodeKind::Progress {
                        y = self.prop_colour(
                            canvas,
                            x,
                            y,
                            w,
                            "Colour",
                            UiProp::Color,
                            n.color,
                            n.color.unwrap_or(theme.accent),
                            "accent",
                        );
                    }
                }
                if matches!(
                    n.kind,
                    NodeKind::Toggle | NodeKind::Slider | NodeKind::Progress
                ) {
                    y = self.section_head(canvas, x, y, "Value");
                    if n.kind == NodeKind::Toggle {
                        let (ny, clicked) = self.prop_check(canvas, x, y, w, "On", n.value > 0.5);
                        y = ny;
                        if clicked {
                            self.edit_selected(|n| n.value = if n.value > 0.5 { 0.0 } else { 1.0 });
                        }
                    } else {
                        y = self.prop_field(
                            canvas,
                            x,
                            y,
                            w,
                            "Value",
                            UiProp::Value,
                            &num(n.value),
                            "",
                        );
                        if n.kind == NodeKind::Slider {
                            y = self.prop_field(
                                canvas,
                                x,
                                y,
                                w,
                                "Min",
                                UiProp::Min,
                                &num(n.min),
                                "",
                            );
                        }
                        y = self.prop_field(canvas, x, y, w, "Max", UiProp::Max, &num(n.max), "");
                        if n.kind == NodeKind::Slider {
                            y = self.prop_field(
                                canvas,
                                x,
                                y,
                                w,
                                "Step",
                                UiProp::Step,
                                &num(n.step),
                                "smooth",
                            );
                        }
                    }
                }
                y = self.section_head(canvas, x, y, "State");
                let (ny, clicked) = self.prop_check(canvas, x, y, w, "Visible", n.visible);
                y = ny;
                if clicked {
                    self.edit_selected(|n| n.visible = !n.visible);
                }
                if matches!(
                    n.kind,
                    NodeKind::Button | NodeKind::Toggle | NodeKind::Slider
                ) {
                    let (ny, clicked) = self.prop_check(canvas, x, y, w, "Enabled", n.enabled);
                    y = ny;
                    if clicked {
                        self.edit_selected(|n| n.enabled = !n.enabled);
                    }
                }
                if !n.name.is_empty() {
                    y += 6.0;
                    let call = match n.kind {
                        NodeKind::Button => format!(".clicked(\"{}\")", n.name),
                        NodeKind::Toggle | NodeKind::Slider => format!(".get(\"{}\")", n.name),
                        NodeKind::Progress => format!(".set(\"{}\", 50)", n.name),
                        NodeKind::Column
                        | NodeKind::Row
                        | NodeKind::Stack
                        | NodeKind::Space
                        | NodeKind::Image => {
                            format!(".set_visible(\"{}\", False)", n.name)
                        }
                        NodeKind::Label => format!(".set(\"{}\", \"…\")", n.name),
                    };
                    canvas.text_clipped(
                        x,
                        y,
                        layout::SMALL_SIZE,
                        &format!("ctx.ui.screen(\"{screen}\"){call}"),
                        color::accent_soft(),
                        w,
                    );
                    y += ROW_H;
                }
            }
        }
        let used = y - top + 20.0;
        self.uied.props_scroll = self.uied.props_scroll.min((used - r.h).max(0.0));
        self.clip = clip;
        canvas.restore_clip(prev);
    }

    fn edit_selected(&mut self, f: impl FnOnce(&mut UiNode)) {
        self.edit_ui_doc(|doc, sel| {
            if let Some(n) = sel.as_deref().and_then(|p| doc.node_mut(p)) {
                f(n);
            }
        });
    }

    pub(super) fn section_head(&self, canvas: &mut Canvas, x: f32, y: f32, title: &str) -> f32 {
        let y = if y > 0.0 { y + 6.0 } else { y };
        canvas.text_in(
            crate::canvas::Face::Medium,
            x,
            y + 2.0,
            layout::FONT_SIZE,
            title,
            color::text(),
        );
        y + 24.0
    }

    #[allow(clippy::too_many_arguments)]
    fn prop_field(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        prop: UiProp,
        value: &str,
        placeholder: &str,
    ) -> f32 {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let field = Rect::new(x + LABEL_W, y, w - LABEL_W, ROW_H - 4.0);
        self.text_box(canvas, field, prop, value, placeholder);
        y + ROW_H
    }

    fn text_box(
        &mut self,
        canvas: &mut Canvas,
        field: Rect,
        prop: UiProp,
        value: &str,
        placeholder: &str,
    ) {
        let numeric = !matches!(
            prop,
            UiProp::Name | UiProp::Text | UiProp::Title | UiProp::Font | UiProp::ThemeFont
        );
        self.field_box(
            canvas,
            field,
            EditTarget::UiProp(prop),
            numeric,
            value,
            placeholder,
        );
    }

    /// A text field editing `target` (committed through
    /// [`Ui::commit_edit`]); a click on a `numeric` one selects it all.
    pub(super) fn field_box(
        &mut self,
        canvas: &mut Canvas,
        field: Rect,
        target: EditTarget,
        numeric: bool,
        value: &str,
        placeholder: &str,
    ) {
        if !self.row_visible(field) {
            return;
        }
        let editing = self.edit.as_ref().is_some_and(|e| e.target == target);
        canvas.fill_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            layout::RADIUS,
            color::field(),
        );
        let border = if editing {
            color::accent()
        } else {
            color::border()
        };
        canvas.stroke_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            layout::RADIUS,
            border,
            1.0,
        );
        if editing {
            self.draw_edit_text(canvas, field, field.x + 6.0);
            return;
        }
        let (text, c) = if value.is_empty() {
            (placeholder, color::text_faint())
        } else {
            (value, color::text())
        };
        canvas.text_clipped(
            field.x + 6.0,
            field.y + (field.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            text,
            c,
            field.w - 12.0,
        );
        if self.hot(field) {
            self.cursor = CursorHint::Text;
            if self.p.pressed {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
                self.edit = Some(TextEdit {
                    id: 0,
                    target,
                    buffer: value.to_string(),
                    rect: field,
                    fresh: numeric,
                    attr: None,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prop_colour(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        prop: UiProp,
        set: Option<[f32; 4]>,
        shown: [f32; 4],
        placeholder: &str,
    ) -> f32 {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let sw = Rect::new(x + LABEL_W, y + 2.0, ROW_H - 8.0, ROW_H - 8.0);
        canvas.fill_rounded(sw.x, sw.y, sw.w, sw.h, 3.0, rgba(shown));
        canvas.stroke_rounded(sw.x, sw.y, sw.w, sw.h, 3.0, color::border(), 1.0);
        let field = Rect::new(sw.x + sw.w + 6.0, y, w - LABEL_W - sw.w - 6.0, ROW_H - 4.0);
        self.text_box(
            canvas,
            field,
            prop,
            &set.map(hex).unwrap_or_default(),
            placeholder,
        );
        y + ROW_H
    }

    pub(super) fn prop_check(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        on: bool,
    ) -> (f32, bool) {
        let row = Rect::new(x, y, w, ROW_H - 4.0);
        let hot = self.hot(row);
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let b = Rect::new(x + LABEL_W, y + 3.0, 16.0, 16.0);
        canvas.fill_rounded(
            b.x,
            b.y,
            b.w,
            b.h,
            3.0,
            if on { color::accent() } else { color::field() },
        );
        canvas.stroke_rounded(
            b.x,
            b.y,
            b.w,
            b.h,
            3.0,
            if hot {
                color::accent_soft()
            } else {
                color::border()
            },
            1.0,
        );
        if on {
            canvas.polyline(
                &[
                    (b.x + 4.0, b.y + 8.5),
                    (b.x + 7.0, b.y + 11.5),
                    (b.x + 12.5, b.y + 5.0),
                ],
                color::on_accent(),
                2.0,
            );
        }
        if hot {
            self.cursor = CursorHint::Pointer;
        }
        (y + ROW_H, hot && self.p.pressed)
    }

    pub(super) fn prop_menu(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        value: &str,
    ) -> (f32, bool) {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let b = Rect::new(x + LABEL_W, y, w - LABEL_W, ROW_H - 4.0);
        let hot = self.hot(b);
        canvas.fill_rounded(
            b.x,
            b.y,
            b.w,
            b.h,
            3.0,
            if hot { color::hover() } else { color::raised() },
        );
        canvas.text_clipped(
            b.x + 6.0,
            b.y + (b.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            value,
            color::text(),
            b.w - 24.0,
        );
        let (cx, cy) = (b.x + b.w - 11.0, b.y + b.h / 2.0);
        canvas.fill_polygon(
            &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
            color::text_dim(),
        );
        if hot {
            self.cursor = CursorHint::Pointer;
        }
        (y + ROW_H, hot && self.p.pressed)
    }

    fn ui_status(&mut self, canvas: &mut Canvas, r: Rect, doc: &UiDoc, screen: &str, idx: usize) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y, r.w, 1.0, color::border());
        let names = doc.names();
        let twice: Vec<&str> = names
            .iter()
            .filter(|(_, n)| **n > 1)
            .map(|(k, _)| k.as_str())
            .collect();
        let count = tree_rows(doc).len();
        let dirty = self.code.tabs.get(idx).is_some_and(|t| t.buf.dirty());
        let mut text = format!(
            "ui \u{201c}{screen}\u{201d} · {count} node{} · {} named",
            if count == 1 { "" } else { "s" },
            names.len()
        );
        if dirty {
            text.push_str(" · unsaved (Ctrl+S)");
        }
        let c = if twice.is_empty() {
            color::text_dim()
        } else {
            text.push_str(" · names used twice: ");
            text.push_str(&twice.join(", "));
            color::warn()
        };
        if self
            .uied
            .drag
            .as_ref()
            .is_some_and(|d| d.moved && d.sign == (0.0, 0.0))
        {
            text = "A centred node stays centred: pick another anchor to move it".into();
        }
        canvas.text_clipped(
            r.x + 10.0,
            r.y + 4.0,
            layout::SMALL_SIZE,
            &text,
            c,
            r.w - 20.0,
        );
    }

    fn ui_broken(&mut self, canvas: &mut Canvas, r: Rect, error: &str) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::viewport_bg());
        let w = (r.w - 80.0).clamp(200.0, 640.0);
        let x = r.x + (r.w - w) / 2.0;
        let mut y = r.y + 60.0;
        canvas.text_in(
            crate::canvas::Face::Medium,
            x,
            y,
            16.0,
            "This UI document doesn't read",
            color::text(),
        );
        y += 30.0;
        for line in super::code_view::wrap(error, 80).iter().take(8) {
            canvas.text_clipped(x, y, layout::FONT_SIZE, line, color::danger(), w);
            y += 20.0;
        }
        y += 14.0;
        if self.button(
            canvas,
            Rect::new(x, y, 140.0, 30.0),
            "Fix it as Text",
            ButtonStyle::Accent,
        ) {
            self.set_ui_source(true);
        }
    }

    /// The bar above a UI document shown as text.
    pub(super) fn ui_source_bar(&mut self, canvas: &mut Canvas, r: Rect) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, color::border());
        canvas.text(
            r.x + 12.0,
            r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
            layout::SMALL_SIZE,
            "The UI document as text",
            color::text_dim(),
        );
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 128.0, r.y + 4.0, 120.0, r.h - 8.0),
            "Back to the Editor",
            ButtonStyle::Ghost,
        ) {
            self.set_ui_source(false);
        }
    }
}

/// The small ▾ at the right of a button that opens a menu.
pub(super) fn menu_caret(canvas: &mut Canvas, r: Rect, c: Color) {
    let (cx, cy) = (r.x + r.w - 13.0, r.y + r.h / 2.0);
    canvas.fill_polygon(
        &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
        c,
    );
}

/// How a node's margin moves it per axis for its anchor: +1 (towards the
/// right / bottom), -1, or 0 (centred: the margin doesn't move it).
fn margin_signs(anchor: &str) -> (f32, f32) {
    let sign = |f: f32| {
        if f < 0.25 {
            1.0
        } else if f > 0.75 {
            -1.0
        } else {
            0.0
        }
    };
    match uidoc::anchor_fractions(anchor) {
        Some((fx, fy)) if !anchor.is_empty() => (sign(fx), sign(fy)),
        _ => (1.0, 1.0),
    }
}

/// A small glyph for the node kind in the tree (`None`: the screen).
fn draw_kind_glyph(canvas: &mut Canvas, kind: Option<NodeKind>, cx: f32, cy: f32) {
    let c = color::text_dim();
    let a = color::accent_soft();
    match kind {
        None => canvas.stroke_rect(cx - 6.0, cy - 4.5, 12.0, 9.0, a, 1.2),
        Some(NodeKind::Column) => {
            canvas.stroke_rect(cx - 5.0, cy - 6.0, 10.0, 12.0, c, 1.0);
            canvas.line(cx - 5.0, cy, cx + 5.0, cy, c, 1.0);
        }
        Some(NodeKind::Row) => {
            canvas.stroke_rect(cx - 6.0, cy - 5.0, 12.0, 10.0, c, 1.0);
            canvas.line(cx, cy - 5.0, cx, cy + 5.0, c, 1.0);
        }
        Some(NodeKind::Stack) => {
            canvas.stroke_rect(cx - 6.0, cy - 5.0, 9.0, 8.0, c, 1.0);
            canvas.stroke_rect(cx - 3.0, cy - 2.0, 9.0, 8.0, c, 1.0);
        }
        Some(NodeKind::Label) => {
            canvas.text(cx - 4.0, cy - 7.0, 11.0, "T", c);
        }
        Some(NodeKind::Button) => canvas.fill_rounded(cx - 6.0, cy - 4.0, 12.0, 8.0, 2.0, a),
        Some(NodeKind::Toggle) => {
            canvas.stroke_rect(cx - 4.5, cy - 4.5, 9.0, 9.0, c, 1.0);
            canvas.fill_rect(cx - 2.0, cy - 2.0, 4.0, 4.0, a);
        }
        Some(NodeKind::Slider) => {
            canvas.line(cx - 6.0, cy, cx + 6.0, cy, c, 1.5);
            canvas.fill_rect(cx - 1.0, cy - 4.0, 3.0, 8.0, a);
        }
        Some(NodeKind::Progress) => {
            canvas.stroke_rect(cx - 6.0, cy - 3.0, 12.0, 6.0, c, 1.0);
            canvas.fill_rect(cx - 5.0, cy - 2.0, 6.0, 4.0, a);
        }
        Some(NodeKind::Image) => {
            canvas.stroke_rect(cx - 6.0, cy - 5.0, 12.0, 10.0, c, 1.0);
            canvas.fill_polygon(
                &[
                    (cx - 5.0, cy + 4.0),
                    (cx - 1.0, cy - 1.0),
                    (cx + 2.0, cy + 2.0),
                    (cx + 5.0, cy + 4.0),
                ],
                a,
            );
        }
        Some(NodeKind::Space) => {
            canvas.line(cx - 5.0, cy - 4.0, cx + 5.0, cy - 4.0, c, 1.0);
            canvas.line(cx - 5.0, cy + 4.0, cx + 5.0, cy + 4.0, c, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    use crate::ui::Pointer;

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::default()
        }
    }

    fn open_hud(tag: &str) -> (Ui, std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("oberon-uied-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("ui").join("hud.ui.ron");
        UiDoc::template_hud().save(&path).unwrap();
        let mut ui = Ui::new();
        ui.open_code(&path, None);
        (ui, dir, path)
    }

    fn frame(ui: &mut Ui, canvas: &mut Canvas, p: Pointer) {
        let _ = ui.frame(canvas, &Document::starter(), p, None);
    }

    fn click(ui: &mut Ui, canvas: &mut Canvas, x: f32, y: f32) {
        let down = Pointer {
            x,
            y,
            pressed: true,
            down: true,
            ..Pointer::default()
        };
        frame(ui, canvas, down);
        frame(
            ui,
            canvas,
            Pointer {
                x,
                y,
                ..Pointer::default()
            },
        );
    }

    #[test]
    fn the_canvas_shows_string_table_keys_in_a_chosen_language() {
        let dir = std::env::temp_dir().join(format!("oberon-uied-{}-lang", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("ui").join("hud.ui.ron");
        let mut hud = UiDoc::template_hud();
        hud.nodes[0].children[0].text = "hud.score".into();
        hud.save(&path).unwrap();
        let mut ui = Ui::new();
        ui.open_code(&path, None);
        std::fs::create_dir_all(dir.join("lang")).unwrap();
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/oberon/shape_signal/fonts/MPLUSRounded1c-ShapeSignal.ttf");
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::copy(font, dir.join("fonts/JP.ttf")).unwrap();
        std::fs::write(
            dir.join("lang/strings.csv"),
            "key,en,ja\n@name,English,日本語\n@font,,fonts/JP.ttf\nhud.score,SCORE,スコア\n",
        )
        .unwrap();
        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        let mut canvas = Canvas::new(1600, 950);
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);
        assert_eq!(ui.strings.table.languages, ["en", "ja"]);
        assert_eq!(
            ui.ui_language().as_deref(),
            Some("en"),
            "the first language"
        );
        assert_eq!(canvas.fallback_font_count(), 1, "the ja font is loaded");

        // The menu lists the keys and each language; picking one shows it.
        ui.ui_doc_action(UiDocAction::Language(2));
        assert_eq!(ui.ui_language().as_deref(), Some("ja"));
        let mut shown = ui.ui_editor_doc().unwrap().clone();
        assert_eq!(
            shown.nodes[0].children[0].text, "hud.score",
            "the file keeps the key"
        );
        uidoc::translate(&mut shown, &ui.strings.table, "ja");
        assert_eq!(shown.nodes[0].children[0].text, "スコア");
        assert!(
            canvas.measure_text(14.0, "スコア") > 30.0,
            "drawn with the ja font"
        );
        ui.ui_doc_action(UiDocAction::Language(0));
        assert_eq!(ui.ui_language(), None, "the keys as written");
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ui_document_opens_in_the_editor_and_edits_round_trip_through_the_file() {
        let (mut ui, dir, path) = open_hud("edit");
        let mut canvas = Canvas::new(1600, 950);
        frame(&mut ui, &mut canvas, Pointer::default());
        assert!(ui.ui_editor_shown());
        assert_eq!(ui.ui_editor_doc().unwrap().nodes.len(), 1);

        // Select the health bar from the tree (row 3: Screen, Column, score, health).
        ui.uied.selected = Some(vec![0, 1]);
        ui.commit_ui_prop(UiProp::Value, "40");
        assert!((ui.ui_editor_doc().unwrap().node(&[0, 1]).unwrap().value - 40.0).abs() < 1e-6);
        ui.commit_ui_prop(UiProp::Name, " hp ");
        ui.commit_ui_prop(UiProp::Width, "");
        ui.commit_ui_prop(UiProp::Value, "lots");
        let n = ui.ui_editor_doc().unwrap().node(&[0, 1]).unwrap().clone();
        assert_eq!(n.name, "hp");
        assert!(n.width.is_none());
        assert!(
            (n.value - 40.0).abs() < 1e-6,
            "a bad number changes nothing"
        );

        // Add a button after it, then undo it with Ctrl+Z.
        ui.ui_doc_action(UiDocAction::Add(NodeKind::Button));
        assert_eq!(ui.uied.selected, Some(vec![0, 2]));
        assert_eq!(ui.ui_editor_doc().unwrap().nodes[0].children.len(), 3);
        ui.code.focused = true;
        assert!(ui.code_key(Key::Char('z'), ctrl()));
        frame(&mut ui, &mut canvas, Pointer::default());
        assert_eq!(ui.ui_editor_doc().unwrap().nodes[0].children.len(), 2);

        // Typing doesn't touch the text; Ctrl+S saves what the editor shows.
        ui.code_text("garbage");
        assert!(ui.code_key(Key::Char('s'), ctrl()));
        let saved = UiDoc::load(&path).unwrap();
        assert_eq!(saved.node(&[0, 1]).unwrap().name, "hp");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("// A UI screen"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_delete_copy_paste_and_nudge() {
        let (mut ui, dir, _) = open_hud("keys");
        let mut canvas = Canvas::new(1600, 950);
        frame(&mut ui, &mut canvas, Pointer::default());
        ui.code.focused = true;
        ui.uied.selected = Some(vec![0, 0]);
        assert!(ui.code_key(Key::Char('c'), ctrl()));
        assert!(ui.code_key(Key::Char('v'), ctrl()));
        let doc = ui.ui_editor_doc().unwrap().clone();
        assert_eq!(
            doc.nodes[0].children[1].name, "score_2",
            "a pasted copy gets a fresh name"
        );
        assert!(ui.code_key(Key::Delete, Mods::default()));
        assert_eq!(ui.ui_editor_doc().unwrap().nodes[0].children.len(), 2);

        // Arrows move the top-level column by its margin (Shift: by 10).
        ui.uied.selected = Some(vec![0]);
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert!(ui.code_key(Key::Right, shift));
        assert!(ui.code_key(Key::Down, Mods::default()));
        let col = &ui.ui_editor_doc().unwrap().nodes[0];
        assert_eq!((col.x, col.y), (34.0, 25.0));
        // Tab walks the tree; Escape selects the screen.
        assert!(ui.code_key(Key::Tab, Mods::default()));
        assert_eq!(ui.uied.selected, Some(vec![0, 0]));
        assert!(ui.code_key(Key::Escape, Mods::default()));
        assert_eq!(ui.uied.selected, None);
        // Shortcuts the editor doesn't use go to the app.
        assert!(!ui.code_key(Key::Char('p'), ctrl()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clicking_the_canvas_selects_and_dragging_moves_by_the_margin() {
        let (mut ui, dir, _) = open_hud("canvas");
        let mut canvas = Canvas::new(1600, 950);
        frame(&mut ui, &mut canvas, Pointer::default());
        // Find the health bar on screen from the same layout the canvas uses.
        let v = ui.layout(1600.0, 950.0).viewport;
        let body = Rect::new(v.x, v.y + TOOLBAR_H, v.w, v.h - TOOLBAR_H - STATUS_H);
        let (tw, pw) = if body.w < 980.0 {
            (170.0, 250.0)
        } else {
            (210.0, 280.0)
        };
        let area = Rect::new(body.x + tw, body.y, body.w - tw - pw, body.h);
        let fit = ((area.w - 40.0) / 1280.0).min((area.h - 56.0) / 720.0);
        let sx = (area.x + (area.w - 1280.0 * fit) / 2.0).round();
        let sy = (area.y + 28.0 + (area.h - 28.0 - 720.0 * fit) / 2.0).round();
        let doc = ui.ui_editor_doc().unwrap().clone();
        let placed = uidoc::layout(&doc, &uidoc::Frame::new(&doc, (1280.0, 720.0)));
        let bar = placed.iter().find(|p| p.path == vec![0, 1]).unwrap();
        let (bx, by) = (
            sx + (bar.rect[0] + 10.0) * fit,
            sy + (bar.rect[1] + 5.0) * fit,
        );
        click(&mut ui, &mut canvas, bx, by);
        assert_eq!(ui.uied.selected, Some(vec![0, 1]));

        // Drag the column (by its bar) 100 units right: written once.
        let down = |x, y, pressed| Pointer {
            x,
            y,
            down: true,
            pressed,
            ..Pointer::default()
        };
        ui.uied.selected = None;
        // Press the column itself: in the gap between its label and bar.
        let col = placed.iter().find(|p| p.path == vec![0]).unwrap();
        let score = placed.iter().find(|p| p.path == vec![0, 0]).unwrap();
        let gap_y = score.rect[1] + score.rect[3] + 4.0;
        let (cx, cy) = (sx + (col.rect[0] + 2.0) * fit, sy + gap_y * fit);
        frame(&mut ui, &mut canvas, down(cx, cy, true));
        assert_eq!(ui.uied.selected, Some(vec![0]));
        frame(&mut ui, &mut canvas, down(cx + 50.0 * fit, cy, false));
        frame(&mut ui, &mut canvas, down(cx + 100.0 * fit, cy, false));
        assert!(
            (ui.ui_editor_doc().unwrap().nodes[0].x - 24.0).abs() < 1e-3,
            "not written mid-drag"
        );
        frame(
            &mut ui,
            &mut canvas,
            Pointer {
                x: cx + 100.0 * fit,
                y: cy,
                ..Pointer::default()
            },
        );
        assert!((ui.ui_editor_doc().unwrap().nodes[0].x - 124.0).abs() < 1.01);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_document_offers_the_text_view() {
        let dir = std::env::temp_dir().join(format!("oberon-uied-{}-broken", std::process::id()));
        let path = dir.join("ui").join("bad.ui.ron");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "(nodes: [(kind: Buton)])").unwrap();
        let mut ui = Ui::new();
        ui.open_code(&path, None);
        let mut canvas = Canvas::new(1280, 800);
        frame(&mut ui, &mut canvas, Pointer::default());
        assert!(ui.ui_editor_shown());
        assert!(ui.ui_editor_doc().is_none());
        ui.set_ui_source(true);
        assert!(!ui.ui_editor_shown());
        frame(&mut ui, &mut canvas, Pointer::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers() {
        assert_eq!(num(24.0), "24");
        assert_eq!(num(0.25), "0.25");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(hex([1.0, 0.5, 0.0, 1.0]), "#ff8000");
        assert_eq!(hex([0.0, 0.0, 0.0, 0.5]), "#00000080");
        assert_eq!(margin_signs(""), (1.0, 1.0));
        assert_eq!(margin_signs("bottom_right"), (-1.0, -1.0));
        assert_eq!(margin_signs("top"), (0.0, 1.0));
    }
}
