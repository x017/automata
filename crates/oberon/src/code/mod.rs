//! Oberon's built-in code editor (model and keyboard handling).
//!
//! Script tabs live next to the Scene tab in the center area. This module
//! owns the editor state (open files, focus, find/replace, go-to-line,
//! completion and diagnostics) and every keyboard command; drawing and the
//! mouse live in `ui::code_view`.

pub mod buffer;
pub mod clipboard;
pub mod syntax;

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

pub use buffer::{Buffer, Pos};
use clipboard::Clipboard;
use syntax::LineState;

/// How serious a diagnostic is (LSP severities).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// A real problem.
    Error,
    /// Probably a problem.
    Warning,
    /// Information.
    Info,
    /// A hint (unused names …).
    Hint,
}

/// A problem reported for a range of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Range start.
    pub start: Pos,
    /// Range end.
    pub end: Pos,
    /// Severity.
    pub severity: Severity,
    /// The message.
    pub message: String,
}

/// One completion proposal.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionItem {
    /// Shown text.
    pub label: String,
    /// Type / signature shown dimmed.
    pub detail: String,
    /// Text inserted in place of the typed prefix.
    pub insert: String,
    /// A one-letter kind badge (`f` function, `v` variable, `c` class …).
    pub kind: char,
}

/// The completion popup.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// All proposals (as received).
    pub items: Vec<CompletionItem>,
    /// Index into [`Completion::visible`].
    pub selected: usize,
    /// Where the word being completed starts.
    pub anchor: Pos,
}

impl Completion {
    /// The proposals matching the typed prefix, best first.
    #[must_use]
    pub fn visible(&self, prefix: &str) -> Vec<&CompletionItem> {
        let p = prefix.to_lowercase();
        let mut v: Vec<&CompletionItem> = self
            .items
            .iter()
            .filter(|i| p.is_empty() || fuzzy(&i.label.to_lowercase(), &p))
            .collect();
        v.sort_by_key(|i| {
            let l = i.label.to_lowercase();
            (!l.starts_with(&p), l.starts_with('_'), l.len())
        });
        v
    }
}

/// Subsequence match (`ctxin` matches `ctx.input`).
fn fuzzy(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|n| it.any(|h| h == n))
}

/// An open file.
#[derive(Debug)]
pub struct CodeTab {
    /// The file.
    pub path: PathBuf,
    /// Its text.
    pub buf: Buffer,
    /// First visible line (fractional while smooth-scrolling).
    pub scroll: f32,
    /// Horizontal scroll in pixels.
    pub scroll_x: f32,
    /// Scroll the caret into view on the next draw.
    pub reveal: bool,
    /// Problems from the language server.
    pub diagnostics: Vec<Diagnostic>,
    /// The file's modification time when last loaded / saved.
    pub disk_mtime: Option<SystemTime>,
    states: Vec<LineState>,
    states_version: Option<u64>,
}

impl CodeTab {
    fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            buf: Buffer::from_text(&text),
            scroll: 0.0,
            scroll_x: 0.0,
            reveal: true,
            diagnostics: Vec::new(),
            disk_mtime: mtime(path),
            states: Vec::new(),
            states_version: None,
        })
    }

    /// The file name for the tab label.
    #[must_use]
    pub fn title(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// The tokenizer state at the start of every line (cached per version).
    pub fn line_states(&mut self) -> &[LineState] {
        if self.states_version != Some(self.buf.version()) {
            self.states.clear();
            let mut state = LineState::Code;
            for line in self.buf.lines() {
                self.states.push(state);
                state = syntax::highlight(line, state).1;
            }
            self.states_version = Some(self.buf.version());
        }
        &self.states
    }

    /// Writes the buffer to disk.
    ///
    /// # Errors
    ///
    /// The I/O error, as text.
    pub fn save(&mut self) -> Result<(), String> {
        std::fs::write(&self.path, self.buf.text())
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.buf.mark_saved();
        self.disk_mtime = mtime(&self.path);
        Ok(())
    }

    /// The most severe diagnostic touching `line`.
    #[must_use]
    pub fn line_severity(&self, line: usize) -> Option<Severity> {
        self.diagnostics
            .iter()
            .filter(|d| d.start.line <= line && line <= d.end.line)
            .map(|d| d.severity)
            .min()
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A key the editor handles (letters arrive as `Char` with Ctrl/Alt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Enter.
    Enter,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// F3 (find next).
    F3,
    /// F12 (go to definition).
    F12,
    /// A character key pressed with Ctrl or Alt (lowercase).
    Char(char),
}

/// Modifier keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Ctrl (or Cmd).
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt.
    pub alt: bool,
}

/// Which input of the editor has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// The text.
    #[default]
    Text,
    /// The find field.
    Find,
    /// The replace field.
    Replace,
    /// The go-to-line field.
    GoTo,
}

