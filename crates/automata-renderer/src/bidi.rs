//! Right-to-left text: Arabic-script joining (Persian, Arabic, Urdu) and
//! bidirectional reordering, enough to draw a line of Persian with numbers
//! and Latin words in it.
//!
//! Text is stored in logical order (the order it's typed and read).
//! [`visual_line`] turns one line into the characters to draw left to right:
//!
//! 1. **Joining.** Each Arabic-script letter becomes its isolated, initial,
//!    medial or final form, picked from its neighbours (Unicode's joining
//!    types), using the presentation-form characters (U+FB50–U+FDFF,
//!    U+FE70–U+FEFF) that most Arabic and Persian fonts map. Lam + alef
//!    become the لا ligature. ZWNJ (the Persian half-space) breaks a join
//!    and ZWJ / tatweel force one.
//! 2. **Ordering.** A simplified Unicode Bidirectional Algorithm: the line's
//!    direction is its first strong letter's; right-to-left runs are
//!    reversed, numbers and left-to-right words inside them keep their own
//!    order, neutrals between two runs of one direction take it, and
//!    brackets are mirrored in right-to-left runs.
//!
//! Not covered: mark positioning from a font's GPOS table (marks are drawn
//! where the font puts them by default), explicit embeddings (U+202A…), and
//! scripts that need full shaping (Indic, Thai line breaking).

/// A character's joining type (Unicode `ArabicShaping.txt`, simplified).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joining {
    /// Joins on both sides.
    Dual,
    /// Joins only to the letter before it (alef, dal, reh, waw…).
    Right,
    /// Forces a join on both sides (tatweel, ZWJ) without changing shape.
    Causing,
    /// Marks: skipped when looking for neighbours.
    Transparent,
    /// Doesn't join.
    None,
}

/// Letters with presentation forms: `(letter, isolated form, dual)`. A dual
/// letter's forms are isolated, final, initial, medial in a row; a
/// right-joining one has isolated and final.
const FORMS: &[(u32, u32, bool)] = &[
    (0x0622, 0xFE81, false),
    (0x0623, 0xFE83, false),
    (0x0624, 0xFE85, false),
    (0x0625, 0xFE87, false),
    (0x0626, 0xFE89, true),
    (0x0627, 0xFE8D, false),
    (0x0628, 0xFE8F, true),
    (0x0629, 0xFE93, false),
    (0x062A, 0xFE95, true),
    (0x062B, 0xFE99, true),
    (0x062C, 0xFE9D, true),
    (0x062D, 0xFEA1, true),
    (0x062E, 0xFEA5, true),
    (0x062F, 0xFEA9, false),
    (0x0630, 0xFEAB, false),
    (0x0631, 0xFEAD, false),
    (0x0632, 0xFEAF, false),
    (0x0633, 0xFEB1, true),
    (0x0634, 0xFEB5, true),
    (0x0635, 0xFEB9, true),
    (0x0636, 0xFEBD, true),
    (0x0637, 0xFEC1, true),
    (0x0638, 0xFEC5, true),
    (0x0639, 0xFEC9, true),
    (0x063A, 0xFECD, true),
    (0x0641, 0xFED1, true),
    (0x0642, 0xFED5, true),
    (0x0643, 0xFED9, true),
    (0x0644, 0xFEDD, true),
    (0x0645, 0xFEE1, true),
    (0x0646, 0xFEE5, true),
    (0x0647, 0xFEE9, true),
    (0x0648, 0xFEED, false),
    (0x0649, 0xFEEF, false),
    (0x064A, 0xFEF1, true),
    // Persian, Urdu and other extensions.
    (0x0671, 0xFB50, false),
    (0x0679, 0xFB66, true),
    (0x067E, 0xFB56, true),
    (0x0686, 0xFB7A, true),
    (0x0688, 0xFB88, false),
    (0x0691, 0xFB8C, false),
    (0x0698, 0xFB8A, false),
    (0x06A4, 0xFB6A, true),
    (0x06A9, 0xFB8E, true),
    (0x06AD, 0xFBD3, true),
    (0x06AF, 0xFB92, true),
    (0x06BA, 0xFB9E, false),
    (0x06BE, 0xFBAA, true),
    (0x06C0, 0xFBA4, false),
    (0x06C1, 0xFBA6, true),
    (0x06CC, 0xFBFC, true),
    (0x06D2, 0xFBAE, false),
];

/// Lam + this alef → the ligature's isolated form (its final form is +1).
const LAM_ALEF: &[(u32, u32)] = &[
    (0x0622, 0xFEF5),
    (0x0623, 0xFEF7),
    (0x0625, 0xFEF9),
    (0x0627, 0xFEFB),
];

const LAM: char = '\u{0644}';
const ZWJ: char = '\u{200D}';

