//! The code editor view: document tabs over the center area, the text
//! area (gutter, highlighting, selection, diagnostics), the find / go-to
//! bars, the completion popup, and mouse handling.

use tiny_skia::Color;

use super::{ButtonStyle, CursorHint, LogLevel, Rect, Ui};
use crate::canvas::Canvas;
use crate::code::buffer::{col_at_visual, visual_col};
use crate::code::syntax::{self, Tok};
use crate::code::{CodeEvent, Focus, Pos, Severity};
use crate::theme::{color, layout};

/// Height of the document tab strip.
pub const DOC_TABS_H: f32 = 28.0;
/// Code font size.
const FONT: f32 = 13.0;
/// Line height.
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

/// Syntax colors (dark-white sci-fi: white text, cool accents).
fn tok_color(t: Tok) -> Color {
    match t {
        Tok::Text => color::text(),
        Tok::Keyword => rgb(150, 190, 255),
        Tok::Builtin => rgb(190, 165, 255),
        Tok::Constant => rgb(255, 165, 125),
        Tok::SelfKw => rgb(230, 135, 160),
        Tok::Str => rgb(150, 215, 170),
        Tok::Number => rgb(236, 196, 110),
        Tok::Comment => rgb(108, 116, 134),
        Tok::Decorator => rgb(236, 170, 110),
        Tok::Function => rgb(120, 220, 255),
        Tok::Class => rgb(130, 235, 200),
        Tok::Op => rgb(170, 178, 196),
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
}

impl TextGeom {
    fn pos_at(&self, buf: &crate::code::Buffer, x: f32, y: f32) -> Pos {
        let line_f = ((y - self.area.y) / LINE_H + self.scroll).floor().max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let line = (line_f as usize).min(buf.line_count() - 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let vcol = ((x - self.text_x + self.scroll_x) / self.cell)
            .round()
            .max(0.0) as usize;
        Pos::new(line, col_at_visual(buf.line(line), vcol))
    }

    fn y_of(&self, line: usize) -> f32 {
        self.area.y + (line as f32 - self.scroll) * LINE_H
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
        let handled = self.code.key(key, mods);
        self.drain_code_events();
        handled
    }

    /// Forwards typed text to the code editor.
    pub fn code_text(&mut self, text: &str) {
        self.code.hover = None;
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

    /// Draws the Scene / script tabs.
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
        let mut activate = None;
        let mut close = None;
        for i in 0..self.code.tabs.len() {
            let title = self.code.tabs[i].title();
            let dirty = self.code.tabs[i].buf.dirty();
            let errors = self.code.tabs[i]
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error);
            let w = canvas.measure_text(layout::FONT_SIZE, &title) + 44.0;
            if x + w > r.x + r.w {
                break;
            }
            let tab = Rect::new(x, r.y + 3.0, w, r.h - 3.0);
            let active = self.code.active == Some(i);
            let (clicked, closed) = self.tab_chip(canvas, tab, &title, active, dirty, errors);
            if closed {
                close = Some(i);
            } else if clicked {
                activate = Some(i);
            }
            x += w + 2.0;
        }
        if let Some(i) = close {
            self.code.close(i, false);
            self.drain_code_events();
        } else if let Some(i) = activate {
            self.code.active = Some(i);
            self.code.focused = true;
            self.code.completion = None;
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
        let bg = if active {
            color::viewport_bg()
        } else if hot {
            color::hover()
        } else {
            color::surface()
        };
        canvas.fill_rect(r.x, r.y, r.w, r.h, bg);
        if active {
            canvas.fill_rect(r.x, r.y, r.w, 2.0, color::accent());
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
        let area = Rect::new(rect.x, top, rect.w, (status.y - top).max(LINE_H));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            self.code.page_lines = ((area.h / LINE_H) as usize).saturating_sub(1).max(1);
        }
        let cell = canvas.cell_width(FONT);
        let line_count = self.code.tabs[idx].buf.line_count();
        let digits = line_count.to_string().len().max(3);
        let gutter_w = digits as f32 * cell + 26.0;
        let text_x = area.x + gutter_w + 8.0;
        let visible = area.h / LINE_H;

        // --- scrolling ---
        {
            let hot = self.hot(area);
            let tab = &mut self.code.tabs[idx];
            if hot && self.p.scroll != 0.0 {
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

        // Current line.
        if selection.is_none() {
            let y = geom.y_of(cursor.line);
            canvas.fill_rect(area.x, y, area.w, LINE_H, rgb(18, 20, 26));
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
                        marks.push((m, rgba(96, 156, 255, 45)));
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
                LINE_H - 2.0,
                2.0,
                *c,
            );
        }
        // Selection.
        if let Some((a, b)) = selection {
            let sel = if focused {
                color::select()
            } else {
                rgb(36, 44, 60)
            };
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
                canvas.fill_rect(x0, geom.y_of(line), (x1 - x0).max(0.0), LINE_H, sel);
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
                        LINE_H - 2.0,
                        rgba(150, 195, 255, 150),
                        1.0,
                    );
                }
            }
        }
        // Text.
        for (k, line_idx) in (first..last).enumerate() {
            let line = buf.line(line_idx);
            let y = geom.y_of(line_idx) + 2.0;
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
                canvas.text(x, y, FONT, &shown, tok_color(tok));
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
                let y = geom.y_of(line) + LINE_H - 2.5;
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
        // Caret.
        if focused && self.code.focus == Focus::Text && cursor.line >= first && cursor.line < last {
            let x = geom.x_of(buf, cursor);
            canvas.fill_rect(
                x - 0.5,
                geom.y_of(cursor.line) + 1.0,
                2.0,
                LINE_H - 2.0,
                color::accent_soft(),
            );
        }
        canvas.restore_clip(clip);

        // Gutter.
        canvas.fill_rect(area.x, area.y, gutter_w, area.h, rgb(13, 14, 18));
        canvas.line(
            area.x + gutter_w,
            area.y,
            area.x + gutter_w,
            area.y + area.h,
            rgb(30, 33, 41),
            1.0,
        );
        let clip = canvas.set_clip(Some((area.x, area.y, gutter_w, area.h)));
        for line_idx in first..last {
            let y = geom.y_of(line_idx);
            let num = (line_idx + 1).to_string();
            let c = if line_idx == cursor.line {
                color::text()
            } else {
                rgb(80, 86, 100)
            };
            let nx = area.x + gutter_w - 10.0 - num.len() as f32 * cell;
            canvas.text(nx, y + 2.0, FONT - 1.0, &num, c);
            if let Some(sev) = tab.line_severity(line_idx) {
                canvas.fill_circle(area.x + 8.0, y + LINE_H / 2.0, 3.0, severity_color(sev));
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
            canvas.fill_rounded(track.x + 2.0, ty, 4.0, th, 2.0, rgba(150, 156, 170, 70));
        }

        self.code_status(canvas, status, idx);
        self.completion_popup(canvas, area, geom, idx);
        self.code_tooltips(canvas, area, geom, idx);
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
                    self.code.events.push(CodeEvent::Hover(tab.path.clone(), a));
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
        if let Some(sel) = tab.buf.selected_text() {
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
        canvas.stroke_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            3.0,
            if focused {
                color::accent()
            } else {
                color::border()
            },
            1.0,
        );
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
        let below = geom.y_of(buf.cursor.line) + LINE_H;
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
            let badge = match item.kind {
                'f' | 'm' => rgb(120, 220, 255),
                'c' => rgb(130, 235, 200),
                'k' => rgb(150, 190, 255),
                'M' => rgb(236, 196, 110),
                _ => rgb(190, 165, 255),
            };
            canvas.fill_rounded(row.x + 5.0, row.y + 4.0, 14.0, 13.0, 3.0, rgba(0, 0, 0, 60));
            canvas.text(
                row.x + 8.5,
                row.y + 3.0,
                layout::SMALL_SIZE,
                &item.kind.to_string(),
                badge,
            );
            let lw = canvas.text(row.x + 26.0, row.y + 3.0, FONT, &item.label, color::text());
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
                after && before && (self.p.y - line_y) < LINE_H
            }) {
                tip = Some((
                    self.p.x,
                    line_y + LINE_H + 2.0,
                    d.message.clone(),
                    severity_color(d.severity),
                ));
            }
        }
        // Language-server hover text at a position.
        if tip.is_none() {
            if let Some((p, text)) = &self.code.hover {
                let y = geom.y_of(p.line) + LINE_H + 2.0;
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
            (y - h - LINE_H - 4.0).max(area.y)
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
fn wrap(text: &str, width: usize) -> Vec<String> {
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

    #[test]
    fn wraps_long_text() {
        let w = wrap("one two three four", 9);
        assert_eq!(w, vec!["one two", "three", "four"]);
        assert_eq!(wrap("a\nb", 80), vec!["a", "b"]);
    }
}
