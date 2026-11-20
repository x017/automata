//! The code editor's text buffer: lines, a caret with selection, Python-
//! aware editing commands, undo/redo, and search.
//!
//! Text is a `Vec<String>` of lines (no line terminators). Positions are
//! `(line, col)` with `col` counted in `char`s. Behaviour scripts are small,
//! so whole-buffer undo snapshots and linear search are simple and fast
//! enough.

use std::time::{Duration, Instant};

/// Spaces per indent level (PEP 8).
pub const INDENT: usize = 4;

/// Undo steps kept per buffer.
const UNDO_LIMIT: usize = 300;

/// A position in the buffer: line and `char` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    /// 0-based line.
    pub line: usize,
    /// 0-based column, in chars.
    pub col: usize,
}

impl Pos {
    /// A position.
    #[must_use]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Pos,
    anchor: Option<Pos>,
}

/// Groups consecutive edits of one kind into a single undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// A text buffer with a caret.
#[derive(Debug, Clone)]
pub struct Buffer {
    lines: Vec<String>,
    /// The caret.
    pub cursor: Pos,
    /// The selection's fixed end, when selecting.
    pub anchor: Option<Pos>,
    /// Visual column kept while moving up/down through short lines.
    want_col: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(EditKind, Instant, Pos)>,
    version: u64,
    saved_version: u64,
    /// Whether the file used `\r\n` line endings (kept on save).
    pub crlf: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::from_text("")
    }
}

fn char_to_byte(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(b, _)| b)
}

fn signed(n: usize) -> isize {
    isize::try_from(n).unwrap_or(isize::MAX)
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Whether `c` belongs to an identifier-like word.
#[must_use]
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The visual column of char column `col` (tabs advance to the next stop).
#[must_use]
pub fn visual_col(line: &str, col: usize) -> usize {
    let mut v = 0;
    for c in line.chars().take(col) {
        v = if c == '\t' {
            (v / INDENT + 1) * INDENT
        } else {
            v + 1
        };
    }
    v
}

/// The char column nearest visual column `vcol`.
#[must_use]
pub fn col_at_visual(line: &str, vcol: usize) -> usize {
    let mut v = 0;
    for (i, c) in line.chars().enumerate() {
        let next = if c == '\t' {
            (v / INDENT + 1) * INDENT
        } else {
            v + 1
        };
        if vcol < next {
            // Round to the nearer edge of the cell.
            return if vcol - v > (next - v) / 2 { i + 1 } else { i };
        }
        v = next;
    }
    char_len(line)
}

/// Leading whitespace of a line.
#[must_use]
pub fn indent_of(line: &str) -> &str {
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..n]
}

