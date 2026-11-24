//! The code editor view: document tabs over the center area, the text
//! area (gutter, highlighting, selection, diagnostics), the find / go-to
//! bars, the completion popup, and mouse handling.

use tiny_skia::Color;

use super::menus::{Action, MenuItem, TabCommand, UiAction};
use super::{ButtonStyle, CursorHint, LogLevel, Rect, Ui};
use crate::canvas::Canvas;
use crate::code::buffer::{col_at_visual, visual_col};
use crate::code::syntax::{self, Tok};
use crate::code::{CodeEvent, CodeTab, CompletionKind, Focus, Pos, Severity};
use crate::theme::{color, layout};

/// Height of the document tab strip.
pub const DOC_TABS_H: f32 = 28.0;
/// Line height at the default font size (tests).
#[cfg(test)]
const LINE_H: f32 = 19.0;
/// Status line height.
const STATUS_H: f32 = 22.0;
/// Find / go-to bar row height.
const BAR_ROW: f32 = 30.0;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

/// Syntax colors: paper-white text on black with two warm / cool tints
/// (peach for things you call, mint for strings) and quiet grays for
/// structure.
fn tok_color(t: Tok) -> Color {
    match t {
        Tok::Text => rgb(236, 234, 228),
        Tok::Keyword => rgb(158, 156, 150),
        Tok::Builtin | Tok::Function | Tok::Decorator => rgb(255, 199, 153),
        Tok::Constant | Tok::Number => rgb(255, 170, 128),
        Tok::SelfKw => rgb(198, 196, 190),
        Tok::Str => rgb(153, 238, 214),
        Tok::Comment => rgb(98, 97, 93),
        Tok::Class => rgb(255, 226, 190),
        Tok::Op => rgb(124, 122, 117),
    }
}

fn severity_color(s: Severity) -> Color {
    match s {
        Severity::Error => color::danger(),
        Severity::Warning => color::warn(),
        Severity::Info => color::accent(),
        Severity::Hint => color::text_dim(),
    }
}

/// Geometry of the text area for one frame.
#[derive(Debug, Clone, Copy)]
struct TextGeom {
    area: Rect,
    text_x: f32,
    cell: f32,
    scroll: f32,
    scroll_x: f32,
    lh: f32,
}

impl TextGeom {
    fn pos_at(&self, buf: &crate::code::Buffer, x: f32, y: f32) -> Pos {
        let line_f = ((y - self.area.y) / self.lh + self.scroll).floor().max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let line = (line_f as usize).min(buf.line_count() - 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let vcol = ((x - self.text_x + self.scroll_x) / self.cell)
            .round()
            .max(0.0) as usize;
        Pos::new(line, col_at_visual(buf.line(line), vcol))
    }

    fn y_of(&self, line: usize) -> f32 {
        self.area.y + (line as f32 - self.scroll) * self.lh
    }

    fn x_of(&self, buf: &crate::code::Buffer, p: Pos) -> f32 {
        self.text_x - self.scroll_x + visual_col(buf.line(p.line), p.col) as f32 * self.cell
    }
}

impl Ui {
    /// The document tab strip above the center area (when scripts are
    /// open).
    pub(super) fn doc_tabs_rect(&self, viewport: Rect) -> Option<Rect> {
        (!self.code.tabs.is_empty())
            .then(|| Rect::new(viewport.x, viewport.y - DOC_TABS_H, viewport.w, DOC_TABS_H))
    }

    /// Code font size and line height from the settings.
    fn code_metrics(&self) -> (f32, f32) {
        let font = self.prefs.editor.font_size.clamp(9.0, 28.0);
        (font, (font * 1.46).round())
    }

    /// Whether the 3D scene is the visible center tab.
    #[must_use]
    pub fn scene_visible(&self) -> bool {
        self.code.active.is_none()
    }

    /// Whether the code editor has the keyboard.
    #[must_use]
    pub fn code_has_focus(&self) -> bool {
        self.code.has_focus() && self.edit.is_none() && self.dialog.is_none()
    }

    /// Forwards a key to the code editor; false when it isn't used.
    pub fn code_key(&mut self, key: crate::code::Key, mods: crate::code::Mods) -> bool {
        self.code.hover = None;
        if self.ui_editor_shown() {
            let handled = self.ui_editor_key(key, mods);
            self.drain_code_events();
            return handled;
        }
        if self.material_editor_shown() {
            let handled = self.material_editor_key(key, mods);
            self.drain_code_events();
            return handled;
        }
        let handled = self.code.key(key, mods);
        self.drain_code_events();
        handled
    }

    /// Tab navigation keys (Ctrl+Tab, Ctrl+PgUp/PgDn, Alt+1…9, Ctrl+W,
    /// Ctrl+K chords, …) when the editor isn't focused, e.g. from the Scene
    /// tab; false when the key isn't one of them.
    pub fn code_tab_key(&mut self, key: crate::code::Key, mods: crate::code::Mods) -> bool {
        let handled = self.code.tab_key(key, mods);
        self.drain_code_events();
        handled
    }

    /// Ctrl was released: settles the Ctrl+Tab switcher on its tab.
    pub fn code_commit_switch(&mut self) {
        self.code.commit_switch();
    }

    /// Forwards typed text to the code editor.
    pub fn code_text(&mut self, text: &str) {
        self.code.hover = None;
        if self.ui_editor_shown() || self.material_editor_shown() {
            return;
        }
        let clean: String = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        if !clean.is_empty() {
            self.code.text(&clean);
        }
        self.drain_code_events();
    }

    /// Requests for the language server / script runner raised by the
    /// editor since the last call.
    pub fn take_code_requests(&mut self) -> Vec<CodeEvent> {
        self.drain_code_events();
        std::mem::take(&mut self.code_requests)
    }

    /// Moves editor messages to the Console; keeps the rest for the app.
    pub(super) fn drain_code_events(&mut self) {
        for ev in std::mem::take(&mut self.code.events) {
            match ev {
                CodeEvent::Info(m) => self.log(LogLevel::Info, m),
                CodeEvent::Error(m) => self.log(LogLevel::Error, m),
                other => self.code_requests.push(other),
            }
        }
    }