fn forms(c: char) -> Option<(u32, bool)> {
    let u = u32::from(c);
    FORMS
        .binary_search_by_key(&u, |f| f.0)
        .ok()
        .map(|i| (FORMS[i].1, FORMS[i].2))
}

fn is_mark(c: char) -> bool {
    matches!(u32::from(c),
        0x0300..=0x036F
        | 0x0591..=0x05BD
        | 0x05BF
        | 0x05C1..=0x05C2
        | 0x05C4..=0x05C5
        | 0x05C7
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x08D3..=0x08FF
        | 0xFE20..=0xFE2F)
}

fn joining(c: char) -> Joining {
    if is_mark(c) {
        return Joining::Transparent;
    }
    if c == ZWJ || c == '\u{0640}' {
        return Joining::Causing;
    }
    if let Some((_, dual)) = forms(c) {
        return if dual { Joining::Dual } else { Joining::Right };
    }
    match u32::from(c) {
        // Other letters of the Arabic blocks: most are dual-joining.
        0x0620 | 0x063B..=0x063F | 0x066E..=0x066F | 0x0672..=0x06D3 | 0x0750..=0x077F => {
            Joining::Dual
        }
        0x06D5 | 0x06EE..=0x06EF => Joining::Right,
        _ => Joining::None,
    }
}

/// Whether `c` is a right-to-left letter (Hebrew, Arabic and the scripts
/// near them, and their presentation forms).
#[must_use]
pub fn is_rtl(c: char) -> bool {
    !is_mark(c)
        && !is_number(c)
        && matches!(u32::from(c),
            0x0590..=0x05FF
            | 0x0608 | 0x060B | 0x060D
            | 0x061B..=0x064A
            | 0x066D..=0x06D5
            | 0x06E5..=0x06E6
            | 0x06EE..=0x06FF
            | 0x0700..=0x08FF
            | 0xFB1D..=0xFDFF
            | 0xFE70..=0xFEFE
            | 0x10800..=0x10FFF
            | 0x1E800..=0x1EFFF)
}

/// Whether `line` reads right to left: its first letter is a right-to-left
/// one.
#[must_use]
pub fn is_rtl_line(line: &str) -> bool {
    line.chars().find_map(strong) == Some(Dir::Rtl)
}

fn is_number(c: char) -> bool {
    c.is_ascii_digit()
        || matches!(u32::from(c), 0x0660..=0x0669 | 0x06F0..=0x06F9 | 0xFF10..=0xFF19)
}

/// Characters that join two numbers (`1,000`, `3.5`, `12:30`, `1/2`) and
/// signs before one (`-5`, `+10`).
fn is_number_separator(c: char) -> bool {
    matches!(
        c,
        ',' | '.' | ':' | '/' | '\u{060C}' | '\u{066B}' | '\u{066C}' | '\u{00A0}'
    )
}

/// Invisible formatting characters, dropped once they've done their job.
fn is_format(c: char) -> bool {
    matches!(u32::from(c), 0x200B..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069 | 0xFEFF)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Ltr,
    Rtl,
}

fn strong(c: char) -> Option<Dir> {
    if is_rtl(c) {
        Some(Dir::Rtl)
    } else if c.is_alphabetic() && !is_mark(c) {
        Some(Dir::Ltr)
    } else {
        None
    }
}

/// Picks each Arabic-script letter's joined form (logical order kept).
/// `has` says whether the fonts have a character; forms they lack stay as
/// the plain letter.
fn shape(chars: &[char], has: &dyn Fn(char) -> bool) -> Vec<char> {
    let n = chars.len();
    // The joining type of the nearest non-mark neighbour on each side.
    let before = |i: usize| {
        chars[..i]
            .iter()
            .rev()
            .map(|&c| joining(c))
            .find(|&j| j != Joining::Transparent)
    };
    let after = |i: usize| {
        chars[i + 1..]
            .iter()
            .map(|&c| joining(c))
            .find(|&j| j != Joining::Transparent)
    };
    let joins_back = |i: usize| matches!(before(i), Some(Joining::Dual | Joining::Causing));
    let joins_on = |i: usize| {
        matches!(
            after(i),
            Some(Joining::Dual | Joining::Right | Joining::Causing)
        )
    };
    let pick = |form: u32, plain: char| char::from_u32(form).filter(|&f| has(f)).unwrap_or(plain);
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == LAM && i + 1 < n {
            let next = u32::from(chars[i + 1]);
            if let Some(&(_, lig)) = LAM_ALEF.iter().find(|l| l.0 == next) {
                let form = lig + u32::from(joins_back(i));
                if let Some(l) = char::from_u32(form).filter(|&l| has(l)) {
                    out.push(l);
                    i += 2;
                    continue;
                }
            }
        }
        let shaped = match (forms(c), joining(c)) {
            (Some((isolated, true)), _) => {
                let offset = match (joins_back(i), joins_on(i)) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    (true, true) => 3,
                };
                pick(isolated + offset, c)
            }
            (Some((isolated, false)), _) => pick(isolated + u32::from(joins_back(i)), c),
            _ => c,
        };
        out.push(shaped);
        i += 1;
    }
    out
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        _ => c,
    }
}