/// The find / replace bar.
#[derive(Debug, Clone, Default)]
pub struct FindBar {
    /// Search text.
    pub query: String,
    /// Replacement text.
    pub replace: String,
    /// Whether the replace row is shown.
    pub show_replace: bool,
    /// Match case.
    pub case: bool,
    /// Whole words.
    pub word: bool,
}

/// Something the rest of the editor should react to.
#[derive(Debug, Clone, PartialEq)]
pub enum CodeEvent {
    /// A file was saved (scripts hot-reload from it).
    Saved(PathBuf),
    /// A message for the Console.
    Info(String),
    /// An error for the Console.
    Error(String),
    /// Ask the language server for completions at the caret.
    Complete(PathBuf, Pos),
    /// Ask the language server where the symbol at the caret is defined.
    Definition(PathBuf, Pos),
    /// Ask the language server about the symbol at a position.
    Hover(PathBuf, Pos),
    /// A tab was closed.
    Closed(PathBuf),
}

/// The editor: open tabs and their shared UI state.
#[derive(Debug, Default)]
pub struct CodeEditor {
    /// Open files.
    pub tabs: Vec<CodeTab>,
    /// The visible tab (`None` shows the Scene).
    pub active: Option<usize>,
    /// Whether the editor has the keyboard.
    pub focused: bool,
    /// Which field inside the editor has it.
    pub focus: Focus,
    /// The find / replace bar, when open.
    pub find: Option<FindBar>,
    /// The go-to-line input, when open.
    pub goto: Option<String>,
    /// The completion popup, when open.
    pub completion: Option<Completion>,
    /// Hover text from the language server: `(position, text)`.
    pub hover: Option<(Pos, String)>,
    /// A tab waiting for "save changes?" before closing.
    pub confirm_close: Option<usize>,
    /// The clipboard.
    pub clipboard: Clipboard,
    /// Events for the app to handle (drained every frame).
    pub events: Vec<CodeEvent>,
    /// Lines per page (set by the view from its height).
    pub page_lines: usize,
    /// The running language server's name (status line), if any.
    pub server: Option<String>,
    last_disk_check: Option<Instant>,
}

impl CodeEditor {
    /// An editor with no tabs.
    #[must_use]
    pub fn new() -> Self {
        Self {
            page_lines: 30,
            ..Self::default()
        }
    }

    /// The visible tab.
    #[must_use]
    pub fn current(&self) -> Option<&CodeTab> {
        self.tabs.get(self.active?)
    }

    /// The visible tab, mutably.
    pub fn current_mut(&mut self) -> Option<&mut CodeTab> {
        let i = self.active?;
        self.tabs.get_mut(i)
    }

    /// Whether a code tab is shown and has the keyboard.
    #[must_use]
    pub fn has_focus(&self) -> bool {
        self.focused && self.current().is_some()
    }

    /// Number of files with unsaved changes.
    #[must_use]
    pub fn dirty_count(&self) -> usize {
        self.tabs.iter().filter(|t| t.buf.dirty()).count()
    }