    /// The Ctrl+Tab switcher: open tabs in most-recently-used order while
    /// Ctrl is held; releasing Ctrl keeps the highlighted one.
    pub(super) fn draw_switcher(&mut self, canvas: &mut Canvas, w: f32) {
        const ROW: f32 = 30.0;
        const MAX_ROWS: usize = 12;
        let Some(sw) = &self.code.switcher else {
            return;
        };
        let (order, index) = (sw.order.clone(), sw.index);
        let width = (w * 0.5).clamp(320.0, 520.0);
        let rows = order.len().min(MAX_ROWS);
        let height = 34.0 + rows as f32 * ROW + 8.0;
        let r = Rect::new(((w - width) / 2.0).round(), 64.0, width, height);
        canvas.fill_rounded(r.x + 3.0, r.y + 8.0, r.w, r.h, 10.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 10.0, color::surface());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 10.0, color::border(), 1.0);
        canvas.text(
            r.x + 16.0,
            r.y + 10.0,
            layout::SMALL_SIZE,
            "Open editors, most recent first",
            color::text_dim(),
        );
        let start = index.saturating_sub(MAX_ROWS - 1);
        for (k, id) in order.iter().enumerate().skip(start).take(MAX_ROWS) {
            let row = Rect::new(
                r.x + 6.0,
                r.y + 34.0 + (k - start) as f32 * ROW,
                r.w - 12.0,
                ROW - 2.0,
            );
            if k == index {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 6.0, color::select());
            }
            let (title, detail, dirty) = match id.and_then(|i| self.code.tabs.get(i)) {
                Some(t) => {
                    let dir = t
                        .path
                        .parent()
                        .map(|d| d.strip_prefix(&self.project_root).unwrap_or(d))
                        .map(|d| d.display().to_string())
                        .unwrap_or_default();
                    (t.title(), dir, t.buf.dirty())
                }
                None => ("Scene".to_string(), "scene view".to_string(), false),
            };
            let fg = if k == index {
                color::text()
            } else {
                color::text_dim()
            };
            let tw = canvas.text(row.x + 12.0, row.y + 7.0, layout::FONT_SIZE, &title, fg);
            if dirty {
                canvas.fill_circle(row.x + 12.0 + tw + 8.0, row.y + row.h / 2.0, 3.0, fg);
            }
            let dx = row.x + 12.0 + tw + if dirty { 20.0 } else { 12.0 };
            canvas.text_clipped(
                dx,
                row.y + 8.5,
                layout::SMALL_SIZE,
                &detail,
                color::text_faint(),
                row.x + row.w - 10.0 - dx,
            );
        }
    }

    /// Runs a script tab context menu command.
    pub(super) fn tab_command(&mut self, cmd: TabCommand) {
        let path = |ui: &Self, i: usize| ui.code.tabs.get(i).map(|t| t.path.clone());
        match cmd {
            TabCommand::Close(i) => self.code.close(i, false),
            TabCommand::CloseOthers(keep) => {
                let keep = path(self, keep);
                self.code.close_where(|_, t| Some(&t.path) != keep.as_ref());
                if let Some(p) = keep {
                    let i = self.code.tabs.iter().position(|t| t.path == p);
                    self.code.activate(i);
                }
            }
            TabCommand::CloseRight(i) => {
                self.code.close_where(|j, _| j > i);
            }
            TabCommand::CloseSaved => {
                self.code.close_where(|_, t| !t.buf.dirty());
            }
            TabCommand::CloseAll => {
                self.code.close_where(|_, _| true);
            }
            TabCommand::CopyPath(i) => {
                if let Some(p) = path(self, i) {
                    self.code.clipboard.set(&p.display().to_string());
                }
            }
            TabCommand::CopyRelativePath(i) => {
                if let Some(p) = path(self, i) {
                    let rel = p.strip_prefix(&self.project_root).unwrap_or(&p);
                    self.code.clipboard.set(&rel.display().to_string());
                }
            }
            TabCommand::Reveal(i) => {
                if let Some(p) = path(self, i) {
                    self.reveal_in_files(&p);
                }
            }
            TabCommand::ReopenClosed => {
                self.code.reopen_closed();
            }
        }
        self.drain_code_events();
    }

    /// The context menu of script tab `i`.
    fn tab_menu(&self, i: usize) -> Vec<MenuItem> {
        let n = self.code.tabs.len();
        let any_saved = self.code.tabs.iter().any(|t| !t.buf.dirty());
        let tab = |c| Action::Ui(UiAction::Tab(c));
        vec![
            MenuItem::new("Close", "Ctrl+W", tab(TabCommand::Close(i)), true),
            MenuItem::new("Close Others", "", tab(TabCommand::CloseOthers(i)), n > 1),
            MenuItem::new(
                "Close to the Right",
                "",
                tab(TabCommand::CloseRight(i)),
                i + 1 < n,
            ),
            MenuItem::new(
                "Close Saved",
                "Ctrl+K U",
                tab(TabCommand::CloseSaved),
                any_saved,
            ),
            MenuItem::new("Close All", "Ctrl+K W", tab(TabCommand::CloseAll), true),
            MenuItem::separator(),
            MenuItem::new(
                "Reopen Closed Tab",
                "Ctrl+Shift+T",
                tab(TabCommand::ReopenClosed),
                true,
            ),
            MenuItem::separator(),
            MenuItem::new("Copy Path", "", tab(TabCommand::CopyPath(i)), true),
            MenuItem::new(
                "Copy Relative Path",
                "",
                tab(TabCommand::CopyRelativePath(i)),
                true,
            ),
            MenuItem::new("Reveal in Files", "", tab(TabCommand::Reveal(i)), true),
        ]
    }

    /// Draws the Scene / script tabs. The Scene tab is pinned; script tabs
    /// scroll (wheel) when they overflow, and the active one is kept in
    /// view. Middle-click closes, right-click opens the tab menu, dragging
    /// reorders.
    #[allow(clippy::too_many_lines)]
    pub(super) fn doc_tabs(&mut self, canvas: &mut Canvas, r: Rect) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.line(
            r.x,
            r.y + r.h - 0.5,
            r.x + r.w,
            r.y + r.h - 0.5,
            color::border(),
            1.0,
        );
        let mut x = r.x + 4.0;
        let scene_active = self.code.active.is_none();
        let w = canvas.measure_text(layout::FONT_SIZE, "Scene") + 24.0;
        let tab = Rect::new(x, r.y + 3.0, w, r.h - 3.0);
        if self
            .tab_chip(canvas, tab, "Scene", scene_active, false, false)
            .0
        {
            self.code.show_scene();
        }
        x += w + 2.0;

        // The scrolling strip of script tabs.
        let strip = Rect::new(x, r.y, (r.x + r.w - x).max(0.0), r.h);
        let titles: Vec<String> = self.code.tabs.iter().map(CodeTab::title).collect();
        let widths: Vec<f32> = titles
            .iter()
            .map(|t| canvas.measure_text(layout::FONT_SIZE, t) + 44.0)
            .collect();
        let starts: Vec<f32> = widths
            .iter()
            .scan(0.0, |acc, w| {
                let s = *acc;
                *acc += w + 2.0;
                Some(s)
            })
            .collect();
        let total = starts.last().zip(widths.last()).map_or(0.0, |(s, w)| s + w);
        let max_scroll = (total - strip.w + 8.0).max(0.0);
        if self.hot(strip) && self.p.scroll != 0.0 {
            self.tab_scroll -= self.p.scroll * 40.0;
        }
        // Keep the active tab in view when it changes (keyboard switching).
        if self.tab_revealed != self.code.active {
            self.tab_revealed = self.code.active;
            if let Some(i) = self.code.active.filter(|i| *i < starts.len()) {
                let (s, w) = (starts[i], widths[i]);
                if s < self.tab_scroll {
                    self.tab_scroll = s;
                } else if s + w > self.tab_scroll + strip.w - 8.0 {
                    self.tab_scroll = s + w - strip.w + 8.0;
                }
            }
        }
        self.tab_scroll = self.tab_scroll.clamp(0.0, max_scroll);

        // Dragging a tab past a neighbour's midpoint swaps them.
        if let Some((from, press_x)) = self.tab_drag {
            if !self.p.down || from >= starts.len() {
                self.tab_drag = None;
            } else if (self.p.x - press_x).abs() > 4.0 {
                // The dragged tab lands after every other tab whose
                // midpoint the pointer has passed.
                let local = self.p.x - strip.x + self.tab_scroll;
                let to = (0..starts.len())
                    .filter(|&j| j != from && local > starts[j] + widths[j] / 2.0)
                    .count();
                if to != from {
                    self.code.reorder(from, to);
                    self.tab_drag = Some((to, press_x));
                    self.tab_revealed = self.code.active;
                }
                self.cursor = CursorHint::Grab;
            }
        }

        let prev_clip = canvas.set_clip(Some((strip.x, strip.y, strip.w, strip.h)));
        let mut activate = None;
        let mut close = None;
        let mut menu = None;
        for i in 0..self.code.tabs.len() {
            let x = strip.x + starts[i] - self.tab_scroll;
            let w = widths[i];
            if x + w < strip.x || x > strip.x + strip.w {
                continue;
            }
            let dirty = self.code.tabs[i].buf.dirty();
            let errors = self.code.tabs[i]
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error);
            // Chips cut off by the strip edge don't take clicks outside it.
            let tab = Rect::new(x, r.y + 3.0, w, r.h - 3.0);
            let inside = self.hot(strip);
            let active = self.code.active == Some(i);
            let (clicked, closed) = self.tab_chip(canvas, tab, &titles[i], active, dirty, errors);
            let hot = inside && self.hot(tab);
            if inside && closed || hot && self.p.middle_pressed {
                close = Some(i);
            } else if inside && clicked {
                activate = Some(i);
                self.tab_drag = Some((i, self.p.x));
            } else if hot && self.p.right_pressed {
                menu = Some(i);
            }
        }
        canvas.restore_clip(prev_clip);
        // Fades hint at tabs scrolled out of view.
        let fade = |canvas: &mut Canvas, x: f32, dir: f32| {
            for k in 0..24u8 {
                let a = 250 - k * 10;
                let xx = x + dir * f32::from(k);
                let mut c = color::surface();
                c.set_alpha(f32::from(a) / 255.0);
                canvas.fill_rect(xx, r.y, 1.0, r.h - 1.0, c);
            }
        };
        if self.tab_scroll > 0.5 {
            fade(canvas, strip.x, 1.0);
        }
        if self.tab_scroll < max_scroll - 0.5 {
            fade(canvas, strip.x + strip.w - 1.0, -1.0);
        }

        if let Some(i) = close {
            self.tab_drag = None;
            self.code.close(i, false);
            self.drain_code_events();
        } else if let Some(i) = activate {
            self.code.activate(Some(i));
        }
        if let Some(i) = menu {
            let items = self.tab_menu(i);
            self.open_menu(self.p.x, self.p.y, items);
        }
    }

    /// One tab chip; returns `(clicked, close clicked)`.
    fn tab_chip(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        title: &str,
        active: bool,
        dirty: bool,
        errors: bool,
    ) -> (bool, bool) {
        let hot = self.hot(r);
        if active {
            canvas.fill_rect(r.x + 6.0, r.y + r.h - 2.0, r.w - 12.0, 2.0, color::accent());
        } else if hot {
            canvas.fill_rounded(
                r.x + 2.0,
                r.y + 2.0,
                r.w - 4.0,
                r.h - 6.0,
                layout::RADIUS,
                color::hover(),
            );
        }
        let fg = if errors {
            color::danger()
        } else if active {
            color::text()
        } else {
            color::text_dim()
        };
        canvas.text(r.x + 10.0, r.y + 6.0, layout::FONT_SIZE, title, fg);
        let mut closed = false;
        if title != "Scene" || dirty {
            let cx = r.x + r.w - 16.0;
            let cy = r.y + r.h / 2.0;
            let close = Rect::new(cx - 8.0, cy - 8.0, 16.0, 16.0);
            let over = self.hot(close);
            if dirty && !over {
                canvas.fill_circle(cx, cy, 3.5, color::text_dim());
            } else if hot || active {
                if over {
                    canvas.fill_rounded(close.x, close.y, close.w, close.h, 3.0, color::raised());
                }
                let c = if over {
                    color::text()
                } else {
                    color::text_dim()
                };
                canvas.line(cx - 3.5, cy - 3.5, cx + 3.5, cy + 3.5, c, 1.3);
                canvas.line(cx + 3.5, cy - 3.5, cx - 3.5, cy + 3.5, c, 1.3);
            }
            if over {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    closed = true;
                }
            }
        }
        if hot && !closed {
            self.cursor = CursorHint::Pointer;
        }
        (hot && self.p.pressed && !closed, closed)
    }

    /// The code editor for the active script tab.
    #[allow(clippy::too_many_lines)]
    pub(super) fn code_view(&mut self, canvas: &mut Canvas, rect: Rect) {
        // UI documents open in the UI editor, or as text under a bar that
        // goes back to it.
        let mut rect = rect;
        if let Some(idx) = self.code.active.filter(|&i| {
            self.code
                .tabs
                .get(i)
                .is_some_and(|t| crate::uidoc::is_ui_doc_path(&t.path))
        }) {
            if self.ui_editor_shown() {
                self.ui_doc_view(canvas, rect, idx);
                return;
            }
            let bar = Rect::new(rect.x, rect.y, rect.w, 30.0);
            self.ui_source_bar(canvas, bar);
            rect = Rect::new(rect.x, rect.y + bar.h, rect.w, (rect.h - bar.h).max(0.0));
        }
        // Materials likewise open in the material editor.
        if let Some(idx) = self.code.active.filter(|&i| {
            self.code
                .tabs
                .get(i)
                .is_some_and(|t| automata_assets::is_material_path(&t.path))
        }) {
            if self.material_editor_shown() {
                self.material_doc_view(canvas, rect, idx);
                return;
            }
            let bar = Rect::new(rect.x, rect.y, rect.w, 30.0);
            self.material_source_bar(canvas, bar);
            rect = Rect::new(rect.x, rect.y + bar.h, rect.w, (rect.h - bar.h).max(0.0));
        }
        // The editor's own zoom: draw in a space scaled by `k` on top of the
        // interface scale, with the pointer and clip band mapped into it, so
        // layout, hit-testing and clipping all stay consistent.
        let k = self.editor_scale();
        if (k - 1.0).abs() < 0.001 {
            self.code_view_scaled(canvas, rect);
            return;
        }
        let (p, clip, base) = (self.p, self.clip, canvas.scale());
        canvas.set_scale(base * k);
        // The canvas clamps its scale; use what it actually applied.
        let k = canvas.scale() / base;
        self.p.x /= k;
        self.p.y /= k;
        self.clip = (clip.0 / k, clip.1 / k);
        let inner = Rect::new(rect.x / k, rect.y / k, rect.w / k, rect.h / k);
        self.code_view_scaled(canvas, inner);
        canvas.set_scale(base);
        self.p = p;
        self.clip = clip;
    }

    /// The script editor's zoom (Settings › Editor › Editor scale).
    #[must_use]
    pub fn editor_scale(&self) -> f32 {
        let k = self.prefs.editor.scale;
        if k.is_finite() {
            k.clamp(0.5, 3.0)
        } else {
            1.0
        }
    }

    #[allow(clippy::too_many_lines)]
    fn code_view_scaled(&mut self, canvas: &mut Canvas, rect: Rect) {
        let (font, lh) = self.code_metrics();
        self.code.settings.clone_from(&self.prefs.editor);
        let opts = self.prefs.editor.clone();
        for tab in &mut self.code.tabs {
            tab.buf.auto_close = opts.auto_close;
        }
        self.code.check_disk();
        self.drain_code_events();
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::viewport_bg());
        let Some(idx) = self.code.active else {
            return;
        };
        if idx >= self.code.tabs.len() {
            self.code.active = None;
            return;
        }
        // Focus follows clicks.
        if self.p.pressed {
            self.code.focused = rect.contains(self.p.x, self.p.y);
            if !self.code.focused {
                self.code.completion = None;
            }
        }

        let mut top = rect.y;
        if let Some(close_idx) = self.code.confirm_close {
            top = self.confirm_close_bar(canvas, rect, close_idx);
        }
        if self.code.find.is_some() {
            top = self.find_bar(canvas, Rect::new(rect.x, top, rect.w, 0.0), idx);
        } else if self.code.goto.is_some() {
            top = self.goto_bar(canvas, Rect::new(rect.x, top, rect.w, 0.0), idx);
        }
        let status = Rect::new(rect.x, rect.y + rect.h - STATUS_H, rect.w, STATUS_H);
        let area = Rect::new(rect.x, top, rect.w, (status.y - top).max(lh));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            self.code.page_lines = ((area.h / lh) as usize).saturating_sub(1).max(1);
        }
        let ui_face = canvas.set_face(crate::canvas::Face::Mono);
        let cell = canvas.cell_width(font);
        let line_count = self.code.tabs[idx].buf.line_count();
        let digits = line_count.to_string().len().max(3);
        let gutter_w = digits as f32 * cell + 26.0;
        let text_x = area.x + gutter_w + 8.0;
        let visible = area.h / lh;

        // --- scrolling ---
        {
            let hot = self.hot(area);
            let tab = &mut self.code.tabs[idx];
            if hot && self.p.scroll != 0.0 && self.p.ctrl {
                // Ctrl+wheel zooms the editor (Settings › Editor scale).
                let k = self.prefs.editor.scale;
                let step = if self.p.scroll > 0.0 { 0.1 } else { -0.1 };
                self.prefs.editor.scale = ((k + step) * 10.0).round().clamp(5.0, 30.0) / 10.0;
                self.prefs_dirty = true;
            } else if hot && self.p.scroll != 0.0 {
                if self.p.shift {
                    tab.scroll_x = (tab.scroll_x - self.p.scroll * cell * 6.0).max(0.0);
                } else {
                    tab.scroll -= self.p.scroll * 3.0;
                }
            }
            if tab.reveal {
                tab.reveal = false;
                let line = tab.buf.cursor.line as f32;
                if line < tab.scroll + 1.0 {
                    tab.scroll = (line - 1.0).max(0.0);
                } else if line > tab.scroll + visible - 3.0 {
                    tab.scroll = line - visible + 3.0;
                }
                let cx =
                    visual_col(tab.buf.line(tab.buf.cursor.line), tab.buf.cursor.col) as f32 * cell;
                let view_w = area.w - gutter_w - 24.0;
                if cx < tab.scroll_x {
                    tab.scroll_x = (cx - cell * 4.0).max(0.0);
                } else if cx > tab.scroll_x + view_w {
                    tab.scroll_x = cx - view_w + cell * 4.0;
                }
            }
            let max_scroll = (line_count as f32 - 3.0).max(0.0);
            tab.scroll = tab.scroll.clamp(0.0, max_scroll);
        }
        let geom = TextGeom {
            area,
            text_x,
            cell,
            scroll: self.code.tabs[idx].scroll,
            scroll_x: self.code.tabs[idx].scroll_x,
            lh,
        };

        // --- mouse ---
        self.code_mouse(area, gutter_w, geom, idx);

        // --- draw ---
        let clip = canvas.set_clip(Some((area.x, area.y, area.w, area.h)));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let first = geom.scroll.floor() as usize;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let last = ((geom.scroll + visible).ceil() as usize + 1).min(line_count);
        let focused = self.code.focused;
        let find_query = self
            .code
            .find
            .as_ref()
            .filter(|f| !f.query.is_empty())
            .map(|f| (f.query.clone(), f.case, f.word));
        let tab = &mut self.code.tabs[idx];
        let states: Vec<syntax::LineState> = tab.line_states()[first..last].to_vec();
        let buf = &tab.buf;
        let cursor = buf.cursor;
        let selection = buf.selection();
        let carets = buf.carets();

        // Current line.
        if selection.is_none() && opts.highlight_line {
            let y = geom.y_of(cursor.line);
            canvas.fill_rect(area.x, y, area.w, lh, rgb(15, 15, 15));
        }
        // Indentation guides (blank lines continue the block below them).
        if opts.indent_guides {
            let unit = crate::code::buffer::INDENT;
            let mut next_indent = 0usize;
            let mut guides = vec![0usize; last - first];
            let scan_end = (last + 40).min(line_count);
            for line_idx in (first..scan_end).rev() {
                let l = buf.line(line_idx);
                let indent = if l.trim().is_empty() {
                    next_indent
                } else {
                    visual_col(l, crate::code::buffer::indent_of(l).chars().count())
                };
                next_indent = indent;
                if line_idx < last {
                    guides[line_idx - first] = indent / unit;
                }
            }
            for (k, levels) in guides.iter().enumerate() {
                let y = geom.y_of(first + k);
                for level in 0..*levels {
                    let x = (text_x - geom.scroll_x + (level * unit) as f32 * cell).round() + 0.5;
                    if x > text_x - 1.0 {
                        canvas.line(x, y, x, y + lh, rgb(30, 30, 29), 1.0);
                    }
                }
            }
        }
        // Find matches, then occurrences of the word under the caret.
        let mut marks: Vec<((Pos, Pos), Color)> = Vec::new();
        if let Some((q, case, word)) = &find_query {
            if line_count < 50_000 {
                for m in buf.find_all(q, *case, *word) {
                    if m.0.line >= first && m.0.line < last {
                        marks.push((m, rgba(236, 190, 92, 70)));
                    }
                }
            }
        } else if selection.is_none() {
            let (a, b) = buf.word_range(cursor);
            if b.col > a.col + 1 && line_count < 20_000 {
                let w = buf.text_range(a, b);
                for m in buf.find_all(&w, true, true) {
                    if m.0.line >= first && m.0.line < last && m.0 != a {
                        marks.push((m, rgba(242, 240, 235, 26)));
                    }
                }
            }
        }
        for ((a, b), c) in &marks {
            let x0 = geom.x_of(buf, *a);
            let x1 = geom.x_of(buf, *b);
            canvas.fill_rounded(
                x0,
                geom.y_of(a.line) + 1.0,
                (x1 - x0).max(2.0),
                lh - 2.0,
                2.0,
                *c,
            );
        }
        // Selections (every caret's).
        let sel = if focused {
            rgb(52, 52, 50)
        } else {
            rgb(32, 32, 31)
        };
        for (a, b) in carets
            .iter()
            .filter_map(crate::code::buffer::Caret::selection)
        {
            for line in a.line.max(first)..=b.line.min(last.saturating_sub(1)) {
                let start = if line == a.line { a.col } else { 0 };
                let l = buf.line(line);
                let end_col = if line == b.line {
                    b.col
                } else {
                    l.chars().count()
                };
                let x0 = geom.x_of(buf, Pos::new(line, start));
                let mut x1 = geom.x_of(buf, Pos::new(line, end_col));
                if line != b.line {
                    x1 += cell * 0.6;
                }
                canvas.fill_rect(x0, geom.y_of(line), (x1 - x0).max(0.0), lh, sel);
            }
        }
        // Matching bracket.
        if let Some((p, q)) = buf.matching_bracket() {
            for m in [p, q] {
                if m.line >= first && m.line < last {
                    let x = geom.x_of(buf, m);
                    canvas.stroke_rect(
                        x,
                        geom.y_of(m.line) + 1.0,
                        cell,
                        lh - 2.0,
                        rgba(242, 240, 235, 110),
                        1.0,
                    );
                }
            }
        }
        // Text.
        for (k, line_idx) in (first..last).enumerate() {
            let line = buf.line(line_idx);
            let y = geom.y_of(line_idx) + ((lh - font) / 2.0 - 1.0).max(0.0);
            if opts.show_whitespace {
                let mut vcol = 0usize;
                for ch in line.chars() {
                    let x = text_x - geom.scroll_x + vcol as f32 * cell;
                    let mid = geom.y_of(line_idx) + lh / 2.0;
                    if ch == ' ' {
                        canvas.fill_rect(
                            x + cell / 2.0 - 1.0,
                            mid - 1.0,
                            2.0,
                            2.0,
                            rgb(58, 58, 56),
                        );
                        vcol += 1;
                    } else if ch == '\t' {
                        let next =
                            (vcol / crate::code::buffer::INDENT + 1) * crate::code::buffer::INDENT;
                        let x1 = text_x - geom.scroll_x + next as f32 * cell - 3.0;
                        canvas.line(x + 2.0, mid, x1, mid, rgb(58, 58, 56), 1.0);
                        vcol = next;
                    } else {
                        vcol += 1;
                    }
                }
            }
            let (spans, _) = syntax::highlight(line, states[k]);
            let mut vcol = 0usize;
            for (range, tok) in spans {
                let piece = &line[range];
                let x = text_x - geom.scroll_x + vcol as f32 * cell;
                let mut shown = String::with_capacity(piece.len());
                for ch in piece.chars() {
                    if ch == '\t' {
                        let next =
                            (vcol / crate::code::buffer::INDENT + 1) * crate::code::buffer::INDENT;
                        shown.extend(std::iter::repeat_n(' ', next - vcol));
                        vcol = next;
                    } else {
                        shown.push(ch);
                        vcol += 1;
                    }
                }
                let end_x = text_x - geom.scroll_x + vcol as f32 * cell;
                if end_x < area.x || x > area.x + area.w || shown.trim().is_empty() {
                    continue;
                }
                canvas.text(x, y, font, &shown, tok_color(tok));
            }
        }
        // Diagnostics: squiggles.
        for d in &tab.diagnostics {
            for line in d.start.line.max(first)..=d.end.line.min(last.saturating_sub(1)) {
                let l = buf.line(line);
                let len = l.chars().count();
                let a = if line == d.start.line { d.start.col } else { 0 };
                let mut b = if line == d.end.line { d.end.col } else { len };
                if b <= a {
                    b = (a + 1).max(len.min(a + 1));
                }
                let x0 = geom.x_of(buf, Pos::new(line, a));
                let x1 = geom.x_of(buf, Pos::new(line, b)).max(x0 + cell);
                let y = geom.y_of(line) + lh - 2.5;
                let mut pts = Vec::new();
                let mut x = x0;
                let mut up = true;
                while x < x1 {
                    pts.push((x, if up { y - 1.5 } else { y + 1.0 }));
                    x += 2.5;
                    up = !up;
                }
                pts.push((x1, y));
                canvas.polyline(&pts, severity_color(d.severity), 1.0);
            }
        }
        // Carets.
        if focused && self.code.focus == Focus::Text {
            for c in &carets {
                let p = c.cursor;
                if p.line >= first && p.line < last {
                    let x = geom.x_of(buf, p);
                    canvas.fill_rect(
                        x - 0.5,
                        geom.y_of(p.line) + 1.0,
                        2.0,
                        lh - 2.0,
                        color::accent_soft(),
                    );
                }
            }
        }
        canvas.restore_clip(clip);

        // Gutter.
        canvas.fill_rect(area.x, area.y, gutter_w, area.h, rgb(3, 3, 3));
        canvas.line(
            area.x + gutter_w,
            area.y,
            area.x + gutter_w,
            area.y + area.h,
            rgb(24, 24, 23),
            1.0,
        );
        let clip = canvas.set_clip(Some((area.x, area.y, gutter_w, area.h)));
        for line_idx in first..last {
            let y = geom.y_of(line_idx);
            let num = (line_idx + 1).to_string();
            let c = if line_idx == cursor.line {
                color::text()
            } else {
                color::text_faint()
            };
            let nx = area.x + gutter_w - 10.0 - num.len() as f32 * cell;
            canvas.text(nx, y + 2.0, font - 1.0, &num, c);
            if let Some(sev) = tab.line_severity(line_idx) {
                canvas.fill_circle(area.x + 8.0, y + lh / 2.0, 3.0, severity_color(sev));
            }
        }
        canvas.restore_clip(clip);

        // Scrollbar.
        let content = (line_count as f32 + visible - 3.0).max(1.0);
        if line_count as f32 > visible - 3.0 {
            let track = Rect::new(area.x + area.w - 8.0, area.y, 8.0, area.h);
            let th = (area.h * visible / content).max(24.0);
            let ty = track.y
                + (area.h - th) * (geom.scroll / (content - visible).max(1.0)).clamp(0.0, 1.0);
            canvas.fill_rounded(track.x + 2.0, ty, 4.0, th, 2.0, rgba(160, 158, 152, 60));
        }

        canvas.set_face(ui_face);
        self.code_status(canvas, status, idx);
        canvas.set_face(crate::canvas::Face::Mono);
        self.completion_popup(canvas, area, geom, idx);
        self.code_tooltips(canvas, area, geom, idx);
        canvas.set_face(ui_face);
    }

    fn code_mouse(&mut self, area: Rect, gutter_w: f32, geom: TextGeom, idx: usize) {
        let p = self.p;
        let over_text = self.hot(area) && p.x > area.x + gutter_w && p.x < area.x + area.w - 8.0;
        let over_gutter = self.hot(area) && p.x <= area.x + gutter_w;
        if over_text {
            self.cursor = CursorHint::Text;
        }
        self.hover_dwell(over_text && !p.down, geom, idx);
        let tab = &mut self.code.tabs[idx];
        if p.pressed
            && (over_text || over_gutter)
            && self.code_popup.is_none_or(|r| !r.contains(p.x, p.y))
        {
            self.code.focus = Focus::Text;
            self.code.completion = None;
            self.code.hover = None;
            let pos = geom.pos_at(&tab.buf, p.x, p.y);
            let now = std::time::Instant::now();
            let clicks = match self.code_clicks {
                Some((t, n, line))
                    if p.double_click
                        || (now - t).as_secs_f32() < 0.45 && line == pos.line && n >= 2 =>
                {
                    n + 1
                }
                _ => 1,
            };
            let clicks = if p.double_click {
                clicks.max(2)
            } else {
                clicks
            };
            self.code_clicks = Some((now, clicks, pos.line));
            // Alt+click adds (or removes) a caret, like VS Code.
            if p.alt && over_text && clicks == 1 {
                tab.buf.toggle_caret(pos);
                self.code_drag = None;
                return;
            }
            tab.buf.single();
            if over_gutter || clicks >= 3 {
                tab.buf.select_line(pos.line);
                self.code_drag = Some(true);
            } else if clicks == 2 {
                tab.buf.select_word(pos);
                self.code_drag = None;
            } else {
                tab.buf.move_to(pos, p.shift);
                self.code_drag = Some(false);
                if p.ctrl {
                    self.code
                        .events
                        .push(CodeEvent::Definition(tab.path.clone(), pos));
                }
            }
        } else if p.down {
            if let Some(lines) = self.code_drag {
                // Drag-select, auto-scrolling past the edges.
                if p.y < area.y {
                    tab.scroll = (tab.scroll - 0.5).max(0.0);
                } else if p.y > area.y + area.h {
                    tab.scroll += 0.5;
                }
                let y = p.y.clamp(area.y, area.y + area.h - 1.0);
                let pos = geom.pos_at(&tab.buf, p.x.max(area.x + gutter_w + 1.0), y);
                if lines {
                    let anchor_line = tab
                        .buf
                        .anchor
                        .map_or(pos.line, |a| a.line.min(tab.buf.cursor.line));
                    let (a, b) = if pos.line >= anchor_line {
                        (Pos::new(anchor_line, 0), Pos::new(pos.line + 1, 0))
                    } else {
                        (tab.buf.anchor.unwrap_or(pos), Pos::new(pos.line, 0))
                    };
                    tab.buf.select(a, b);
                } else {
                    tab.buf.move_to(pos, true);
                }
            }
        } else {
            self.code_drag = None;
        }
    }

    /// Resting the mouse on a word asks the language server for hover
    /// info; moving away hides it.
    fn hover_dwell(&mut self, active: bool, geom: TextGeom, idx: usize) {
        let tab = &self.code.tabs[idx];
        let pos = geom.pos_at(&tab.buf, self.p.x, self.p.y);
        let (a, b) = tab.buf.word_range(pos);
        let on_word = active && b.col > a.col && pos.col < b.col;
        let now = std::time::Instant::now();
        match self.code_dwell {
            Some((since, at, sent)) if on_word && at == a => {
                if !sent && (now - since).as_secs_f32() > 0.6 {
                    let path = tab.path.clone();
                    if !self.code.api_hover(a) {
                        self.code.events.push(CodeEvent::Hover(path, a));
                    }
                    self.code_dwell = Some((since, at, true));
                }
            }
            _ => {
                if self.code_dwell.is_some_and(|(_, _, sent)| sent) {
                    self.code.hover = None;
                }
                self.code_dwell = on_word.then_some((now, a, false));
            }
        }
    }

    fn code_status(&mut self, canvas: &mut Canvas, r: Rect, idx: usize) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.line(r.x, r.y + 0.5, r.x + r.w, r.y + 0.5, color::border(), 1.0);
        let tab = &self.code.tabs[idx];
        let c = tab.buf.cursor;
        let errors = tab
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = tab
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        let mut left = format!("Ln {}, Col {}", c.line + 1, c.col + 1);
        if tab.buf.multi() {
            // Like VS Code: the caret count, not the primary's selection.
            left = format!("{left}  ({} selections)", tab.buf.carets().len());
        } else if let Some(sel) = tab.buf.selected_text() {
            left = format!("{left}  ({} selected)", sel.chars().count());
        }
        canvas.text(
            r.x + 10.0,
            r.y + 4.0,
            layout::SMALL_SIZE,
            &left,
            color::text_dim(),
        );
        let mut x = r.x + 190.0;
        if errors > 0 {
            x += canvas.text(
                x,
                r.y + 4.0,
                layout::SMALL_SIZE,
                &format!("{errors} error{}", if errors == 1 { "" } else { "s" }),
                color::danger(),
            ) + 12.0;
        }
        if warnings > 0 {
            canvas.text(
                x,
                r.y + 4.0,
                layout::SMALL_SIZE,
                &format!("{warnings} warning{}", if warnings == 1 { "" } else { "s" }),
                color::warn(),
            );
        }
        let right = format!(
            "{}Spaces: 4   UTF-8   {}   Python{}",
            if tab.buf.dirty() { "Unsaved   " } else { "" },
            if tab.buf.crlf { "CRLF" } else { "LF" },
            self.code
                .server
                .as_ref()
                .map_or_else(String::new, |s| format!(" · {s}"))
        );
        let w = canvas.measure_text(layout::SMALL_SIZE, &right);
        canvas.text(
            r.x + r.w - w - 10.0,
            r.y + 4.0,
            layout::SMALL_SIZE,
            &right,
            color::text_dim(),
        );
    }

    /// A one-line input box; returns whether it was clicked.
    fn code_field(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        text: &str,
        placeholder: &str,
        focused: bool,
    ) -> bool {
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::field());
        if focused {
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.0);
        }
        let clip = canvas.set_clip(Some((r.x + 2.0, r.y, r.w - 4.0, r.h)));
        if text.is_empty() {
            canvas.text(
                r.x + 6.0,
                r.y + 4.0,
                layout::FONT_SIZE,
                placeholder,
                color::text_dim(),
            );
        }
        let w = canvas.measure_text(layout::FONT_SIZE, text);
        let shift = (w - (r.w - 14.0)).max(0.0);
        canvas.text(
            r.x + 6.0 - shift,
            r.y + 4.0,
            layout::FONT_SIZE,
            text,
            color::text(),
        );
        if focused {
            canvas.fill_rect(
                r.x + 6.0 - shift + w + 1.0,
                r.y + 4.0,
                1.5,
                r.h - 8.0,
                color::accent_soft(),
            );
        }
        canvas.restore_clip(clip);
        let hot = self.hot(r);
        if hot {
            self.cursor = CursorHint::Text;
        }
        hot && self.p.pressed
    }

    fn small_toggle(&mut self, canvas: &mut Canvas, r: Rect, label: &str, on: bool) -> bool {
        let style = if on {
            ButtonStyle::Active
        } else {
            ButtonStyle::Normal
        };
        self.button(canvas, r, label, style)
    }

    #[allow(clippy::too_many_lines)]
    fn find_bar(&mut self, canvas: &mut Canvas, at: Rect, idx: usize) -> f32 {
        let Some(bar) = self.code.find.clone() else {
            return at.y;
        };
        let rows = if bar.show_replace { 2.0 } else { 1.0 };
        let r = Rect::new(at.x, at.y, at.w, BAR_ROW * rows + 6.0);
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.line(
            r.x,
            r.y + r.h - 0.5,
            r.x + r.w,
            r.y + r.h - 0.5,
            color::border(),
            1.0,
        );
        let fw = (r.w * 0.42).clamp(160.0, 380.0);
        let y = r.y + 5.0;
        let focus = self.code.focus;
        if self.code_field(
            canvas,
            Rect::new(r.x + 10.0, y, fw, 22.0),
            &bar.query,
            "Find",
            focus == Focus::Find,
        ) {
            self.code.focus = Focus::Find;
        }
        let mut x = r.x + 16.0 + fw;
        let tab = &self.code.tabs[idx];
        let matches = tab.buf.find_all(&bar.query, bar.case, bar.word);
        let current = tab
            .buf
            .selection()
            .and_then(|s| matches.iter().position(|m| *m == s))
            .map_or(0, |i| i + 1);
        if self.small_toggle(canvas, Rect::new(x, y, 30.0, 22.0), "Aa", bar.case) {
            if let Some(f) = &mut self.code.find {
                f.case = !f.case;
            }
        }
        x += 34.0;
        if self.small_toggle(canvas, Rect::new(x, y, 30.0, 22.0), "ab", bar.word) {
            if let Some(f) = &mut self.code.find {
                f.word = !f.word;
            }
        }
        x += 34.0;
        if self.button(
            canvas,
            Rect::new(x, y, 26.0, 22.0),
            "<",
            ButtonStyle::Normal,
        ) {
            self.code.find_step(true, false);
        }
        x += 30.0;
        if self.button(
            canvas,
            Rect::new(x, y, 26.0, 22.0),
            ">",
            ButtonStyle::Normal,
        ) {
            self.code.find_step(false, false);
        }
        x += 34.0;
        let count = if bar.query.is_empty() {
            String::new()
        } else if matches.is_empty() {
            "No results".to_string()
        } else {
            format!("{current} of {}", matches.len())
        };
        let count_color = if !bar.query.is_empty() && matches.is_empty() {
            color::danger()
        } else {
            color::text_dim()
        };
        canvas.text(x, y + 4.0, layout::SMALL_SIZE, &count, count_color);
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 30.0, y, 22.0, 22.0),
            "x",
            ButtonStyle::Normal,
        ) {
            self.code.find = None;
            self.code.focus = Focus::Text;
            return r.y + r.h;
        }
        if !bar.show_replace
            && self.button(
                canvas,
                Rect::new(r.x + r.w - 110.0, y, 74.0, 22.0),
                "Replace…",
                ButtonStyle::Normal,
            )
        {
            if let Some(f) = &mut self.code.find {
                f.show_replace = true;
            }
            self.code.focus = Focus::Replace;
        }
        if bar.show_replace {
            let y = y + BAR_ROW;
            if self.code_field(
                canvas,
                Rect::new(r.x + 10.0, y, fw, 22.0),
                &bar.replace,
                "Replace",
                focus == Focus::Replace,
            ) {
                self.code.focus = Focus::Replace;
            }
            let x = r.x + 16.0 + fw;
            if self.button(
                canvas,
                Rect::new(x, y, 70.0, 22.0),
                "Replace",
                ButtonStyle::Normal,
            ) {
                self.code.replace_one();
            }
            if self.button(
                canvas,
                Rect::new(x + 74.0, y, 50.0, 22.0),
                "All",
                ButtonStyle::Normal,
            ) {
                self.code.replace_all();
                self.drain_code_events();
            }
        }
        r.y + r.h
    }

    fn goto_bar(&mut self, canvas: &mut Canvas, at: Rect, idx: usize) -> f32 {
        let r = Rect::new(at.x, at.y, at.w, BAR_ROW + 4.0);
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.line(
            r.x,
            r.y + r.h - 0.5,
            r.x + r.w,
            r.y + r.h - 0.5,
            color::border(),
            1.0,
        );
        canvas.text(
            r.x + 10.0,
            r.y + 9.0,
            layout::FONT_SIZE,
            "Go to line",
            color::text_dim(),
        );
        let text = self.code.goto.clone().unwrap_or_default();
        let n = self.code.tabs[idx].buf.line_count();
        if self.code_field(
            canvas,
            Rect::new(r.x + 96.0, r.y + 5.0, 120.0, 22.0),
            &text,
            &format!("1 – {n}"),
            self.code.focus == Focus::GoTo,
        ) {
            self.code.focus = Focus::GoTo;
        }
        canvas.text(
            r.x + 226.0,
            r.y + 9.0,
            layout::SMALL_SIZE,
            "Enter to jump, Esc to cancel",
            color::text_dim(),
        );
        r.y + r.h
    }

    fn confirm_close_bar(&mut self, canvas: &mut Canvas, rect: Rect, idx: usize) -> f32 {
        let r = Rect::new(rect.x, rect.y, rect.w, BAR_ROW + 6.0);
        canvas.fill_rect(r.x, r.y, r.w, r.h, rgb(48, 40, 24));
        let name = self
            .code
            .tabs
            .get(idx)
            .map(crate::code::CodeTab::title)
            .unwrap_or_default();
        canvas.text(
            r.x + 12.0,
            r.y + 10.0,
            layout::FONT_SIZE,
            &format!("Save changes to {name} before closing?"),
            color::warn(),
        );
        let y = r.y + 6.0;
        let mut x = r.x + r.w - 10.0;
        x -= 70.0;
        if self.button(
            canvas,
            Rect::new(x, y, 66.0, 24.0),
            "Cancel",
            ButtonStyle::Normal,
        ) {
            self.code.confirm_close = None;
        }
        x -= 96.0;
        if self.button(
            canvas,
            Rect::new(x, y, 92.0, 24.0),
            "Don't Save",
            ButtonStyle::Normal,
        ) {
            self.code.close(idx, true);
            self.drain_code_events();
            return r.y + r.h;
        }
        x -= 70.0;
        if self.button(
            canvas,
            Rect::new(x, y, 66.0, 24.0),
            "Save",
            ButtonStyle::Accent,
        ) {
            if self.code.save(idx) {
                self.code.close(idx, true);
            }
            self.drain_code_events();
        }
        r.y + r.h
    }

    fn completion_popup(&mut self, canvas: &mut Canvas, area: Rect, geom: TextGeom, idx: usize) {
        self.code_popup = None;
        let prefix = self.code.completion_prefix();
        let Some(comp) = self.code.completion.clone() else {
            return;
        };
        let items = comp.visible(&prefix);
        if items.is_empty() {
            return;
        }
        let buf = &self.code.tabs[idx].buf;
        let anchor_x = geom.x_of(buf, comp.anchor);
        let below = geom.y_of(buf.cursor.line) + geom.lh;
        let rows = items.len().min(10);
        let row_h = 21.0;
        let w = 380.0_f32.min(area.w - 20.0);
        let h = rows as f32 * row_h + 6.0;
        let y = if below + h > area.y + area.h {
            geom.y_of(buf.cursor.line) - h
        } else {
            below
        };
        let x = anchor_x.min(area.x + area.w - w - 10.0).max(area.x + 4.0);
        let r = Rect::new(x, y, w, h);
        self.code_popup = Some(r);
        canvas.fill_rounded(r.x + 2.0, r.y + 3.0, r.w, r.h, 5.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 5.0, color::raised());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 5.0, color::border(), 1.0);
        let start = comp.selected.saturating_sub(rows - 1);
        let mut accept = None;
        for (k, item) in items.iter().enumerate().skip(start).take(rows) {
            let ry = r.y + 3.0 + (k - start) as f32 * row_h;
            let row = Rect::new(r.x + 3.0, ry, r.w - 6.0, row_h);
            if k == comp.selected {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::select());
            } else if self.hot(row) {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::hover());
            }
            draw_kind_icon(canvas, item.kind, row.x + 12.0, row.y + row_h / 2.0);
            let lw = canvas.text(
                row.x + 26.0,
                row.y + 3.0,
                layout::FONT_SIZE,
                &item.label,
                color::text(),
            );
            if !item.detail.is_empty() {
                let dx = row.x + 34.0 + lw;
                canvas.text_clipped(
                    dx,
                    row.y + 4.0,
                    layout::SMALL_SIZE,
                    &item.detail,
                    color::text_dim(),
                    (row.x + row.w - dx - 6.0).max(0.0),
                );
            }
            if self.hot(row) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    accept = Some(k);
                }
            }
        }
        if let Some(k) = accept {
            if let Some(c) = &mut self.code.completion {
                c.selected = k;
            }
            self.code.accept_completion();
            self.code.focused = true;
        }
    }

    fn code_tooltips(&mut self, canvas: &mut Canvas, area: Rect, geom: TextGeom, idx: usize) {
        let tab = &self.code.tabs[idx];
        let mut tip: Option<(f32, f32, String, Color)> = None;
        // Diagnostics under the mouse.
        if self.hot(area) && self.code_drag.is_none() {
            let pos = geom.pos_at(&tab.buf, self.p.x, self.p.y);
            let line_y = geom.y_of(pos.line);
            if let Some(d) = tab.diagnostics.iter().find(|d| {
                let after = (pos.line, pos.col) >= (d.start.line, d.start.col);
                let before = (pos.line, pos.col) <= (d.end.line, d.end.col.max(d.start.col + 1));
                after && before && (self.p.y - line_y) < geom.lh
            }) {
                tip = Some((
                    self.p.x,
                    line_y + geom.lh + 2.0,
                    d.message.clone(),
                    severity_color(d.severity),
                ));
            }
        }
        // Language-server hover text at a position.
        if tip.is_none() {
            if let Some((p, text)) = &self.code.hover {
                let y = geom.y_of(p.line) + geom.lh + 2.0;
                if y > area.y && y < area.y + area.h {
                    tip = Some((geom.x_of(&tab.buf, *p), y, text.clone(), color::accent()));
                }
            }
        }
        let Some((x, y, text, accent)) = tip else {
            return;
        };
        let lines: Vec<String> = wrap(&text, 80).into_iter().take(14).collect();
        let w = lines
            .iter()
            .map(|l| canvas.measure_text(layout::SMALL_SIZE, l))
            .fold(0.0, f32::max)
            + 20.0;
        let h = lines.len() as f32 * 16.0 + 10.0;
        let x = x.min(area.x + area.w - w - 6.0).max(area.x + 4.0);
        let y = if y + h > area.y + area.h {
            (y - h - geom.lh - 4.0).max(area.y)
        } else {
            y
        };
        canvas.fill_rounded(x + 2.0, y + 3.0, w, h, 4.0, color::shadow());
        canvas.fill_rounded(x, y, w, h, 4.0, color::raised());
        canvas.fill_rect(x, y + 4.0, 2.0, h - 8.0, accent);
        canvas.stroke_rounded(x, y, w, h, 4.0, color::border(), 1.0);
        for (i, l) in lines.iter().enumerate() {
            canvas.text(
                x + 10.0,
                y + 5.0 + i as f32 * 16.0,
                layout::SMALL_SIZE,
                l,
                color::text(),
            );
        }
    }
}