/// A cluster's bidi class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Strong(Dir),
    Number,
    Neutral,
}

/// Each cluster's class (by its first character): letters by direction,
/// numbers with the separators, signs and percent signs that belong to
/// them, and numbers after left-to-right text counted as part of it.
fn classify(firsts: &[char], base: Dir) -> Vec<Class> {
    let mut class: Vec<Class> = firsts
        .iter()
        .map(|&c| {
            if is_number(c) {
                Class::Number
            } else {
                strong(c).map_or(Class::Neutral, Class::Strong)
            }
        })
        .collect();
    for i in 0..firsts.len() {
        let c = firsts[i];
        let next_number = class.get(i + 1) == Some(&Class::Number);
        let prev_number = i > 0 && class[i - 1] == Class::Number;
        if class[i] == Class::Neutral
            && next_number
            && ((is_number_separator(c) && prev_number)
                || (matches!(c, '-' | '+' | '−') && !prev_number))
        {
            class[i] = Class::Number;
        }
        if class[i] == Class::Neutral && prev_number && matches!(c, '%' | '٪') {
            class[i] = Class::Number;
        }
    }
    let mut last = base;
    for k in &mut class {
        match *k {
            Class::Strong(d) => last = d,
            Class::Number if last == Dir::Ltr => *k = Class::Strong(Dir::Ltr),
            _ => {}
        }
    }
    class
}

/// Each cluster's embedding level: neutrals take the direction around them
/// when both sides agree (numbers count as right-to-left), else the
/// line's; right-to-left text is odd, numbers and left-to-right text in it
/// one above, and trailing spaces the line's own level.
fn levels(firsts: &[char], class: &[Class], base: Dir) -> Vec<u8> {
    let side = |k: Class| match k {
        Class::Strong(d) => Some(d),
        Class::Number => Some(Dir::Rtl),
        Class::Neutral => None,
    };
    let mut dir: Vec<Option<Dir>> = class.iter().map(|&k| side(k)).collect();
    let mut i = 0;
    while i < dir.len() {
        if dir[i].is_some() {
            i += 1;
            continue;
        }
        let start = i;
        while i < dir.len() && dir[i].is_none() {
            i += 1;
        }
        let left = if start == 0 {
            base
        } else {
            dir[start - 1].unwrap_or(base)
        };
        let right = if i == dir.len() {
            base
        } else {
            dir[i].unwrap_or(base)
        };
        let d = if left == right { left } else { base };
        for slot in &mut dir[start..i] {
            *slot = Some(d);
        }
    }
    let base_level = u8::from(base == Dir::Rtl);
    let mut levels: Vec<u8> = class
        .iter()
        .zip(&dir)
        .map(|(&k, &d)| match (k, d.unwrap_or(base)) {
            (Class::Number, _) => 2,
            (_, Dir::Rtl) => 1,
            (_, Dir::Ltr) => base_level * 2,
        })
        .collect();
    for (c, level) in firsts.iter().zip(levels.iter_mut()).rev() {
        if c.is_whitespace() {
            *level = base_level;
        } else {
            break;
        }
    }
    levels
}

/// The drawing order of clusters at these levels: runs reversed from the
/// highest level down.
fn reorder(levels: &[u8]) -> Vec<usize> {
    let top = levels.iter().copied().max().unwrap_or(0);
    let mut order: Vec<usize> = (0..levels.len()).collect();
    for k in (1..=top).rev() {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] < k {
                i += 1;
                continue;
            }
            let start = i;
            while i < order.len() && levels[order[i]] >= k {
                i += 1;
            }
            order[start..i].reverse();
        }
    }
    order
}

/// One line (no `\n`) in drawing order: Arabic-script letters joined, and
/// right-to-left runs reversed. `has` says whether the fonts have a
/// character (forms they lack fall back to the plain letter). Returns the
/// characters and whether the line reads right to left.
#[must_use]
pub fn visual_line(line: &str, has: &dyn Fn(char) -> bool) -> (String, bool) {
    let rtl_line = is_rtl_line(line);
    if !rtl_line && !line.chars().any(is_rtl) {
        return (line.to_string(), false);
    }
    let chars: Vec<char> = line.chars().collect();
    let base = if rtl_line { Dir::Rtl } else { Dir::Ltr };
    // Group marks with the letter before them, so they stay after it.
    let mut clusters: Vec<Vec<char>> = Vec::new();
    for c in shape(&chars, has) {
        match clusters.last_mut() {
            Some(last) if is_mark(c) => last.push(c),
            _ => clusters.push(vec![c]),
        }
    }
    let firsts: Vec<char> = clusters.iter().map(|cl| cl[0]).collect();
    let class = classify(&firsts, base);
    let levels = levels(&firsts, &class, base);
    // Brackets mirror in right-to-left runs.
    for (cl, &level) in clusters.iter_mut().zip(&levels) {
        if level % 2 == 1 {
            cl[0] = mirror(cl[0]);
        }
    }
    let text = reorder(&levels)
        .iter()
        .flat_map(|&i| clusters[i].iter().copied())
        .filter(|&c| !is_format(c))
        .collect();
    (text, rtl_line)
}