    /// Opens `path` (or switches to it) and puts the caret on 1-based
    /// `line`.
    ///
    /// # Errors
    ///
    /// When the file can't be read.
    pub fn open(&mut self, path: &Path, line: Option<u32>) -> Result<(), String> {
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let idx = if let Some(i) = self.tabs.iter().position(|t| {
            std::fs::canonicalize(&t.path).unwrap_or_else(|_| t.path.clone()) == canon
        }) {
            i
        } else {
            self.tabs.push(CodeTab::load(path)?);
            self.tabs.len() - 1
        };
        self.active = Some(idx);
        self.focused = true;
        self.focus = Focus::Text;
        self.completion = None;
        if let Some(line) = line.filter(|l| *l > 0) {
            let tab = &mut self.tabs[idx];
            let target = tab.buf.clamp(Pos::new(line as usize - 1, 0));
            let col = buffer::indent_of(tab.buf.line(target.line)).chars().count();
            tab.buf.move_to(Pos::new(target.line, col), false);
            // Center the line.
            tab.scroll = (target.line as f32 - self.page_lines as f32 / 2.0).max(0.0);
        }
        self.tabs[idx].reveal = true;
        Ok(())
    }

    /// Shows the Scene tab.
    pub fn show_scene(&mut self) {
        self.active = None;
        self.focused = false;
        self.completion = None;
    }