/// Wraps text to `width` columns (keeps explicit line breaks).
/// The accent color of a completion kind's icon.
fn kind_color(kind: CompletionKind) -> Color {
    match kind {
        CompletionKind::Function | CompletionKind::Method => tok_color(Tok::Function),
        CompletionKind::Class => tok_color(Tok::Class),
        CompletionKind::Variable => rgb(138, 190, 245),
        CompletionKind::Property => tok_color(Tok::Str),
        CompletionKind::Constant => tok_color(Tok::Constant),
        CompletionKind::Keyword | CompletionKind::Text => rgb(150, 148, 142),
        CompletionKind::Module | CompletionKind::Snippet => rgb(205, 202, 195),
        CompletionKind::Key => color::warn(),
        CompletionKind::Entity => color::camera_icon(),
    }
}

/// Draws a 14 px symbol icon for a completion kind centered on `(cx, cy)`
/// (VS Code-style: shapes, not letters).
fn draw_kind_icon(canvas: &mut Canvas, kind: CompletionKind, cx: f32, cy: f32) {
    let c = kind_color(kind);
    let w = 1.3;
    match kind {
        // A cube: functions and methods.
        CompletionKind::Function | CompletionKind::Method => {
            let top = [
                (cx, cy - 6.0),
                (cx + 5.5, cy - 3.0),
                (cx, cy),
                (cx - 5.5, cy - 3.0),
            ];
            if kind == CompletionKind::Method {
                canvas.fill_polygon(&top, rgba(255, 199, 153, 90));
            }
            canvas.polyline(
                &[
                    (cx - 5.5, cy - 3.0),
                    (cx, cy - 6.0),
                    (cx + 5.5, cy - 3.0),
                    (cx + 5.5, cy + 3.0),
                    (cx, cy + 6.0),
                    (cx - 5.5, cy + 3.0),
                    (cx - 5.5, cy - 3.0),
                    (cx, cy),
                    (cx + 5.5, cy - 3.0),
                ],
                c,
                w,
            );
            canvas.line(cx, cy, cx, cy + 6.0, c, w);
        }
        // Two linked boxes: classes and types.
        CompletionKind::Class => {
            canvas.stroke_rect(cx - 6.0, cy - 5.5, 5.0, 5.0, c, w);
            canvas.stroke_rect(cx + 1.0, cy + 0.5, 5.0, 5.0, c, w);
            canvas.polyline(
                &[
                    (cx - 3.5, cy - 0.5),
                    (cx - 3.5, cy + 3.0),
                    (cx + 1.0, cy + 3.0),
                ],
                c,
                w,
            );
        }
        // [x]: variables.
        CompletionKind::Variable => {
            canvas.polyline(
                &[
                    (cx - 3.5, cy - 5.5),
                    (cx - 6.0, cy - 5.5),
                    (cx - 6.0, cy + 5.5),
                    (cx - 3.5, cy + 5.5),
                ],
                c,
                w,
            );
            canvas.polyline(
                &[
                    (cx + 3.5, cy - 5.5),
                    (cx + 6.0, cy - 5.5),
                    (cx + 6.0, cy + 5.5),
                    (cx + 3.5, cy + 5.5),
                ],
                c,
                w,
            );
            canvas.fill_rounded(cx - 2.5, cy - 1.5, 5.0, 3.0, 1.0, c);
        }
        // A tag: properties, fields and attributes.
        CompletionKind::Property => {
            canvas.polyline(
                &[
                    (cx - 6.0, cy - 4.0),
                    (cx + 1.5, cy - 4.0),
                    (cx + 6.0, cy),
                    (cx + 1.5, cy + 4.0),
                    (cx - 6.0, cy + 4.0),
                    (cx - 6.0, cy - 4.0),
                ],
                c,
                w,
            );
            canvas.fill_circle(cx - 2.5, cy, 1.4, c);
        }
        // A box with an equals sign: constants.
        CompletionKind::Constant => {
            canvas.stroke_rounded(cx - 6.0, cy - 5.0, 12.0, 10.0, 2.0, c, w);
            canvas.line(cx - 3.0, cy - 1.5, cx + 3.0, cy - 1.5, c, w);
            canvas.line(cx - 3.0, cy + 1.5, cx + 3.0, cy + 1.5, c, w);
        }
        // Stacked lines: keywords and plain words.
        CompletionKind::Keyword | CompletionKind::Text => {
            canvas.line(cx - 6.0, cy - 4.0, cx + 6.0, cy - 4.0, c, w);
            canvas.line(cx - 6.0, cy, cx + 3.0, cy, c, w);
            canvas.line(cx - 6.0, cy + 4.0, cx + 5.0, cy + 4.0, c, w);
            if kind == CompletionKind::Keyword {
                canvas.fill_rect(cx + 4.5, cy - 1.0, 2.0, 2.0, c);
            }
        }
        // { }: modules.
        CompletionKind::Module => {
            for dir in [-1.0f32, 1.0] {
                let x = cx + dir * 5.5;
                let xi = cx + dir * 3.5;
                canvas.polyline(
                    &[
                        (xi, cy - 6.0),
                        (x - dir * 0.5, cy - 5.0),
                        (x - dir * 0.5, cy - 1.5),
                        (x + dir * 0.5, cy),
                        (x - dir * 0.5, cy + 1.5),
                        (x - dir * 0.5, cy + 5.0),
                        (xi, cy + 6.0),
                    ],
                    c,
                    w,
                );
            }
        }
        // A dashed square: snippets.
        CompletionKind::Snippet => {
            for (x0, y0, x1, y1) in [
                (-6.0, -6.0, -2.0, -6.0),
                (2.0, -6.0, 6.0, -6.0),
                (-6.0, 6.0, -2.0, 6.0),
                (2.0, 6.0, 6.0, 6.0),
                (-6.0, -6.0, -6.0, 6.0),
                (6.0, -6.0, 6.0, 6.0),
            ] {
                canvas.line(cx + x0, cy + y0, cx + x1, cy + y1, c, w);
            }
        }
        // A keycap: input key names.
        CompletionKind::Key => {
            canvas.stroke_rounded(cx - 6.0, cy - 6.0, 12.0, 12.0, 2.5, c, w);
            canvas.line(cx - 6.0, cy + 2.5, cx + 6.0, cy + 2.5, c, 1.0);
            canvas.fill_rect(cx - 2.0, cy - 3.0, 4.0, 2.0, c);
        }
        // A solid cube: scene entities.
        CompletionKind::Entity => {
            canvas.fill_polygon(
                &[
                    (cx, cy - 6.0),
                    (cx + 5.5, cy - 3.0),
                    (cx + 5.5, cy + 3.0),
                    (cx, cy + 6.0),
                    (cx - 5.5, cy + 3.0),
                    (cx - 5.5, cy - 3.0),
                ],
                rgba(140, 220, 170, 70),
            );
            canvas.polyline(
                &[(cx - 5.5, cy - 3.0), (cx, cy), (cx + 5.5, cy - 3.0)],
                c,
                w,
            );
            canvas.line(cx, cy, cx, cy + 6.0, c, w);
        }
    }
}

pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split(' ') {
            if !line.is_empty() && line.chars().count() + word.chars().count() + 1 > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::code::{Key, Mods};
    use crate::doc::Document;
    use crate::ui::Pointer;

    #[test]
    fn edits_and_saves_a_script_through_the_ui() {
        let dir = std::env::temp_dir().join(format!("oberon-code-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("player.py");
        std::fs::write(&path, "def on_update(self, ctx):\n    pass\n").unwrap();

        let mut ui = Ui::new();
        let doc = Document::starter();
        let mut canvas = Canvas::new(1280, 800);
        let before = ui.layout(1280.0, 800.0).viewport;
        ui.open_code(&path, Some(2));
        let lay = ui.layout(1280.0, 800.0);
        assert!((lay.viewport.y - before.y - DOC_TABS_H).abs() < 0.01);
        assert!(!ui.scene_visible());
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);

        // Click at the end of line 2 to focus and place the caret.
        let v = lay.viewport;
        let click = Pointer {
            x: v.x + v.w - 40.0,
            y: v.y + LINE_H * 1.5,
            pressed: true,
            down: true,
            ..Pointer::default()
        };
        let _ = ui.frame(&mut canvas, &doc, click, None);
        assert!(ui.code_has_focus());
        assert_eq!(ui.code.current().unwrap().buf.cursor, Pos::new(1, 8));
        ui.code_key(Key::Backspace, Mods::default());
        ui.code_text("x = 1");
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert!(ui.code_key(Key::Char('s'), ctrl));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "def on_update(self, ctx):\n    pasx = 1\n");
        let reqs = ui.take_code_requests();
        assert!(reqs.contains(&CodeEvent::Saved(path.clone())), "{reqs:?}");

        // Unhandled shortcuts fall through to the app (Ctrl+P = play).
        assert!(!ui.code_key(Key::Char('p'), ctrl));
        // Closing the last tab shows the scene again.
        assert!(ui.code_key(Key::Char('w'), ctrl));
        assert!(ui.scene_visible());
        assert!((ui.layout(1280.0, 800.0).viewport.y - before.y).abs() < 0.01);
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scripts(tag: &str, names: &[&str]) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
        let dir = std::env::temp_dir().join(format!("oberon-tabs-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = names
            .iter()
            .map(|n| {
                let p = dir.join(n);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, "x = 1\nlonger_line = 2\n").unwrap();
                p
            })
            .collect();
        (dir, paths)
    }

    #[test]
    fn tab_strip_middle_click_closes_and_right_click_offers_the_tab_menu() {
        let (dir, paths) = scripts("strip", &["a.py", "b.py"]);
        let mut ui = Ui::new();
        let doc = Document::starter();
        let mut canvas = Canvas::new(1280, 800);
        for p in &paths {
            ui.open_code(p, None);
        }
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let strip = ui.doc_tabs_rect(ui.layout(1280.0, 800.0).viewport).unwrap();
        let scene_w = canvas.measure_text(layout::FONT_SIZE, "Scene") + 24.0;
        let first = (strip.x + 4.0 + scene_w + 2.0 + 12.0, strip.y + 14.0);
        let right = Pointer {
            x: first.0,
            y: first.1,
            right_pressed: true,
            ..Pointer::default()
        };
        let _ = ui.frame(&mut canvas, &doc, right, None);
        let menu = ui.menu.as_ref().expect("tab context menu");
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Close Others"), "{labels:?}");
        assert!(labels.contains(&"Reveal in Files"));
        ui.menu = None;
        let middle = Pointer {
            x: first.0,
            y: first.1,
            middle_pressed: true,
            ..Pointer::default()
        };
        let _ = ui.frame(&mut canvas, &doc, middle, None);
        assert_eq!(ui.code.tabs.len(), 1);
        assert_eq!(ui.code.tabs[0].path, paths[1]);
        // Close Others from the menu command.
        ui.open_code(&paths[0], None);
        let a = ui
            .code
            .tabs
            .iter()
            .position(|t| t.path == paths[0])
            .unwrap();
        ui.tab_command(TabCommand::CloseOthers(a));
        assert_eq!(ui.code.tabs.len(), 1);
        assert_eq!(ui.code.current().unwrap().path, paths[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_scale_zooms_the_editor_and_maps_clicks() {
        let (dir, paths) = scripts("scale", &["s.py"]);
        let mut ui = Ui::new();
        let doc = Document::starter();
        let mut canvas = Canvas::new(1280, 800);
        ui.prefs.editor.scale = 2.0;
        ui.open_code(&paths[0], None);
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let v = ui.layout(1280.0, 800.0).viewport;
        let click = |y: f32| Pointer {
            x: v.x + v.w - 40.0,
            y,
            pressed: true,
            down: true,
            ..Pointer::default()
        };
        // At 2x, the middle of line 2 is twice as far down.
        let _ = ui.frame(&mut canvas, &doc, click(v.y + LINE_H * 2.0 * 1.5), None);
        assert_eq!(ui.code.current().unwrap().buf.cursor.line, 1);
        let _ = ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let _ = ui.frame(&mut canvas, &doc, click(v.y + LINE_H * 1.5), None);
        assert_eq!(ui.code.current().unwrap().buf.cursor.line, 0);
        assert!(
            (canvas.scale() - 1.0).abs() < 1e-6,
            "the canvas scale is restored"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_open_lists_open_files_first_then_project_scripts() {
        let (dir, paths) = scripts("quick", &["a.py", "b.py", "sub/c.py"]);
        let mut ui = Ui::new();
        ui.project_root.clone_from(&dir);
        ui.open_code(&paths[0], None);
        ui.open_code(&paths[1], None);
        ui.open_quick_open();
        let p = ui.palette.as_ref().unwrap();
        let m = p.matches();
        assert_eq!((m[0].label.as_str(), m[0].group), ("a.py", "Open"));
        let labels: Vec<&str> = m.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"c.py") && labels.contains(&"b.py"),
            "{labels:?}"
        );
        assert_eq!(labels.len(), 3, "no duplicates: {labels:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wraps_long_text() {
        let w = wrap("one two three four", 9);
        assert_eq!(w, vec!["one two", "three", "four"]);
        assert_eq!(wrap("a\nb", 80), vec!["a", "b"]);
    }
}