impl Buffer {
    /// A buffer holding `text` (`\r\n` is normalized and remembered).
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let crlf = text.contains("\r\n");
        let lines: Vec<String> = text
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect();
        Self {
            lines,
            cursor: Pos::default(),
            anchor: None,
            want_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            version: 0,
            saved_version: 0,
            crlf,
        }
    }

    /// The whole text (with the original line endings).
    #[must_use]
    pub fn text(&self) -> String {
        self.lines.join(if self.crlf { "\r\n" } else { "\n" })
    }

    /// Number of lines (at least 1).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Line `i` (empty past the end).
    #[must_use]
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    /// All lines.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Increments on every change (caches and the language server key on
    /// it).
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Whether there are unsaved changes.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.version != self.saved_version
    }

    /// Marks the current text as saved.
    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    /// Replaces the whole text (e.g. after the file changed on disk),
    /// keeping the caret where possible. Undoable.
    pub fn set_text(&mut self, text: &str) {
        self.checkpoint(EditKind::Other);
        let fresh = Self::from_text(text);
        self.lines = fresh.lines;
        self.crlf = fresh.crlf;
        self.anchor = None;
        self.cursor = self.clamp(self.cursor);
        self.changed();
    }

    /// Clamps a position into the buffer.
    #[must_use]
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        Pos::new(line, p.col.min(char_len(&self.lines[line])))
    }

    fn changed(&mut self) {
        self.version += 1;
        self.want_col = None;
    }

    // ----- undo ----------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    /// Records an undo step unless this edit continues the previous one.
    fn checkpoint(&mut self, kind: EditKind) {
        let now = Instant::now();
        let continues = kind != EditKind::Other
            && self.anchor.is_none()
            && self.last_edit.is_some_and(|(k, t, at)| {
                k == kind
                    && now.duration_since(t) < Duration::from_millis(1200)
                    && at == self.cursor
            });
        if !continues {
            self.undo.push(self.snapshot());
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some((kind, now, self.cursor));
    }

    /// Notes where the caret ended up after an edit (for coalescing).
    fn edited(&mut self) {
        if let Some((k, t, _)) = self.last_edit {
            self.last_edit = Some((k, t, self.cursor));
        }
    }

    /// Undoes the last edit. Returns false if there was none.
    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snap);
        true
    }

    /// Redoes the last undone edit. Returns false if there was none.
    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snap);
        true
    }

    fn restore(&mut self, snap: Snapshot) {
        self.lines = snap.lines;
        self.cursor = self.clamp(snap.cursor);
        self.anchor = snap.anchor.map(|a| self.clamp(a));
        self.last_edit = None;
        self.changed();
    }

    // ----- selection -----------------------------------------------------

    /// The selection as an ordered, non-empty range.
    #[must_use]
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        match a.cmp(&self.cursor) {
            std::cmp::Ordering::Less => Some((a, self.cursor)),
            std::cmp::Ordering::Greater => Some((self.cursor, a)),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// The text between two positions.
    #[must_use]
    pub fn text_range(&self, a: Pos, b: Pos) -> String {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        if a.line == b.line {
            let l = &self.lines[a.line];
            return l[char_to_byte(l, a.col)..char_to_byte(l, b.col)].to_string();
        }
        let first = &self.lines[a.line];
        let mut out = first[char_to_byte(first, a.col)..].to_string();
        for l in &self.lines[a.line + 1..b.line] {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        let last = &self.lines[b.line];
        out.push_str(&last[..char_to_byte(last, b.col)]);
        out
    }

    /// The selected text, if any.
    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.text_range(a, b))
    }

    /// Moves the caret, extending the selection when `select`.
    pub fn move_to(&mut self, p: Pos, select: bool) {
        let p = self.clamp(p);
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = p;
        self.last_edit = None;
    }

    /// Selects `a..b` with the caret at `b`.
    pub fn select(&mut self, a: Pos, b: Pos) {
        self.anchor = Some(self.clamp(a));
        self.cursor = self.clamp(b);
        self.want_col = None;
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        let last = self.lines.len() - 1;
        self.select(Pos::new(0, 0), Pos::new(last, char_len(&self.lines[last])));
    }

    /// The word range around `p` (empty range when not on a word).
    #[must_use]
    pub fn word_range(&self, p: Pos) -> (Pos, Pos) {
        let p = self.clamp(p);
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        let mut a = p.col;
        let mut b = p.col;
        let on_word = chars.get(p.col).copied().is_some_and(is_word)
            || (p.col > 0 && chars.get(p.col - 1).copied().is_some_and(is_word));
        if !on_word {
            return (p, p);
        }
        while a > 0 && is_word(chars[a - 1]) {
            a -= 1;
        }
        while b < chars.len() && is_word(chars[b]) {
            b += 1;
        }
        (Pos::new(p.line, a), Pos::new(p.line, b))
    }

    /// Selects the word at `p`.
    pub fn select_word(&mut self, p: Pos) {
        let (a, b) = self.word_range(p);
        self.select(a, b);
    }

    /// Selects line `line` including its line break.
    pub fn select_line(&mut self, line: usize) {
        let line = line.min(self.lines.len() - 1);
        if line + 1 < self.lines.len() {
            self.select(Pos::new(line, 0), Pos::new(line + 1, 0));
        } else {
            self.select(
                Pos::new(line, 0),
                Pos::new(line, char_len(&self.lines[line])),
            );
        }
    }

    // ----- motion --------------------------------------------------------

    /// One char left (or to the selection start).
    pub fn left(&mut self, select: bool) {
        if let (false, Some((a, _))) = (select, self.selection()) {
            self.move_to(a, false);
            return;
        }
        let c = self.cursor;
        let p = if c.col > 0 {
            Pos::new(c.line, c.col - 1)
        } else if c.line > 0 {
            Pos::new(c.line - 1, char_len(&self.lines[c.line - 1]))
        } else {
            c
        };
        self.move_to(p, select);
        self.want_col = None;
    }

    /// One char right (or to the selection end).
    pub fn right(&mut self, select: bool) {
        if let (false, Some((_, b))) = (select, self.selection()) {
            self.move_to(b, false);
            return;
        }
        let c = self.cursor;
        let p = if c.col < char_len(&self.lines[c.line]) {
            Pos::new(c.line, c.col + 1)
        } else if c.line + 1 < self.lines.len() {
            Pos::new(c.line + 1, 0)
        } else {
            c
        };
        self.move_to(p, select);
        self.want_col = None;
    }

    /// `n` lines up (negative) or down (positive), keeping the visual
    /// column.
    pub fn vertical(&mut self, n: isize, select: bool) {
        let c = self.cursor;
        let want = self
            .want_col
            .unwrap_or_else(|| visual_col(&self.lines[c.line], c.col));
        let last = self.lines.len() - 1;
        let target = c.line.saturating_add_signed(n).min(last);
        let p = if n < 0 && c.line == 0 {
            Pos::new(0, 0)
        } else if n > 0 && c.line == last {
            Pos::new(last, char_len(&self.lines[last]))
        } else {
            Pos::new(target, col_at_visual(&self.lines[target], want))
        };
        self.move_to(p, select);
        self.want_col = Some(want);
    }

    /// Smart Home: first non-blank char, then column 0.
    pub fn home(&mut self, select: bool) {
        let c = self.cursor;
        let first = char_len(indent_of(&self.lines[c.line]));
        let col = if c.col == first { 0 } else { first };
        self.move_to(Pos::new(c.line, col), select);
        self.want_col = None;
    }

    /// End of line.
    pub fn end(&mut self, select: bool) {
        let c = self.cursor;
        self.move_to(Pos::new(c.line, char_len(&self.lines[c.line])), select);
        self.want_col = None;
    }

    /// Start / end of the buffer.
    pub fn doc_edge(&mut self, end: bool, select: bool) {
        let p = if end {
            let last = self.lines.len() - 1;
            Pos::new(last, char_len(&self.lines[last]))
        } else {
            Pos::new(0, 0)
        };
        self.move_to(p, select);
        self.want_col = None;
    }

    /// The position one word left/right of `p`.
    #[must_use]
    pub fn word_step(&self, p: Pos, forward: bool) -> Pos {
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        if forward {
            if p.col >= chars.len() {
                return if p.line + 1 < self.lines.len() {
                    Pos::new(p.line + 1, 0)
                } else {
                    p
                };
            }
            let mut i = p.col;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && is_word(chars[i]) {
                while i < chars.len() && is_word(chars[i]) {
                    i += 1;
                }
            } else if i < chars.len() {
                while i < chars.len() && !is_word(chars[i]) && !chars[i].is_whitespace() {
                    i += 1;
                }
            }
            Pos::new(p.line, i)
        } else {
            if p.col == 0 {
                return if p.line > 0 {
                    Pos::new(p.line - 1, char_len(&self.lines[p.line - 1]))
                } else {
                    p
                };
            }
            let mut i = p.col;
            while i > 0 && chars[i - 1].is_whitespace() {
                i -= 1;
            }
            if i > 0 && is_word(chars[i - 1]) {
                while i > 0 && is_word(chars[i - 1]) {
                    i -= 1;
                }
            } else {
                while i > 0 && !is_word(chars[i - 1]) && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
            }
            Pos::new(p.line, i)
        }
    }

    /// One word left/right.
    pub fn word(&mut self, forward: bool, select: bool) {
        let p = self.word_step(self.cursor, forward);
        self.move_to(p, select);
        self.want_col = None;
    }

    // ----- editing primitives -------------------------------------------

    /// Deletes `a..b` (no undo bookkeeping); returns `a`.
    fn remove(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let tail = {
            let l = &self.lines[b.line];
            l[char_to_byte(l, b.col)..].to_string()
        };
        let head = &mut self.lines[a.line];
        head.truncate(char_to_byte(head, a.col));
        head.push_str(&tail);
        self.lines.drain(a.line + 1..=b.line);
        a
    }

    /// Inserts `text` at `p` (no undo bookkeeping); returns the end.
    fn put(&mut self, p: Pos, text: &str) -> Pos {
        let p = self.clamp(p);
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let line = &self.lines[p.line];
        let at = char_to_byte(line, p.col);
        let tail = line[at..].to_string();
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or_default();
        self.lines[p.line].truncate(at);
        self.lines[p.line].push_str(first);
        let mut end = Pos::new(p.line, p.col + char_len(first));
        let mut idx = p.line;
        for part in parts {
            idx += 1;
            self.lines.insert(idx, part.to_string());
            end = Pos::new(idx, char_len(part));
        }
        self.lines[idx].push_str(&tail);
        end
    }

    fn delete_selection_raw(&mut self) -> bool {
        if let Some((a, b)) = self.selection() {
            self.cursor = self.remove(a, b);
            self.anchor = None;
            true
        } else {
            self.anchor = None;
            false
        }
    }

    /// Replaces the selection (or inserts at the caret) with `text`, as one
    /// undo step. Used for paste and completions.
    pub fn insert(&mut self, text: &str) {
        self.checkpoint(EditKind::Other);
        self.delete_selection_raw();
        self.cursor = self.put(self.cursor, text);
        self.changed();
    }

    /// Replaces `a..b` with `text` (one undo step), caret at the end.
    pub fn replace_range(&mut self, a: Pos, b: Pos, text: &str) {
        self.checkpoint(EditKind::Other);
        self.anchor = None;
        let at = self.remove(a, b);
        self.cursor = self.put(at, text);
        self.changed();
    }

    fn char_at(&self, p: Pos) -> Option<char> {
        self.lines.get(p.line)?.chars().nth(p.col)
    }

    fn char_before(&self, p: Pos) -> Option<char> {
        if p.col == 0 {
            return None;
        }
        self.lines.get(p.line)?.chars().nth(p.col - 1)
    }

    /// Types one character: replaces the selection, auto-closes brackets,
    /// steps over a matching closer, and wraps a selection in brackets or
    /// quotes.
    pub fn type_char(&mut self, c: char) {
        let closer = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' | '\'' => Some(c),
            _ => None,
        };
        // Wrap the selection.
        if let (Some(close), Some((a, b))) = (closer, self.selection()) {
            if a.line == b.line {
                self.checkpoint(EditKind::Other);
                let inner = self.text_range(a, b);
                self.remove(a, b);
                let end = self.put(a, &format!("{c}{inner}{close}"));
                self.select(Pos::new(a.line, a.col + 1), Pos::new(end.line, end.col - 1));
                self.changed();
                return;
            }
        }
        let next = self.char_at(self.cursor);
        // Step over the closer that auto-closing inserted.
        if self.anchor.is_none() && matches!(c, ')' | ']' | '}' | '"' | '\'') && next == Some(c) {
            self.cursor.col += 1;
            self.last_edit = None;
            return;
        }
        self.checkpoint(if c.is_whitespace() {
            EditKind::Other
        } else {
            EditKind::Typing
        });
        self.delete_selection_raw();
        let prev = self.char_before(self.cursor);
        let next_ok = next.is_none_or(|n| n.is_whitespace() || ")]}:,".contains(n));
        let pair = match closer {
            Some(close) if c == '"' || c == '\'' => {
                // Quotes pair only between non-word characters.
                (next_ok && !prev.is_some_and(is_word) && prev != Some(c)).then_some(close)
            }
            Some(close) => next_ok.then_some(close),
            None => None,
        };
        let mut s = String::from(c);
        if let Some(close) = pair {
            s.push(close);
        }
        let start = self.cursor;
        self.put(start, &s);
        self.cursor = Pos::new(start.line, start.col + 1);
        self.changed();
        self.edited();
    }

    /// Types text from the keyboard (each char through [`Self::type_char`];
    /// pasted / IME text with newlines goes through [`Self::insert`]).
    pub fn type_text(&mut self, text: &str) {
        if text.contains('\n') {
            self.insert(text);
            return;
        }
        for c in text.chars().filter(|c| !c.is_control()) {
            self.type_char(c);
        }
    }

    /// Enter: newline with Python auto-indent (one level deeper after `:`
    /// or an open bracket, one shallower after `return`/`pass`/…).
    pub fn newline(&mut self) {
        self.checkpoint(EditKind::Other);
        self.delete_selection_raw();
        let c = self.cursor;
        let line = self.lines[c.line].clone();
        let before: String = line.chars().take(c.col).collect();
        let mut indent = indent_of(&line).to_string();
        if indent.len() > before.len() {
            indent.truncate(before.len());
        }
        let trimmed = before.trim_end();
        let code = trimmed.split(" #").next().unwrap_or(trimmed).trim_end();
        let opens = code.ends_with(':') || code.ends_with(['(', '[', '{']);
        let first_word = trimmed.split_whitespace().next().unwrap_or_default();
        if opens {
            indent.push_str(&" ".repeat(INDENT));
        } else if matches!(
            first_word,
            "return" | "pass" | "break" | "continue" | "raise"
        ) {
            let n = indent.len().saturating_sub(INDENT);
            indent.truncate(n);
        }
        // Between a bracket pair: put the closer on its own line.
        let next = self.char_at(c);
        let between =
            code.ends_with(['(', '[', '{']) && next.is_some_and(|n| matches!(n, ')' | ']' | '}'));
        let at = self.put(c, &format!("\n{indent}"));
        if between {
            let outer = indent_of(&line).to_string();
            self.put(at, &format!("\n{outer}"));
        }
        self.cursor = at;
        // Trailing whitespace left on the split line is noise.
        let prev = &mut self.lines[c.line];
        let kept = prev.trim_end().len();
        if kept < prev.len() && c.col > 0 {
            prev.truncate(kept);
        }
        self.changed();
    }

    /// Backspace: deletes the selection, a bracket pair, back to the
    /// previous indent stop in leading whitespace, or one char.
    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.checkpoint(EditKind::Other);
            self.delete_selection_raw();
            self.changed();
            return;
        }
        let c = self.cursor;
        if c == Pos::default() {
            return;
        }
        self.checkpoint(EditKind::Deleting);
        self.anchor = None;
        if c.col == 0 {
            let col = char_len(&self.lines[c.line - 1]);
            self.cursor = self.remove(Pos::new(c.line - 1, col), c);
        } else {
            let line = &self.lines[c.line];
            let before: String = line.chars().take(c.col).collect();
            let prev = self.char_before(c);
            let next = self.char_at(c);
            let pair = matches!(
                (prev, next),
                (Some('('), Some(')'))
                    | (Some('['), Some(']'))
                    | (Some('{'), Some('}'))
                    | (Some('"'), Some('"'))
                    | (Some('\''), Some('\''))
            );
            let start = if !before.is_empty() && before.chars().all(|ch| ch == ' ') {
                let stop = (c.col - 1) / INDENT * INDENT;
                Pos::new(c.line, stop)
            } else {
                Pos::new(c.line, c.col - 1)
            };
            let end = if pair { Pos::new(c.line, c.col + 1) } else { c };
            self.cursor = self.remove(start, end);
        }
        self.changed();
        self.edited();
    }

    /// Delete: the selection or the char after the caret.
    pub fn delete(&mut self) {
        if self.selection().is_some() {
            self.checkpoint(EditKind::Other);
            self.delete_selection_raw();
            self.changed();
            return;
        }
        let c = self.cursor;
        let len = char_len(&self.lines[c.line]);
        if c.col == len && c.line + 1 == self.lines.len() {
            return;
        }
        self.checkpoint(EditKind::Deleting);
        self.anchor = None;
        let end = if c.col < len {
            Pos::new(c.line, c.col + 1)
        } else {
            Pos::new(c.line + 1, 0)
        };
        self.remove(c, end);
        self.changed();
        self.edited();
    }

    /// Deletes the word before / after the caret (Ctrl+Backspace/Delete).
    pub fn delete_word(&mut self, forward: bool) {
        if self.selection().is_some() {
            self.delete();
            return;
        }
        let other = self.word_step(self.cursor, forward);
        if other == self.cursor {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.cursor = self.remove(self.cursor, other);
        self.changed();
    }

    /// The lines the selection (or the caret) touches.
    #[must_use]
    pub fn touched_lines(&self) -> (usize, usize) {
        match self.selection() {
            Some((a, b)) => {
                // A selection ending at column 0 doesn't include that line.
                let last = if b.col == 0 && b.line > a.line {
                    b.line - 1
                } else {
                    b.line
                };
                (a.line, last)
            }
            None => (self.cursor.line, self.cursor.line),
        }
    }

    fn shift_cols(&mut self, line: usize, from_col: usize, delta: isize) {
        let adjust = |p: &mut Pos| {
            if p.line == line && p.col >= from_col {
                p.col = p.col.saturating_add_signed(delta);
            }
        };
        adjust(&mut self.cursor);
        if let Some(a) = &mut self.anchor {
            adjust(a);
        }
    }

    /// Tab: indents the selected lines, or inserts spaces to the next stop.
    pub fn tab(&mut self) {
        let (first, last) = self.touched_lines();
        if self.selection().is_some() && last > first
            || self.selection().is_some_and(|(a, b)| a.line != b.line)
        {
            self.indent_lines(first, last, true);
            return;
        }
        self.checkpoint(EditKind::Typing);
        self.delete_selection_raw();
        let v = visual_col(&self.lines[self.cursor.line], self.cursor.col);
        let n = INDENT - v % INDENT;
        let start = self.cursor;
        self.cursor = self.put(start, &" ".repeat(n));
        self.changed();
        self.edited();
    }

    /// Shift+Tab: dedents the selected lines (or the caret's line).
    pub fn untab(&mut self) {
        let (first, last) = self.touched_lines();
        self.indent_lines(first, last, false);
    }

    fn indent_lines(&mut self, first: usize, last: usize, indent: bool) {
        self.checkpoint(EditKind::Other);
        for i in first..=last.min(self.lines.len() - 1) {
            if indent {
                if self.lines[i].trim().is_empty() {
                    continue;
                }
                self.lines[i].insert_str(0, &" ".repeat(INDENT));
                self.shift_cols(i, 0, signed(INDENT));
            } else {
                let lead = self.lines[i]
                    .chars()
                    .take(INDENT)
                    .take_while(|c| *c == ' ')
                    .count();
                let lead = if lead == 0 && self.lines[i].starts_with('\t') {
                    1
                } else {
                    lead
                };
                if lead > 0 {
                    self.lines[i].drain(..lead);
                    self.shift_cols(i, 0, -signed(lead));
                }
            }
        }
        self.cursor = self.clamp(self.cursor);
        self.anchor = self.anchor.map(|a| self.clamp(a));
        self.changed();
    }

    /// Ctrl+/: comments out the selected lines with `# `, or uncomments
    /// them when all are already comments.
    pub fn toggle_comment(&mut self) {
        let (first, last) = self.touched_lines();
        let code: Vec<usize> = (first..=last)
            .filter(|&i| !self.lines[i].trim().is_empty())
            .collect();
        if code.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let all_commented = code
            .iter()
            .all(|&i| self.lines[i].trim_start().starts_with('#'));
        if all_commented {
            for &i in &code {
                let ind = char_len(indent_of(&self.lines[i]));
                let byte = char_to_byte(&self.lines[i], ind);
                let n = if self.lines[i][byte..].starts_with("# ") {
                    2
                } else {
                    1
                };
                self.lines[i].replace_range(byte..byte + n, "");
                self.shift_cols(i, ind, -signed(n));
            }
        } else {
            let min_ind = code
                .iter()
                .map(|&i| char_len(indent_of(&self.lines[i])))
                .min()
                .unwrap_or(0);
            for &i in &code {
                let byte = char_to_byte(&self.lines[i], min_ind);
                self.lines[i].insert_str(byte, "# ");
                self.shift_cols(i, min_ind, 2);
            }
        }
        self.cursor = self.clamp(self.cursor);
        self.changed();
    }

    /// Duplicates the selected lines (or the caret's line) below.
    pub fn duplicate_lines(&mut self) {
        let (first, last) = self.touched_lines();
        self.checkpoint(EditKind::Other);
        let block: Vec<String> = self.lines[first..=last].to_vec();
        let n = block.len();
        for (k, l) in block.into_iter().enumerate() {
            self.lines.insert(last + 1 + k, l);
        }
        self.cursor.line += n;
        if let Some(a) = &mut self.anchor {
            a.line += n;
        }
        self.changed();
    }

    /// Moves the selected lines (or the caret's line) up or down.
    pub fn move_lines(&mut self, down: bool) {
        let (first, last) = self.touched_lines();
        if (down && last + 1 >= self.lines.len()) || (!down && first == 0) {
            return;
        }
        self.checkpoint(EditKind::Other);
        if down {
            let l = self.lines.remove(last + 1);
            self.lines.insert(first, l);
        } else {
            let l = self.lines.remove(first - 1);
            self.lines.insert(last, l);
        }
        let shift = |p: &mut Pos| {
            p.line = if down { p.line + 1 } else { p.line - 1 };
        };
        shift(&mut self.cursor);
        if let Some(a) = &mut self.anchor {
            shift(a);
        }
        self.changed();
    }

    /// Deletes the selected lines (or the caret's line).
    pub fn delete_lines(&mut self) {
        let (first, last) = self.touched_lines();
        self.checkpoint(EditKind::Other);
        self.lines.drain(first..=last);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.anchor = None;
        self.cursor = self.clamp(Pos::new(first, self.cursor.col));
        self.changed();
    }

    // ----- search --------------------------------------------------------

    /// All matches of `query` (char ranges), in order.
    #[must_use]
    pub fn find_all(&self, query: &str, case_sensitive: bool, whole_word: bool) -> Vec<(Pos, Pos)> {
        if query.is_empty() || query.contains('\n') {
            return Vec::new();
        }
        let q = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        let qlen = char_len(query);
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            let hay = if case_sensitive {
                line.clone()
            } else {
                line.to_lowercase()
            };
            // Lowercasing can change byte lengths for some scripts; fall back
            // to the original when it does.
            let hay = if hay.len() == line.len() {
                hay
            } else {
                line.clone()
            };
            let chars: Vec<char> = line.chars().collect();
            let mut from = 0;
            while let Some(off) = hay[from..].find(&q) {
                let byte = from + off;
                let col = line[..byte].chars().count();
                let ok = !whole_word
                    || ((col == 0 || !is_word(chars[col - 1]))
                        && chars.get(col + qlen).is_none_or(|c| !is_word(*c)));
                if ok {
                    out.push((Pos::new(i, col), Pos::new(i, col + qlen)));
                }
                from = byte + q.len().max(1);
                if from > hay.len() {
                    break;
                }
            }
        }
        out
    }

    /// Selects the next (or previous) match after the caret, wrapping.
    /// Returns false when there is none.
    pub fn find_next(
        &mut self,
        query: &str,
        case_sensitive: bool,
        whole_word: bool,
        back: bool,
    ) -> bool {
        let all = self.find_all(query, case_sensitive, whole_word);
        if all.is_empty() {
            return false;
        }
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let pick = if back {
            all.iter().rev().find(|(a, _)| *a < start).or(all.last())
        } else {
            all.iter().find(|(a, _)| *a >= end).or(all.first())
        };
        if let Some(&(a, b)) = pick {
            self.select(a, b);
        }
        true
    }

    /// Replaces every match; returns how many.
    pub fn replace_all(
        &mut self,
        query: &str,
        with: &str,
        case_sensitive: bool,
        whole_word: bool,
    ) -> usize {
        let all = self.find_all(query, case_sensitive, whole_word);
        if all.is_empty() {
            return 0;
        }
        self.checkpoint(EditKind::Other);
        for &(a, b) in all.iter().rev() {
            self.remove(a, b);
            self.put(a, with);
        }
        self.anchor = None;
        self.cursor = self.clamp(self.cursor);
        self.changed();
        all.len()
    }

    /// The bracket matching the one at (or just before) the caret.
    #[must_use]
    pub fn matching_bracket(&self) -> Option<(Pos, Pos)> {
        let c = self.cursor;
        let candidates = [Some(c), (c.col > 0).then(|| Pos::new(c.line, c.col - 1))];
        for p in candidates.into_iter().flatten() {
            let Some(ch) = self.char_at(p) else { continue };
            let (open, close, forward) = match ch {
                '(' => ('(', ')', true),
                '[' => ('[', ']', true),
                '{' => ('{', '}', true),
                ')' => ('(', ')', false),
                ']' => ('[', ']', false),
                '}' => ('{', '}', false),
                _ => continue,
            };
            let mut depth = 0i32;
            let mut q = p;
            // Bounded scan (scripts are small; avoid pathological files).
            for _ in 0..200_000 {
                let qc = self.char_at(q);
                if qc == Some(open) {
                    depth += if forward { 1 } else { -1 };
                } else if qc == Some(close) {
                    depth += if forward { -1 } else { 1 };
                }
                if depth == 0 {
                    return Some((p, q));
                }
                let next = if forward {
                    if q.col + 1 < char_len(&self.lines[q.line]) {
                        Pos::new(q.line, q.col + 1)
                    } else if q.line + 1 < self.lines.len() {
                        Pos::new(q.line + 1, 0)
                    } else {
                        break;
                    }
                } else if q.col > 0 {
                    Pos::new(q.line, q.col - 1)
                } else if q.line > 0 {
                    let l = q.line - 1;
                    Pos::new(l, char_len(&self.lines[l]).saturating_sub(1))
                } else {
                    break;
                };
                q = next;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::from_text(text)
    }

    #[test]
    fn text_round_trips_with_line_endings() {
        let b = buf("a\r\nb\r\n");
        assert!(b.crlf);
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.text(), "a\r\nb\r\n");
        assert_eq!(buf("x\ny").text(), "x\ny");
    }

    #[test]
    fn typing_inserts_and_undo_groups_words() {
        let mut b = buf("");
        b.type_text("hello");
        b.type_char(' ');
        b.type_text("world");
        assert_eq!(b.text(), "hello world");
        assert!(b.dirty());
        assert!(b.undo());
        assert_eq!(b.text(), "hello ");
        assert!(b.undo());
        assert!(b.undo());
        assert_eq!(b.text(), "");
        assert!(b.redo());
        assert_eq!(b.text(), "hello");
    }

    #[test]
    fn brackets_and_quotes_auto_close_and_step_over() {
        let mut b = buf("");
        b.type_text("print(");
        assert_eq!(b.text(), "print()");
        b.type_char('"');
        assert_eq!(b.text(), "print(\"\")");
        b.type_text("hi");
        b.type_char('"');
        b.type_char(')');
        assert_eq!(b.text(), "print(\"hi\")");
        assert_eq!(b.cursor, Pos::new(0, 11));
        // Backspace inside an empty pair deletes both.
        let mut b = buf("");
        b.type_char('[');
        b.backspace();
        assert_eq!(b.text(), "");
        // Apostrophes inside words don't pair.
        let mut b = buf("");
        b.type_text("don't");
        assert_eq!(b.text(), "don't");
    }

    #[test]
    fn enter_auto_indents_python_blocks() {
        let mut b = buf("def update(entity, ctx):");
        b.end(false);
        b.newline();
        assert_eq!(b.line(1), "    ");
        b.type_text("return");
        b.newline();
        assert_eq!(b.line(2), "", "dedent after return");
        let mut b = buf("x = [");
        b.end(false);
        b.type_char(']');
        b.left(false);
        b.newline();
        assert_eq!(b.text(), "x = [\n    \n]");
        assert_eq!(b.cursor, Pos::new(1, 4));
    }

    #[test]
    fn backspace_removes_soft_tabs() {
        let mut b = buf("        x");
        b.move_to(Pos::new(0, 8), false);
        b.backspace();
        assert_eq!(b.line(0), "    x");
        b.backspace();
        assert_eq!(b.line(0), "x");
        let mut b = buf("a\nb");
        b.move_to(Pos::new(1, 0), false);
        b.backspace();
        assert_eq!(b.text(), "ab");
    }

    #[test]
    fn tab_indents_and_untab_dedents_selections() {
        let mut b = buf("a\nb\nc");
        b.select(Pos::new(0, 0), Pos::new(1, 1));
        b.tab();
        assert_eq!(b.text(), "    a\n    b\nc");
        b.untab();
        assert_eq!(b.text(), "a\nb\nc");
        let mut b = buf("ab");
        b.move_to(Pos::new(0, 1), false);
        b.tab();
        assert_eq!(b.text(), "a   b");
    }

    #[test]
    fn comments_toggle_on_selected_lines() {
        let mut b = buf("    x = 1\n    y = 2\n");
        b.select(Pos::new(0, 0), Pos::new(1, 3));
        b.toggle_comment();
        assert_eq!(b.text(), "    # x = 1\n    # y = 2\n");
        b.toggle_comment();
        assert_eq!(b.text(), "    x = 1\n    y = 2\n");
    }

    #[test]
    fn line_operations() {
        let mut b = buf("one\ntwo\nthree");
        b.move_to(Pos::new(0, 1), false);
        b.duplicate_lines();
        assert_eq!(b.text(), "one\none\ntwo\nthree");
        assert_eq!(b.cursor.line, 1);
        b.move_lines(true);
        assert_eq!(b.text(), "one\ntwo\none\nthree");
        b.delete_lines();
        assert_eq!(b.text(), "one\ntwo\nthree");
    }

    #[test]
    fn motion_words_home_and_vertical_columns() {
        let mut b = buf("    foo.bar(x)\nab\nlong line here");
        b.move_to(Pos::new(0, 14), false);
        b.home(false);
        assert_eq!(b.cursor, Pos::new(0, 4));
        b.home(false);
        assert_eq!(b.cursor, Pos::new(0, 0));
        b.word(true, false);
        assert_eq!(b.cursor, Pos::new(0, 7), "skips spaces and the word");
        b.word(true, true);
        assert_eq!(b.selected_text().as_deref(), Some("."));
        b.move_to(Pos::new(0, 10), false);
        b.vertical(1, false);
        assert_eq!(b.cursor, Pos::new(1, 2));
        b.vertical(1, false);
        assert_eq!(b.cursor, Pos::new(2, 10), "the column is remembered");
    }

    #[test]
    fn selections_wrap_and_replace() {
        let mut b = buf("value");
        b.select_all();
        b.type_char('(');
        assert_eq!(b.text(), "(value)");
        assert_eq!(b.selected_text().as_deref(), Some("value"));
        b.insert("x\ny");
        assert_eq!(b.text(), "(x\ny)");
        assert_eq!(b.cursor, Pos::new(1, 1));
    }

    #[test]
    fn find_and_replace() {
        let mut b = buf("speed = 1\nSpeed = speed * 2\nspeedy");
        assert_eq!(b.find_all("speed", false, false).len(), 4);
        assert_eq!(b.find_all("speed", true, false).len(), 3);
        assert_eq!(b.find_all("speed", true, true).len(), 2);
        assert!(b.find_next("speed", true, true, false));
        assert_eq!(b.selection(), Some((Pos::new(0, 0), Pos::new(0, 5))));
        assert!(b.find_next("speed", true, true, false));
        assert_eq!(b.selection(), Some((Pos::new(1, 8), Pos::new(1, 13))));
        assert!(b.find_next("speed", true, true, false));
        assert_eq!(
            b.selection(),
            Some((Pos::new(0, 0), Pos::new(0, 5))),
            "wraps"
        );
        assert_eq!(b.replace_all("speed", "pace", true, true), 2);
        assert_eq!(b.text(), "pace = 1\nSpeed = pace * 2\nspeedy");
        assert!(b.undo());
        assert_eq!(b.line(0), "speed = 1");
    }

    #[test]
    fn brackets_match_across_lines() {
        let mut b = buf("f(a, [1,\n  2])");
        b.move_to(Pos::new(0, 1), false);
        assert_eq!(b.matching_bracket(), Some((Pos::new(0, 1), Pos::new(1, 4))));
        b.move_to(Pos::new(1, 3), false);
        assert_eq!(b.matching_bracket(), Some((Pos::new(1, 3), Pos::new(0, 5))));
        b.move_to(Pos::new(1, 5), false); // after the last ')'
        assert_eq!(b.matching_bracket(), Some((Pos::new(1, 4), Pos::new(0, 1))));
    }

    #[test]
    fn tabs_have_visual_columns() {
        assert_eq!(visual_col("\tx", 1), 4);
        assert_eq!(visual_col("ab\tx", 3), 4);
        assert_eq!(col_at_visual("\tx", 4), 1);
        assert_eq!(col_at_visual("abc", 10), 3);
    }
}