    /// Closes tab `idx`; a dirty tab asks first unless `force`.
    pub fn close(&mut self, idx: usize, force: bool) {
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        if tab.buf.dirty() && !force {
            self.confirm_close = Some(idx);
            self.active = Some(idx);
            return;
        }
        let tab = self.tabs.remove(idx);
        self.events.push(CodeEvent::Closed(tab.path));
        self.confirm_close = None;
        self.completion = None;
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > idx => Some(a - 1),
            Some(a) if a == idx => Some(idx.min(self.tabs.len() - 1)),
            other => other,
        };
        if self.active.is_none() {
            self.focused = false;
        }
    }

    /// Saves tab `idx`.
    pub fn save(&mut self, idx: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(idx) else {
            return false;
        };
        match tab.save() {
            Ok(()) => {
                let path = tab.path.clone();
                self.events
                    .push(CodeEvent::Info(format!("Saved {}", path.display())));
                self.events.push(CodeEvent::Saved(path));
                true
            }
            Err(e) => {
                self.events
                    .push(CodeEvent::Error(format!("Could not save {e}")));
                false
            }
        }
    }

    /// Saves every dirty tab; returns how many were saved.
    pub fn save_all(&mut self) -> usize {
        let dirty: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| self.tabs[i].buf.dirty())
            .collect();
        dirty.into_iter().filter(|&i| self.save(i)).count()
    }

    /// Reloads clean tabs whose files changed on disk (at most every
    /// second).
    pub fn check_disk(&mut self) {
        let now = Instant::now();
        if self
            .last_disk_check
            .is_some_and(|t| now.duration_since(t).as_secs_f32() < 1.0)
        {
            return;
        }
        self.last_disk_check = Some(now);
        for tab in &mut self.tabs {
            let m = mtime(&tab.path);
            if m.is_some() && m != tab.disk_mtime {
                tab.disk_mtime = m;
                if !tab.buf.dirty() {
                    if let Ok(text) = std::fs::read_to_string(&tab.path) {
                        if text != tab.buf.text() {
                            tab.buf.set_text(&text);
                            tab.buf.mark_saved();
                            self.events.push(CodeEvent::Info(format!(
                                "Reloaded {} (changed on disk)",
                                tab.title()
                            )));
                        }
                    }
                }
            }
        }
    }

    /// The word prefix being completed (from the completion anchor to the
    /// caret).
    #[must_use]
    pub fn completion_prefix(&self) -> String {
        match (self.current(), &self.completion) {
            (Some(tab), Some(c))
                if c.anchor.line == tab.buf.cursor.line && c.anchor.col <= tab.buf.cursor.col =>
            {
                tab.buf.text_range(c.anchor, tab.buf.cursor)
            }
            _ => String::new(),
        }
    }

    /// Opens the completion popup with `items` for the word at the caret.
    pub fn show_completions(&mut self, items: Vec<CompletionItem>) {
        let Some(tab) = self.current() else {
            return;
        };
        if items.is_empty() {
            self.completion = None;
            return;
        }
        let (a, _) = tab.buf.word_range(tab.buf.cursor);
        let anchor = if a.col < tab.buf.cursor.col {
            a
        } else {
            tab.buf.cursor
        };
        self.completion = Some(Completion {
            items,
            selected: 0,
            anchor,
        });
    }

    /// Inserts the selected completion. Returns false if none is visible.
    pub fn accept_completion(&mut self) -> bool {
        let prefix = self.completion_prefix();
        let Some(c) = self.completion.take() else {
            return false;
        };
        let Some(item) = c.visible(&prefix).get(c.selected).map(|i| i.insert.clone()) else {
            return false;
        };
        if let Some(tab) = self.current_mut() {
            let cursor = tab.buf.cursor;
            tab.buf.replace_range(c.anchor, cursor, &item);
            tab.reveal = true;
        }
        true
    }

    /// Completions without a language server: identifiers in the open
    /// files (current file first), then keywords and builtins.
    #[must_use]
    pub fn local_completions(&self) -> Vec<CompletionItem> {
        let Some(tab) = self.current() else {
            return Vec::new();
        };
        let (a, b) = tab.buf.word_range(tab.buf.cursor);
        let typing = if b == tab.buf.cursor {
            tab.buf.text_range(a, b)
        } else {
            String::new()
        };
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let others = self.tabs.iter().filter(|t| !std::ptr::eq(*t, tab));
        for (file, t) in std::iter::once(tab).chain(others).enumerate() {
            for line in t.buf.lines() {
                for word in line.split(|c: char| !buffer::is_word(c)) {
                    let starts_ok = word.chars().next().is_some_and(|c| !c.is_ascii_digit());
                    if word.chars().count() >= 3 && starts_ok && word != typing && seen.insert(word)
                    {
                        let detail = if file == 0 { "" } else { "other file" };
                        out.push(CompletionItem {
                            label: word.to_string(),
                            detail: detail.to_string(),
                            insert: word.to_string(),
                            kind: 'v',
                        });
                    }
                }
            }
        }
        for (words, kind) in [(syntax::KEYWORDS, 'k'), (syntax::BUILTINS, 'f')] {
            for w in words {
                if seen.insert(w) {
                    out.push(CompletionItem {
                        label: (*w).to_string(),
                        detail: String::new(),
                        insert: (*w).to_string(),
                        kind,
                    });
                }
            }
        }
        out
    }

    fn request(&mut self, make: fn(PathBuf, Pos) -> CodeEvent) {
        if let Some(tab) = self.current() {
            let ev = make(tab.path.clone(), tab.buf.cursor);
            self.events.push(ev);
        }
    }

    /// Typed text (no control characters).
    pub fn text(&mut self, s: &str) {
        match self.focus {
            Focus::Find => {
                if let Some(f) = &mut self.find {
                    f.query.push_str(s);
                }
                self.find_step(false, true);
            }
            Focus::Replace => {
                if let Some(f) = &mut self.find {
                    f.replace.push_str(s);
                }
            }
            Focus::GoTo => {
                if let Some(g) = &mut self.goto {
                    g.extend(s.chars().filter(char::is_ascii_digit));
                }
            }
            Focus::Text => {
                let Some(tab) = self.current_mut() else {
                    return;
                };
                tab.buf.type_text(s);
                tab.reveal = true;
                let word_char = s.chars().last().is_some_and(buffer::is_word);
                let after_dot = s.ends_with('.');
                if self.completion.is_some() {
                    if !word_char {
                        self.completion = None;
                    } else if let Some(c) = &mut self.completion {
                        c.selected = 0;
                    }
                }
                if after_dot
                    || (word_char && self.completion.is_none() && self.word_len_at_caret() == 2)
                {
                    self.request(CodeEvent::Complete);
                }
            }
        }
    }

    fn word_len_at_caret(&self) -> usize {
        self.current().map_or(0, |t| {
            let (a, b) = t.buf.word_range(t.buf.cursor);
            if b == t.buf.cursor {
                b.col - a.col
            } else {
                0
            }
        })
    }

    /// Selects the next / previous match of the find query (`incremental`
    /// re-searches from the current match start while typing).
    pub fn find_step(&mut self, back: bool, incremental: bool) {
        let Some(f) = self.find.clone() else {
            return;
        };
        let Some(tab) = self.current_mut() else {
            return;
        };
        if incremental {
            if let Some((a, _)) = tab.buf.selection() {
                tab.buf.move_to(a, false);
            }
        }
        tab.buf.find_next(&f.query, f.case, f.word, back);
        tab.reveal = true;
    }

    fn open_find(&mut self, replace: bool) {
        let seed = self
            .current()
            .and_then(|t| t.buf.selected_text())
            .filter(|s| !s.contains('\n'));
        let bar = self.find.get_or_insert_with(FindBar::default);
        if let Some(seed) = seed {
            bar.query = seed;
        }
        bar.show_replace |= replace;
        self.focus = if replace && !bar.query.is_empty() {
            Focus::Replace
        } else {
            Focus::Find
        };
    }

    /// Handles a key. Returns false when the editor doesn't use it (the app
    /// may treat it as a global shortcut).
    #[allow(clippy::too_many_lines)]
    pub fn key(&mut self, key: Key, m: Mods) -> bool {
        if self.current().is_none() {
            return false;
        }
        // Tab switching and closing work from any field.
        match (key, m.ctrl, m.shift) {
            (Key::Tab | Key::PageDown, true, false) | (Key::PageDown, true, true) => {
                self.cycle_tab(1);
                return true;
            }
            (Key::Tab, true, true) | (Key::PageUp, true, _) => {
                self.cycle_tab(-1);
                return true;
            }
            (Key::Char('w'), true, _) => {
                if let Some(i) = self.active {
                    self.close(i, false);
                }
                return true;
            }
            (Key::Char('s'), true, false) => {
                if let Some(i) = self.active {
                    self.save(i);
                }
                return true;
            }
            (Key::Char('s'), true, true) => {
                let n = self.save_all();
                self.events
                    .push(CodeEvent::Info(format!("Saved {n} file(s)")));
                return true;
            }
            _ => {}
        }
        match self.focus {
            Focus::Find | Focus::Replace => return self.find_key(key, m),
            Focus::GoTo => return self.goto_key(key),
            Focus::Text => {}
        }

        // Completion popup navigation.
        if self.completion.is_some() {
            let prefix = self.completion_prefix();
            let n = self
                .completion
                .as_ref()
                .map_or(0, |c| c.visible(&prefix).len());
            match key {
                Key::Up | Key::Down if n > 0 => {
                    if let Some(c) = &mut self.completion {
                        c.selected = if key == Key::Up {
                            (c.selected + n - 1) % n
                        } else {
                            (c.selected + 1) % n
                        };
                    }
                    return true;
                }
                Key::Enter | Key::Tab if n > 0 && !m.shift => {
                    self.accept_completion();
                    return true;
                }
                Key::Escape => {
                    self.completion = None;
                    return true;
                }
                Key::Left
                | Key::Right
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
                | Key::Enter => {
                    self.completion = None;
                }
                _ => {}
            }
        }

        let page = self.page_lines.max(1);
        let page_i = isize::try_from(page).unwrap_or(30);
        let Some(i) = self.active else { return false };
        let mut handled = true;
        let mut keep_completion = false;
        {
            let clipboard = &mut self.clipboard;
            let tab = &mut self.tabs[i];
            let b = &mut tab.buf;
            let sel = m.shift;
            match (key, m.ctrl, m.alt) {
                (Key::Left, false, false) => b.left(sel),
                (Key::Right, false, false) => b.right(sel),
                (Key::Left, true, false) => b.word(false, sel),
                (Key::Right, true, false) => b.word(true, sel),
                (Key::Up, false, false) => b.vertical(-1, sel),
                (Key::Down, false, false) => b.vertical(1, sel),
                (Key::Up, true, false) => tab.scroll = (tab.scroll - 1.0).max(0.0),
                (Key::Down, true, false) => tab.scroll += 1.0,
                (Key::Up, false, true) => b.move_lines(false),
                (Key::Down, false, true) => b.move_lines(true),
                (Key::Home, false, _) => b.home(sel),
                (Key::End, false, _) => b.end(sel),
                (Key::Home, true, _) => b.doc_edge(false, sel),
                (Key::End, true, _) => b.doc_edge(true, sel),
                (Key::PageUp, false, _) => {
                    b.vertical(-page_i, sel);
                    tab.scroll = (tab.scroll - page as f32).max(0.0);
                }
                (Key::PageDown, false, _) => {
                    b.vertical(page_i, sel);
                    tab.scroll += page as f32;
                }
                (Key::Backspace, ctrl, _) => {
                    if ctrl {
                        b.delete_word(false);
                    } else {
                        b.backspace();
                    }
                    keep_completion = true;
                }
                (Key::Delete, ctrl, _) => {
                    if ctrl {
                        b.delete_word(true);
                    } else {
                        b.delete();
                    }
                }
                (Key::Enter, false, _) => b.newline(),
                (Key::Enter, true, _) => {
                    // Ctrl+Enter: new line below without splitting.
                    b.end(false);
                    b.newline();
                }
                (Key::Tab, false, _) => {
                    if m.shift {
                        b.untab();
                    } else {
                        b.tab();
                    }
                }
                (Key::Escape, _, _) => {
                    if b.selection().is_some() {
                        b.anchor = None;
                    } else {
                        handled = false;
                    }
                }
                (Key::Char('a'), true, _) => b.select_all(),
                (Key::Char('c'), true, _) => {
                    let text = b
                        .selected_text()
                        .unwrap_or_else(|| format!("{}\n", b.line(b.cursor.line)));
                    clipboard.set(&text);
                }
                (Key::Char('x'), true, _) => {
                    if let Some(text) = b.selected_text() {
                        clipboard.set(&text);
                        b.insert("");
                    } else {
                        clipboard.set(&format!("{}\n", b.line(b.cursor.line)));
                        b.delete_lines();
                    }
                }
                (Key::Char('v'), true, _) => {
                    let text = clipboard.get();
                    if !text.is_empty() {
                        b.insert(&text);
                    }
                }
                (Key::Char('z'), true, _) if m.shift => {
                    b.redo();
                }
                (Key::Char('z'), true, _) => {
                    b.undo();
                }
                (Key::Char('y'), true, _) => {
                    b.redo();
                }
                (Key::Char('/'), true, _) => b.toggle_comment(),
                (Key::Char('d'), true, _) => b.duplicate_lines(),
                (Key::Char('k'), true, _) if m.shift => b.delete_lines(),
                (Key::Char('l'), true, _) => {
                    let (first, last) = b.touched_lines();
                    if b.selection().is_some() {
                        b.select(Pos::new(first, 0), Pos::new(last + 1, 0));
                    } else {
                        b.select_line(first);
                    }
                }
                (Key::Char(']'), true, _) => {
                    let (first, last) = b.touched_lines();
                    let had = b.selection();
                    b.select(
                        Pos::new(first, 0),
                        Pos::new(last, b.line(last).chars().count()),
                    );
                    b.tab();
                    if had.is_none() {
                        b.anchor = None;
                    }
                }
                (Key::Char('['), true, _) => b.untab(),
                _ => handled = false,
            }
            if handled {
                tab.reveal = !matches!((key, m.ctrl), (Key::Up | Key::Down, true));
            }
        }
        if handled {
            if !keep_completion {
                self.completion = None;
            }
            return true;
        }
        // Commands that open bars or talk to the language server.
        match (key, m.ctrl) {
            (Key::Char('f'), true) => self.open_find(false),
            (Key::Char('h'), true) => self.open_find(true),
            (Key::Char('g'), true) => {
                self.goto = Some(String::new());
                self.focus = Focus::GoTo;
            }
            (Key::F3, _) => self.find_step(m.shift, false),
            (Key::Char(' '), true) => self.request(CodeEvent::Complete),
            (Key::F12, _) => self.request(CodeEvent::Definition),
            (Key::Char('i'), true) if m.shift => self.request(CodeEvent::Hover),
            (Key::Escape, _) => {
                if self.find.is_some() {
                    self.find = None;
                } else if self.hover.is_some() {
                    self.hover = None;
                } else {
                    // Escape with nothing to dismiss hands the keyboard back.
                    self.focused = false;
                }
            }
            _ => return false,
        }
        true
    }

    fn cycle_tab(&mut self, dir: isize) {
        let n = self.tabs.len();
        if n == 0 {
            return;
        }
        let cur = self.active.unwrap_or(0);
        let next = (cur.cast_signed_compat() + dir).rem_euclid(n.cast_signed_compat());
        self.active = usize::try_from(next).ok();
        self.completion = None;
        self.focused = true;
    }

    fn find_key(&mut self, key: Key, m: Mods) -> bool {
        let Some(f) = &mut self.find else {
            self.focus = Focus::Text;
            return true;
        };
        let field = if self.focus == Focus::Replace {
            &mut f.replace
        } else {
            &mut f.query
        };
        match key {
            Key::Backspace => {
                field.pop();
                if self.focus == Focus::Find {
                    self.find_step(false, true);
                }
            }
            Key::Escape => {
                self.find = None;
                self.focus = Focus::Text;
            }
            Key::Tab => {
                if f.show_replace {
                    self.focus = if self.focus == Focus::Find {
                        Focus::Replace
                    } else {
                        Focus::Find
                    };
                }
            }
            Key::Enter | Key::F3 if self.focus == Focus::Find => self.find_step(m.shift, false),
            Key::Enter if m.ctrl && m.alt => {
                self.replace_all();
            }
            Key::Enter => self.replace_one(),
            Key::Char('a') if m.ctrl => field.clear(),
            Key::Char('v') if m.ctrl => {
                let text = self.clipboard.get();
                let line = text.lines().next().unwrap_or_default().to_string();
                if let Some(f) = &mut self.find {
                    if self.focus == Focus::Replace {
                        f.replace.push_str(&line);
                    } else {
                        f.query.push_str(&line);
                    }
                }
            }
            Key::Char('f') if m.ctrl => self.focus = Focus::Find,
            Key::Char('h') if m.ctrl => {
                f.show_replace = true;
                self.focus = Focus::Replace;
            }
            Key::Down | Key::Up => self.find_step(key == Key::Up, false),
            _ => return m.ctrl, // swallow plain keys; let Ctrl shortcuts through
        }
        true
    }

    /// Replaces the selected match and moves to the next.
    pub fn replace_one(&mut self) {
        let Some(f) = self.find.clone() else { return };
        let Some(tab) = self.current_mut() else {
            return;
        };
        if let Some((a, b)) = tab.buf.selection() {
            let selected = tab.buf.text_range(a, b);
            let same = if f.case {
                selected == f.query
            } else {
                selected.to_lowercase() == f.query.to_lowercase()
            };
            if same {
                tab.buf.replace_range(a, b, &f.replace);
            }
        }
        tab.buf.find_next(&f.query, f.case, f.word, false);
        tab.reveal = true;
    }

    /// Replaces every match.
    pub fn replace_all(&mut self) {
        let Some(f) = self.find.clone() else { return };
        let Some(tab) = self.current_mut() else {
            return;
        };
        let n = tab.buf.replace_all(&f.query, &f.replace, f.case, f.word);
        self.events
            .push(CodeEvent::Info(format!("Replaced {n} occurrence(s)")));
    }

    fn goto_key(&mut self, key: Key) -> bool {
        match key {
            Key::Backspace => {
                if let Some(g) = &mut self.goto {
                    g.pop();
                }
            }
            Key::Escape => {
                self.goto = None;
                self.focus = Focus::Text;
            }
            Key::Enter => {
                let line = self.goto.take().and_then(|g| g.parse::<usize>().ok());
                self.focus = Focus::Text;
                if let (Some(line), Some(tab)) = (line, self.current_mut()) {
                    let p = tab.buf.clamp(Pos::new(line.saturating_sub(1), 0));
                    tab.buf.move_to(p, false);
                    tab.reveal = true;
                }
            }
            _ => {}
        }
        true
    }
}

