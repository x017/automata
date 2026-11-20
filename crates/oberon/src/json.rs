//! A small JSON value type with a parser and serializer.
//!
//! Oberon speaks JSON with two out-of-process peers: the Python script
//! bridge (one JSON object per line) and language servers (JSON-RPC). The
//! messages are small, so a straightforward recursive-descent parser over
//! `&str` is plenty.

use std::fmt::Write as _;

/// A JSON value. Integers and floats stay distinct so attribute types
/// survive the round trip (`3` is an Int, `3.0` a Float).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Json {
    /// `null`.
    #[default]
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A number without fraction or exponent.
    Int(i64),
    /// Any other number.
    Float(f64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Json>),
    /// An object; key order is preserved.
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// An object from `(key, value)` pairs.
    #[must_use]
    pub fn obj<K: Into<String>>(pairs: impl IntoIterator<Item = (K, Json)>) -> Self {
        Self::Obj(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// A string value.
    #[must_use]
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }

    /// An array of floats.
    #[must_use]
    pub fn floats(values: &[f32]) -> Self {
        Self::Arr(values.iter().map(|v| Self::Float(f64::from(*v))).collect())
    }

    /// The member `key` of an object.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets (or adds) the member `key` of an object; no-op for non-objects.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Self::Obj(pairs) = self {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            } else {
                pairs.push((key.to_string(), value));
            }
        }
    }

    /// The string, if this is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The number as `f64`, if this is a number.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The number as `i64`, if this is an integer (or an integral float).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(*f as i64),
            _ => None,
        }
    }

    /// The bool, if this is one.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The elements, if this is an array.
    #[must_use]
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Self::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// The members, if this is an object.
    #[must_use]
    pub fn as_obj(&self) -> Option<&[(String, Json)]> {
        match self {
            Self::Obj(o) => Some(o),
            _ => None,
        }
    }

    /// `N` floats from an array of numbers.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn as_floats<const N: usize>(&self) -> Option<[f32; N]> {
        let arr = self.as_arr()?;
        if arr.len() != N {
            return None;
        }
        let mut out = [0.0; N];
        for (slot, v) in out.iter_mut().zip(arr) {
            *slot = v.as_f64()? as f32;
        }
        Some(out)
    }

    /// Parses one JSON document (surrounding whitespace allowed).
    ///
    /// # Errors
    ///
    /// Returns a message with the byte offset of the first problem.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut p = Parser {
            s: text.as_bytes(),
            text,
            i: 0,
        };
        let value = p.value(0)?;
        p.ws();
        if p.i != p.s.len() {
            return Err(format!("trailing characters at {}", p.i));
        }
        Ok(value)
    }

    /// Compact serialization (no newlines — safe for line protocols).
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Self::Float(f) => {
                if f.is_finite() {
                    // `{:?}` always keeps a `.0` or exponent, so floats stay
                    // floats on the other side.
                    let _ = write!(out, "{f:?}");
                } else {
                    out.push_str("null");
                }
            }
            Self::Str(s) => write_str(out, s),
            Self::Arr(items) => {
                out.push('[');
                for (n, item) in items.iter().enumerate() {
                    if n > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Self::Obj(pairs) => {
                out.push('{');
                for (n, (k, v)) in pairs.iter().enumerate() {
                    if n > 0 {
                        out.push(',');
                    }
                    write_str(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for Json {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<f64> for Json {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Self::Str(v.to_string())
    }
}

impl From<String> for Json {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Nesting limit (guards the recursion against hostile input).
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self
            .s
            .get(self.i)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at {}", self.i))
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return self.err("nesting too deep");
        }
        self.ws();
        match self.s.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'"') => self.string().map(Json::Str),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return self.err("expected , or ]"),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(pairs));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return self.err("expected a key");
                    }
                    let key = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("expected :");
                    }
                    self.i += 1;
                    pairs.push((key, self.value(depth + 1)?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(pairs));
                        }
                        _ => return self.err("expected , or }"),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.err("unexpected character"),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        let mut float = false;
        while let Some(&b) = self.s.get(self.i) {
            match b {
                b'0'..=b'9' | b'-' | b'+' => {}
                b'.' | b'e' | b'E' => float = true,
                _ => break,
            }
            self.i += 1;
        }
        let text = &self.text[start..self.i];
        if !float {
            if let Ok(i) = text.parse::<i64>() {
                return Ok(Json::Int(i));
            }
        }
        text.parse::<f64>()
            .map(Json::Float)
            .map_err(|_| format!("bad number at {start}"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.i..self.i + 4)
            .ok_or_else(|| format!("bad escape at {}", self.i))?;
        let v = u32::from_str_radix(digits, 16).map_err(|_| format!("bad escape at {}", self.i))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            // Copy the run of plain characters in one go.
            let run = self.i;
            while self
                .s
                .get(self.i)
                .is_some_and(|b| *b != b'"' && *b != b'\\')
            {
                self.i += 1;
            }
            out.push_str(&self.text[run..self.i]);
            match self.s.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(_) => {
                    self.i += 1;
                    let Some(&e) = self.s.get(self.i) else {
                        return self.err("unterminated string");
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return self.err("bad escape"),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_dumps_round_trip() {
        let text = r#"{"op":"frame","dt":0.016,"n":3,"ok":true,"none":null,"p":[1.0,-2.5,3e2],"s":"a\"b\\c\nd\u00e9\ud83d\ude00"}"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.get("op").and_then(Json::as_str), Some("frame"));
        assert_eq!(v.get("n"), Some(&Json::Int(3)));
        assert_eq!(v.get("dt"), Some(&Json::Float(0.016)));
        assert_eq!(
            v.get("p").and_then(Json::as_floats::<3>),
            Some([1.0, -2.5, 300.0])
        );
        assert_eq!(v.get("s").and_then(Json::as_str), Some("a\"b\\c\ndé😀"));
        let again = Json::parse(&v.dump()).unwrap();
        assert_eq!(again, v);
        assert!(!v.dump().contains('\n'));
    }

    #[test]
    fn ints_and_floats_stay_distinct() {
        assert_eq!(Json::Float(3.0).dump(), "3.0");
        assert_eq!(Json::Int(3).dump(), "3");
        assert_eq!(Json::parse("3").unwrap(), Json::Int(3));
        assert_eq!(Json::parse("3.0").unwrap(), Json::Float(3.0));
        assert_eq!(Json::Float(f64::NAN).dump(), "null");
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "{",
            "[1,",
            "{\"a\" 1}",
            "tru",
            "\"abc",
            "[1] x",
            "{1:2}",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(500) + &"]".repeat(500);
        assert!(Json::parse(&deep).is_err());
    }

    #[test]
    fn builders_and_setters() {
        let mut v = Json::obj([("a", Json::Int(1))]);
        v.set("a", Json::from("x"));
        v.set("b", Json::from(true));
        assert_eq!(v.dump(), r#"{"a":"x","b":true}"#);
    }
}
