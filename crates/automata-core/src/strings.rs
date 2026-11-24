//! String tables for localization: `lang/*.csv` files in a project.
//!
//! The first row names the columns: a key column, then one column per
//! language code (`key,en,es,ja`). Every other row is a key and its text in
//! each language:
//!
//! ```text
//! key,en,es,ja
//! @name,English,Español,日本語
//! @font,,,fonts/MPLUSRounded1c-Medium.ttf
//! menu.play,Play,Jugar,プレイ
//! hud.coins,Coins: {count},Monedas: {count},コイン: {count}
//! # Rows starting with # are comments.
//! ```
//!
//! - `@name` gives each language the name a language picker shows.
//! - `@font` gives a language a font for the characters the text's own
//!   font lacks (before the built-in font).
//! - An empty cell falls back to the first language's text.
//! - `\n` in a cell starts a new line.
//!
//! Scripts read the tables with `ctx.tr("menu.play")` (the Python side
//! parses them itself with the same rules); this module is what Oberon and
//! export validation use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The folder string tables live in, relative to the scene's folder.
pub const STRINGS_DIR: &str = "lang";

/// One key's texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Where it's defined: `lang/strings.csv:12`.
    pub source: String,
    /// Text per language, in [`StringTable::languages`] order (`None`:
    /// empty).
    pub texts: Vec<Option<String>>,
}

/// Every `lang/*.csv` file of a project, merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringTable {
    /// Language codes in column order (the first is the default).
    pub languages: Vec<String>,
    /// The `@name` row, per language (empty: none given).
    pub names: Vec<String>,
    /// The `@font` row, per language (empty: none given).
    pub fonts: Vec<String>,
    /// Texts by key.
    pub entries: BTreeMap<String, Entry>,
    /// The files read, relative to the scene folder (`lang/strings.csv`).
    pub files: Vec<String>,
    /// Mistakes found while reading (with file and line), for a warning.
    pub problems: Vec<String>,
}

/// Splits CSV text into rows of cells (RFC 4180: quoted cells may hold
/// commas, doubled quotes and line breaks), each with the line it starts
/// on (from 1). A leading byte-order mark is skipped.
#[must_use]
pub fn parse_csv(text: &str) -> Vec<(usize, Vec<String>)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut line = 1;
    let mut row_line = 1;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => {
                    if c == '\n' {
                        line += 1;
                    }
                    cell.push(c);
                }
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut cell)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut cell));
                rows.push((row_line, std::mem::take(&mut row)));
                line += 1;
                row_line = line;
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push((row_line, row));
    }
    rows
}

/// A language code made comparable: lowercase, `_` as `-` (`pt_BR` and
/// `pt-br` match).
#[must_use]
pub fn normalize_language(code: &str) -> String {
    code.trim().to_lowercase().replace('_', "-")
}