/// `usize` → `isize` without `cast_signed` (MSRV 1.86).
trait CastSignedCompat {
    fn cast_signed_compat(self) -> isize;
}

impl CastSignedCompat for usize {
    fn cast_signed_compat(self) -> isize {
        isize::try_from(self).unwrap_or(isize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(text: &str) -> (CodeEditor, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("oberon_code_{}_{}", std::process::id(), text.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.py");
        std::fs::write(&path, text).unwrap();
        let mut ed = CodeEditor::new();
        ed.open(&path, None).unwrap();
        (ed, path)
    }

    const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
        alt: false,
    };

    #[test]
    fn opening_saving_and_closing_tabs() {
        let (mut ed, path) = editor_with("x = 1\n");
        assert!(ed.has_focus());
        assert_eq!(ed.current().unwrap().title(), "script.py");
        // Re-opening switches instead of duplicating, and jumps to a line.
        ed.open(&path, Some(2)).unwrap();
        assert_eq!(ed.tabs.len(), 1);
        assert_eq!(ed.current().unwrap().buf.cursor.line, 1);
        ed.text("y = 2");
        assert_eq!(ed.dirty_count(), 1);
        // Closing a dirty tab asks first.
        assert!(ed.key(Key::Char('w'), CTRL));
        assert_eq!(ed.confirm_close, Some(0));
        assert_eq!(ed.tabs.len(), 1);
        assert!(ed.key(Key::Char('s'), CTRL));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x = 1\ny = 2");
        assert!(ed.events.contains(&CodeEvent::Saved(path.clone())));
        ed.close(0, false);
        assert!(ed.tabs.is_empty());
        assert!(ed.active.is_none() && !ed.has_focus());
    }

    #[test]
    fn find_replace_and_goto() {
        let (mut ed, _) = editor_with("hp = 1\nhp += hp\n");
        assert!(ed.key(Key::Char('h'), CTRL));
        assert_eq!(ed.focus, Focus::Find);
        ed.text("hp");
        assert!(
            ed.current().unwrap().buf.selection().is_some(),
            "incremental search selects"
        );
        ed.key(Key::Tab, Mods::default());
        assert_eq!(ed.focus, Focus::Replace);
        ed.text("health");
        ed.key(
            Key::Enter,
            Mods {
                ctrl: true,
                alt: true,
                shift: false,
            },
        );
        assert_eq!(
            ed.current().unwrap().buf.text(),
            "health = 1\nhealth += health\n"
        );
        ed.key(Key::Escape, Mods::default());
        assert!(ed.find.is_none());
        assert_eq!(ed.focus, Focus::Text);

        ed.key(Key::Char('g'), CTRL);
        ed.text("2");
        ed.key(Key::Enter, Mods::default());
        assert_eq!(ed.current().unwrap().buf.cursor, Pos::new(1, 0));
    }

    #[test]
    fn completions_filter_and_insert() {
        let (mut ed, path) = editor_with("");
        ed.text("ctx.");
        assert_eq!(
            ed.events.last(),
            Some(&CodeEvent::Complete(path, Pos::new(0, 4)))
        );
        ed.show_completions(vec![
            CompletionItem {
                label: "input".into(),
                detail: String::new(),
                insert: "input".into(),
                kind: 'v',
            },
            CompletionItem {
                label: "time".into(),
                detail: String::new(),
                insert: "time".into(),
                kind: 'v',
            },
        ]);
        ed.text("ti");
        assert_eq!(ed.completion_prefix(), "ti");
        let c = ed.completion.as_ref().unwrap();
        assert_eq!(c.visible("ti")[0].label, "time");
        assert!(ed.key(Key::Enter, Mods::default()));
        assert_eq!(ed.current().unwrap().buf.text(), "ctx.time");
        assert!(ed.completion.is_none());
    }

    #[test]
    fn clipboard_cut_copy_paste_lines() {
        let (mut ed, _) = editor_with("a\nb\n");
        ed.key(Key::Char('x'), CTRL); // cut the whole line
        assert_eq!(ed.current().unwrap().buf.text(), "b\n");
        ed.key(Key::Char('v'), CTRL);
        let text = ed.current().unwrap().buf.text();
        // With a system clipboard the paste may differ; the in-process one
        // round-trips exactly.
        if std::env::var_os("DISPLAY").is_none() {
            assert_eq!(text, "a\nb\n");
        }
    }

    #[test]
    fn reloads_clean_files_changed_on_disk() {
        let (mut ed, path) = editor_with("old\n");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "new\n").unwrap();
        // Force a different mtime even on coarse filesystems.
        ed.tabs[0].disk_mtime = None;
        ed.check_disk();
        assert_eq!(ed.current().unwrap().buf.text(), "new\n");
        assert!(!ed.current().unwrap().buf.dirty());
    }

    #[test]
    fn fuzzy_matching() {
        assert!(fuzzy("character_velocity", "chvel"));
        assert!(!fuzzy("time", "tx"));
    }
}
