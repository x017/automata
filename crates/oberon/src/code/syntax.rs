//! Python syntax highlighting: a small line tokenizer with the cross-line
//! state needed for triple-quoted strings.

/// A token's highlight class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    /// Identifiers and anything unclassified.
    Text,
    /// `def`, `if`, `return` …
    Keyword,
    /// `print`, `len`, `range` … and builtin exception types.
    Builtin,
    /// `True`, `False`, `None`.
    Constant,
    /// `self` / `cls`.
    SelfKw,
    /// String literals (including f-string text).
    Str,
    /// Numbers.
    Number,
    /// `# …`
    Comment,
    /// `@decorator`
    Decorator,
    /// The name after `def`.
    Function,
    /// The name after `class`.
    Class,
    /// Operators and punctuation.
    Op,
}

/// Tokenizer state carried from one line to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineState {
    /// Plain code.
    #[default]
    Code,
    /// Inside a `"""` string.
    TripleDouble,
    /// Inside a `'''` string.
    TripleSingle,
}

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "match", "case", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
    "yield",
];

const BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "format",
    "getattr",
    "hasattr",
    "hash",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "map",
    "max",
    "min",
    "next",
    "object",
    "open",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "RuntimeError",
    "AttributeError",
    "ZeroDivisionError",
    "StopIteration",
    "NotImplementedError",
    "staticmethod",
    "classmethod",
    "__name__",
];