impl StringTable {
    /// Reads every `*.csv` file in `<base>/lang` (in name order). No folder
    /// gives an empty table.
    #[must_use]
    pub fn load(base: &Path) -> Self {
        let mut table = Self::default();
        let dir = base.join(STRINGS_DIR);
        let Ok(read) = std::fs::read_dir(&dir) else {
            return table;
        };
        let mut files: Vec<PathBuf> = read
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
            })
            .collect();
        files.sort();
        for path in files {
            let name = path.file_name().map_or_else(String::new, |n| {
                format!("{STRINGS_DIR}/{}", n.to_string_lossy())
            });
            match std::fs::read_to_string(&path) {
                Ok(text) => table.add_file(&name, &text),
                Err(e) => table.problems.push(format!("{name}: {e}")),
            }
        }
        table
    }

    /// Adds one CSV file's rows (`name` is how messages refer to it).
    pub fn add_file(&mut self, name: &str, text: &str) {
        self.files.push(name.to_string());
        let mut rows = parse_csv(text)
            .into_iter()
            .filter(|(_, cells)| !is_blank_or_comment(cells));
        let Some((_, header)) = rows.next() else {
            self.problems.push(format!("{name}: empty"));
            return;
        };
        // Column -> language index (None: no language heads the column).
        let mut columns: Vec<Option<usize>> = Vec::new();
        for (i, cell) in header.iter().enumerate().skip(1) {
            let code = cell.trim();
            if code.is_empty() {
                columns.push(None);
                continue;
            }
            let want = normalize_language(code);
            let index = if let Some(at) = self
                .languages
                .iter()
                .position(|l| normalize_language(l) == want)
            {
                if columns.contains(&Some(at)) {
                    self.problems.push(format!(
                        "{name}:1: column {} repeats language {code}; ignoring it",
                        i + 1
                    ));
                    columns.push(None);
                    continue;
                }
                at
            } else {
                self.languages.push(code.to_string());
                self.names.push(String::new());
                self.fonts.push(String::new());
                for entry in self.entries.values_mut() {
                    entry.texts.push(None);
                }
                self.languages.len() - 1
            };
            columns.push(Some(index));
        }
        if columns.iter().all(Option::is_none) {
            self.problems.push(format!(
                "{name}:1: the first row should name the languages: key,en,es,…"
            ));
            return;
        }
        for (line, cells) in rows {
            let key = cells[0].trim();
            let source = format!("{name}:{line}");
            if cells.len() > header.len() && cells[header.len()..].iter().any(|c| !c.is_empty()) {
                self.problems.push(format!(
                    "{source}: more cells than languages (a comma inside text needs \"quotes\")"
                ));
            }
            if key.is_empty() {
                self.problems
                    .push(format!("{source}: text without a key in the first column"));
                continue;
            }
            let mut texts = vec![None; self.languages.len()];
            for (col, cell) in cells.iter().skip(1).enumerate() {
                if let Some(Some(lang)) = columns.get(col) {
                    if !cell.is_empty() {
                        texts[*lang] = Some(cell.replace("\\n", "\n"));
                    }
                }
            }
            match key {
                "@name" | "@font" => {
                    let target = if key == "@name" {
                        &mut self.names
                    } else {
                        &mut self.fonts
                    };
                    for (i, text) in texts.into_iter().enumerate() {
                        if let Some(t) = text {
                            target[i] = t.trim().to_string();
                        }
                    }
                }
                _ if key.starts_with('@') => self.problems.push(format!(
                    "{source}: unknown row {key} (the special rows are @name and @font)"
                )),
                _ => {
                    if let Some(first) = self.entries.get(key) {
                        self.problems.push(format!(
                            "{source}: {key} is already in {}; keeping that one",
                            first.source
                        ));
                    } else {
                        self.entries
                            .insert(key.to_string(), Entry { source, texts });
                    }
                }
            }
        }
    }

    /// Whether there are no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The column of `language` (`pt-BR` also finds `pt`), or `None`.
    #[must_use]
    pub fn language_index(&self, language: &str) -> Option<usize> {
        let want = normalize_language(language);
        let codes: Vec<String> = self
            .languages
            .iter()
            .map(|l| normalize_language(l))
            .collect();
        if let Some(i) = codes.iter().position(|c| *c == want) {
            return Some(i);
        }
        let base = want.split('-').next().unwrap_or_default();
        codes
            .iter()
            .position(|c| c == base)
            .or_else(|| codes.iter().position(|c| c.split('-').next() == Some(base)))
    }

    /// `key`'s text in `language`: its own, else the first language's.
    #[must_use]
    pub fn get(&self, key: &str, language: &str) -> Option<&str> {
        let entry = self.entries.get(key)?;
        let own = self
            .language_index(language)
            .and_then(|i| entry.texts.get(i)?.as_deref());
        own.or_else(|| entry.texts.first()?.as_deref())
    }

    /// The name a language picker shows (`@name`), else the code.
    #[must_use]
    pub fn name<'a>(&'a self, language: &'a str) -> &'a str {
        self.language_index(language)
            .map(|i| self.names[i].as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or(language)
    }

    /// The language's font (`@font`), relative to the scene folder.
    #[must_use]
    pub fn font(&self, language: &str) -> Option<&str> {
        self.language_index(language)
            .map(|i| self.fonts[i].as_str())
            .filter(|f| !f.is_empty())
    }

    /// Keys missing a text in a language other than the first, by
    /// language: `(language, keys)`.
    #[must_use]
    pub fn untranslated(&self) -> Vec<(String, Vec<String>)> {
        (1..self.languages.len())
            .filter_map(|i| {
                let keys: Vec<String> = self
                    .entries
                    .iter()
                    .filter(|(_, e)| e.texts.get(i).is_none_or(Option::is_none))
                    .map(|(k, _)| k.clone())
                    .collect();
                (!keys.is_empty()).then(|| (self.languages[i].clone(), keys))
            })
            .collect()
    }
}