/// [`visual_line`] for every line of `text`, assuming the fonts have every
/// presentation form.
#[must_use]
pub fn visual(text: &str) -> String {
    text.split('\n')
        .map(|l| visual_line(l, &|_| true).0)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(s: &str) -> Vec<u32> {
        s.chars().map(u32::from).collect()
    }

    #[test]
    fn the_forms_table_is_sorted_for_binary_search() {
        assert!(FORMS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn letters_join_and_lam_alef_becomes_one() {
        // سلام: seen initial, lam-alef final, meem isolated (alef doesn't
        // join forward), drawn right to left.
        assert_eq!(codes(&visual("سلام")), [0xFEE1, 0xFEFC, 0xFEB3]);
        // Persian letters: پیروز (peh init, farsi yeh medial, reh final,
        // waw isolated, zain isolated).
        assert_eq!(
            codes(&visual("پیروز")),
            [0xFEAF, 0xFEED, 0xFEAE, 0xFBFF, 0xFB58]
        );
        // گ alone, and a word made of one dual letter twice.
        assert_eq!(codes(&visual("گ")), [0xFB92]);
        assert_eq!(codes(&visual("بب")), [0xFE90, 0xFE91]);
    }

    #[test]
    fn zwnj_breaks_a_join_and_is_dropped() {
        // می‌روم: meem init + yeh final, then (after the half-space) reh
        // isolated, waw isolated, meem isolated.
        let v = visual("می\u{200C}روم");
        assert!(!v.contains('\u{200C}'));
        assert_eq!(codes(&v), [0xFEE1, 0xFEED, 0xFEAD, 0xFBFD, 0xFEE3]);
        // ZWJ forces one: a lone beh followed by ZWJ is initial.
        assert_eq!(codes(&visual("ب\u{200D}")), [0xFE91]);
    }

    #[test]
    fn marks_dont_break_joins_and_stay_with_their_letter() {
        // بَب: the fatha sits between two joined behs.
        let v = codes(&visual("ب\u{064E}ب"));
        assert_eq!(v, [0xFE90, 0xFE91, 0x064E]);
    }

    #[test]
    fn numbers_and_latin_words_keep_their_order_in_a_persian_line() {
        assert!(is_rtl_line("امتیاز 120"));
        // "Score 120" in Persian: the number ends up on the left.
        let v = visual("امتیاز 120");
        assert!(v.starts_with("120 "), "{v}");
        let v = visual("امتیاز: 1,250 از 3000");
        assert!(v.starts_with("3000 "), "{v}");
        assert!(v.contains(" 1,250"), "{v}");
        // A Latin word inside stays readable.
        let v = visual("بازی Shape Signal است");
        assert!(v.contains("Shape Signal"), "{v}");
        assert_eq!(
            v.chars().next(),
            Some('\u{FE96}'),
            "the last word comes first: {v}"
        );
        // Persian digits are numbers too.
        assert!(visual("زمان ۴۵ ثانیه").contains("۴۵"));
        // Signs and percentages stay with their number.
        assert!(visual("صدا 80%").starts_with("80% "));
        assert!(visual("دما -5").starts_with("-5 "));
    }

    #[test]
    fn brackets_mirror_in_right_to_left_text() {
        let v = visual("سلام (۱)");
        assert!(v.starts_with("(۱)"), "{v:?}");
    }

    #[test]
    fn left_to_right_lines_with_persian_inside() {
        assert!(!is_rtl_line("Language: فارسی"));
        let v = visual("Language: فارسی!");
        assert!(v.starts_with("Language: "));
        assert!(v.ends_with('!'));
        assert_eq!(
            v.chars().nth(10),
            Some('\u{FBFD}'),
            "the Persian word reversed: {v}"
        );
        // Plain text is untouched.
        assert_eq!(visual("Coins: 3\nプレイ"), "Coins: 3\nプレイ");
    }

    #[test]
    fn forms_the_fonts_lack_fall_back_to_the_letter() {
        let none = |_| false;
        let (v, rtl) = visual_line("سلام", &none);
        assert!(rtl);
        assert_eq!(v, "مالس");
    }
}