/// Highlights one line, returning `(byte range, class)` spans covering the
/// whole line and the state for the next line.
#[must_use]
pub fn highlight(line: &str, state: LineState) -> (Vec<(std::ops::Range<usize>, Tok)>, LineState) {
    let b = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut state = state;

    // Continue a triple-quoted string from the previous line.
    if state != LineState::Code {
        let quote = if state == LineState::TripleDouble {
            "\"\"\""
        } else {
            "'''"
        };
        if let Some(end) = find_unescaped(line, 0, quote) {
            spans.push((0..end + 3, Tok::Str));
            i = end + 3;
            state = LineState::Code;
        } else {
            spans.push((0..line.len(), Tok::Str));
            return (spans, state);
        }
    }

    // Context for naming the identifier after `def` / `class`.
    let mut after: Option<Tok> = None;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'#' {
            spans.push((i..b.len(), Tok::Comment));
            break;
        }
        if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            spans.push((start..i, Tok::Text));
            continue;
        }
        // String prefixes: r, b, f, u, rb, br, fr, rf (any case).
        let mut j = i;
        while j < b.len()
            && j - i < 2
            && matches!(b[j].to_ascii_lowercase(), b'r' | b'b' | b'f' | b'u')
        {
            j += 1;
        }
        // (Tokens start after whitespace/punctuation, so `j > i` means the
        // identifier so far is exactly a string prefix.)
        if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
            let raw = line[i..j].to_ascii_lowercase().contains('r');
            let q = b[j];
            let triple = b.get(j + 1) == Some(&q) && b.get(j + 2) == Some(&q);
            if triple {
                let quote = if q == b'"' { "\"\"\"" } else { "'''" };
                if let Some(end) = find_unescaped(line, j + 3, quote) {
                    i = end + 3;
                } else {
                    spans.push((start..b.len(), Tok::Str));
                    state = if q == b'"' {
                        LineState::TripleDouble
                    } else {
                        LineState::TripleSingle
                    };
                    return (spans, state);
                }
            } else {
                let mut k = j + 1;
                while k < b.len() && b[k] != q {
                    if b[k] == b'\\' && !raw {
                        k += 1;
                    }
                    k += 1;
                }
                i = (k + 1).min(b.len());
            }
            spans.push((start..i, Tok::Str));
            after = None;
            continue;
        }
        if c == b'@' && line[..i].trim().is_empty() {
            i += 1;
            while i < b.len() && (is_ident_byte(b[i]) || b[i] == b'.') {
                i += 1;
            }
            spans.push((start..i, Tok::Decorator));
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                // Exponent sign: 1e-3.
                if (b[i] == b'e' || b[i] == b'E') && matches!(b.get(i + 1), Some(b'+' | b'-')) {
                    i += 1;
                }
                i += 1;
            }
            spans.push((start..i, Tok::Number));
            after = None;
            continue;
        }
        if is_ident_byte(c) {
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            let word = &line[start..i];
            let tok = if let Some(t) = after.take() {
                t
            } else if KEYWORDS.contains(&word) {
                match word {
                    "def" => after = Some(Tok::Function),
                    "class" => after = Some(Tok::Class),
                    _ => {}
                }
                Tok::Keyword
            } else if matches!(word, "True" | "False" | "None") {
                Tok::Constant
            } else if matches!(word, "self" | "cls") {
                Tok::SelfKw
            } else if BUILTINS.contains(&word) && !line[..start].trim_end().ends_with('.') {
                Tok::Builtin
            } else {
                Tok::Text
            };
            spans.push((start..i, tok));
            continue;
        }
        // Operators / punctuation; one char at a time (multibyte safe).
        let len = line[i..].chars().next().map_or(1, char::len_utf8);
        i += len;
        let tok = if c.is_ascii_punctuation() {
            Tok::Op
        } else {
            Tok::Text
        };
        spans.push((start..i, tok));
        after = None;
    }
    (spans, state)
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// The byte offset of `quote` at or after `from`, skipping backslash
/// escapes.
fn find_unescaped(line: &str, from: usize, quote: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = from;
    while i + quote.len() <= b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i..].starts_with(quote.as_bytes()) {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(line: &str) -> Vec<(String, Tok)> {
        highlight(line, LineState::Code)
            .0
            .into_iter()
            .map(|(r, t)| (line[r].to_string(), t))
            .filter(|(s, _)| !s.trim().is_empty())
            .collect()
    }

    #[test]
    fn classifies_python_tokens() {
        let got = classes("def update(self, ctx):  # tick");
        assert_eq!(got[0], ("def".into(), Tok::Keyword));
        assert_eq!(got[1], ("update".into(), Tok::Function));
        assert_eq!(got[3], ("self".into(), Tok::SelfKw));
        assert_eq!(got.last().unwrap(), &("# tick".into(), Tok::Comment));

        let got = classes("x = print(f\"hp={hp!r}\", 1.5e-3, None)");
        assert!(got.contains(&("print".into(), Tok::Builtin)));
        assert!(got.contains(&("f\"hp={hp!r}\"".into(), Tok::Str)));
        assert!(got.contains(&("1.5e-3".into(), Tok::Number)));
        assert!(got.contains(&("None".into(), Tok::Constant)));

        let got = classes("@dataclass");
        assert_eq!(got[0], ("@dataclass".into(), Tok::Decorator));
        let got = classes("class Player(Base):");
        assert_eq!(got[1], ("Player".into(), Tok::Class));
        // Attribute names that shadow builtins aren't builtins.
        let got = classes("ctx.input.pressed('w')");
        assert!(got.iter().all(|(_, t)| *t != Tok::Builtin), "{got:?}");
        assert!(got.contains(&("'w'".into(), Tok::Str)));
    }

    #[test]
    fn spans_cover_the_line_without_gaps() {
        for line in [
            "a = 'x' + \"y\"  # c",
            "é = λ(1)",
            "\"\"\"a — b\"\"\"",
            "'— \\— é'",
            "s = r'\\d+'",
            "",
            "   ",
        ] {
            let (spans, _) = highlight(line, LineState::Code);
            let mut at = 0;
            for (r, _) in &spans {
                assert_eq!(r.start, at, "{line:?}: {spans:?}");
                at = r.end;
            }
            assert_eq!(at, line.len(), "{line:?}");
        }
    }

    #[test]
    fn triple_quoted_strings_span_lines() {
        let (spans, state) = highlight("x = \"\"\"start", LineState::Code);
        assert_eq!(state, LineState::TripleDouble);
        assert_eq!(spans.last().unwrap().1, Tok::Str);
        let (spans, state) = highlight("middle", state);
        assert_eq!(state, LineState::TripleDouble);
        assert_eq!(spans, vec![(0..6, Tok::Str)]);
        let (spans, state) = highlight("end\"\"\" + 1", state);
        assert_eq!(state, LineState::Code);
        assert_eq!(spans[0], (0..6, Tok::Str));
        assert!(spans.iter().any(|(_, t)| *t == Tok::Number));
        let (_, state) = highlight("'''one line'''", LineState::Code);
        assert_eq!(state, LineState::Code);
    }
}