fn is_blank_or_comment(cells: &[String]) -> bool {
    cells.iter().all(|c| c.trim().is_empty())
        || cells
            .first()
            .is_some_and(|c| c.trim_start().starts_with('#'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\u{feff}key,en,es,ja\r\n\
        @name,English,Español,日本語\r\n\
        @font,,,fonts/JP.ttf\r\n\
        # menus\r\n\
        menu.play,Play,Jugar,プレイ\r\n\
        \r\n\
        hud.coins,\"Coins: {count}, total\",\"Monedas: {count}\",\r\n\
        two.lines,\"First\nSecond\",Uno\\nDos,\r\n";

    #[test]
    fn csv_cells_follow_rfc_4180() {
        let rows = parse_csv("a,\"b,c\",\"say \"\"hi\"\"\"\n\"x\ny\",z\nlast");
        assert_eq!(
            rows,
            vec![
                (1, vec!["a".into(), "b,c".into(), "say \"hi\"".into()]),
                (2, vec!["x\ny".into(), "z".into()]),
                (4, vec!["last".into()]),
            ]
        );
    }

    #[test]
    fn a_table_reads_languages_names_fonts_and_texts() {
        let mut t = StringTable::default();
        t.add_file("lang/strings.csv", TABLE);
        assert_eq!(t.problems, Vec::<String>::new());
        assert_eq!(t.languages, ["en", "es", "ja"]);
        assert_eq!(t.name("ja"), "日本語");
        assert_eq!(t.name("fr"), "fr");
        assert_eq!(t.font("ja"), Some("fonts/JP.ttf"));
        assert_eq!(t.font("en"), None);
        assert_eq!(t.get("menu.play", "es"), Some("Jugar"));
        assert_eq!(t.get("hud.coins", "en"), Some("Coins: {count}, total"));
        // An empty cell falls back to the first language.
        assert_eq!(t.get("hud.coins", "ja"), Some("Coins: {count}, total"));
        assert_eq!(t.get("two.lines", "en"), Some("First\nSecond"));
        assert_eq!(t.get("two.lines", "es"), Some("Uno\nDos"));
        assert_eq!(t.get("nope", "en"), None);
        assert_eq!(t.entries["hud.coins"].source, "lang/strings.csv:7");
        assert_eq!(
            t.untranslated(),
            vec![(
                "ja".to_string(),
                vec!["hud.coins".to_string(), "two.lines".to_string()]
            )]
        );
    }

    #[test]
    fn regional_codes_find_their_language() {
        let mut t = StringTable::default();
        t.add_file("a.csv", "key,en,pt-BR,zh_Hans\nk,A,B,C\n");
        assert_eq!(t.language_index("EN_us"), Some(0));
        assert_eq!(t.language_index("pt"), Some(1));
        assert_eq!(t.language_index("pt_br"), Some(1));
        assert_eq!(t.language_index("zh-hans"), Some(2));
        assert_eq!(t.language_index("de"), None);
        assert_eq!(t.get("k", "pt-PT"), Some("B"));
    }

    #[test]
    fn files_merge_and_mistakes_are_reported_with_lines() {
        let mut t = StringTable::default();
        t.add_file("lang/a.csv", "key,en\nmenu.play,Play\n@colour,red\n");
        t.add_file(
            "lang/b.csv",
            "id,ja,en\nmenu.play,プレイ,Play again\nmenu.quit,終了,Quit\n,orphan\nx,a,b,c\n",
        );
        assert_eq!(t.languages, ["en", "ja"]);
        assert_eq!(t.get("menu.quit", "ja"), Some("終了"));
        assert_eq!(t.get("menu.quit", "en"), Some("Quit"));
        // The first definition wins.
        assert_eq!(t.get("menu.play", "en"), Some("Play"));
        assert_eq!(t.get("menu.play", "ja"), Some("Play"));
        assert_eq!(
            t.problems,
            [
                "lang/a.csv:3: unknown row @colour (the special rows are @name and @font)",
                "lang/b.csv:2: menu.play is already in lang/a.csv:2; keeping that one",
                "lang/b.csv:4: text without a key in the first column",
                "lang/b.csv:5: more cells than languages (a comma inside text needs \"quotes\")",
            ]
        );
    }

    #[test]
    fn a_folder_of_tables_loads_in_name_order() {
        let dir = std::env::temp_dir().join(format!("automata-strings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lang")).unwrap();
        std::fs::write(dir.join("lang/b.csv"), "key,en\nb,B\n").unwrap();
        std::fs::write(dir.join("lang/a.CSV"), "key,en,de\na,A,Ä\n").unwrap();
        std::fs::write(dir.join("lang/notes.txt"), "key,fr\n").unwrap();
        let t = StringTable::load(&dir);
        assert_eq!(t.files, ["lang/a.CSV", "lang/b.csv"]);
        assert_eq!(t.languages, ["en", "de"]);
        assert_eq!(t.get("a", "de"), Some("Ä"));
        assert_eq!(t.get("b", "de"), Some("B"));
        assert!(StringTable::load(&dir.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
