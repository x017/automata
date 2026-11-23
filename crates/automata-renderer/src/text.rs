//! HUD text: the built-in font, line layout, and a glyph atlas drawn as
//! sprites.
//!
//! Text is laid out in screen pixels (top-left origin, `+y` down) and drawn
//! as one sprite per glyph, sampling a shared atlas texture. The engine's
//! `ctx.screen.text` and Oberon's Play-mode HUD both go through
//! [`TextAtlas`], so text looks the same in the editor and in the game.
//!
//! Use it in two passes per frame: [`TextAtlas::prepare`] every string you
//! are about to draw (this rasterizes new glyphs and uploads the atlas at
//! most once), then [`TextAtlas::sprites`] each one.
//!
//! Besides the built-in font, an atlas holds any `.ttf` / `.otf` files a
//! game asks for ([`TextAtlas::font`]), each known by a [`FontId`].
//! Characters a font doesn't have are drawn with the built-in font.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A parsed font, shareable and cheap to clone.
pub use ab_glyph::FontArc;
use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont};
use glam::Vec2;

use crate::draw::Sprite;
use crate::renderer::Renderer;
use crate::texture::Texture;

/// The built-in HUD font: Geist Medium (SIL Open Font License,
/// `assets/Geist-LICENSE.txt`).
pub const BUILTIN_FONT: &[u8] = include_bytes!("../assets/Geist-Medium.ttf");

/// Smallest and largest text size, in pixels.
pub const SIZE_RANGE: (f32, f32) = (1.0, 256.0);

const ATLAS_SIZE: u32 = 1024;
/// Empty pixels around each glyph in the atlas (keeps filtering clean).
const PAD: u32 = 1;

/// A font in a [`TextAtlas`]: the built-in one, or a file it loaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(u16);

impl FontId {
    /// The built-in font (Geist Medium).
    pub const BUILTIN: Self = Self(0);

    fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// Whether `path` names a font file the engine reads (`.ttf` or `.otf`).
#[must_use]
pub fn is_font_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
}

/// Parses font file bytes.
///
/// # Errors
///
/// When the bytes aren't a TrueType or OpenType font.
pub fn parse_font(bytes: Vec<u8>) -> Result<FontArc, String> {
    FontArc::try_from_vec(bytes).map_err(|_| "not a TrueType or OpenType font".to_string())
}

/// Reads and parses a `.ttf` / `.otf` file.
///
/// # Errors
///
/// When the file isn't a font file by name, can't be read, or doesn't parse.
pub fn load_font_file(path: &Path) -> Result<FontArc, String> {
    if !is_font_path(path) {
        return Err(format!("font {}: use a .ttf or .otf file", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("font {}: {e}", path.display()))?;
    parse_font(bytes).map_err(|e| format!("font {}: {e}", path.display()))
}

/// Where the anchor point sits on each line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlign {
    /// The anchor is the left edge.
    #[default]
    Left,
    /// The anchor is the centre.
    Center,
    /// The anchor is the right edge.
    Right,
}

impl TextAlign {
    /// Parses `"left"`, `"center"` (or `"centre"`) and `"right"`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "center" | "centre" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// The name [`parse`](Self::parse) accepts.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// How a string is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Line height in pixels (ascender to descender).
    pub size: f32,
    /// Fill colour.
    pub color: [f32; 4],
    /// Horizontal anchoring.
    pub align: TextAlign,
    /// A drop shadow below-right of the text, in this colour.
    pub shadow: Option<[f32; 4]>,
    /// An outline around the text, in this colour.
    pub outline: Option<[f32; 4]>,
    /// The font, from [`TextAtlas::font`] (the built-in one by default).
    pub font: FontId,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 24.0,
            color: [1.0; 4],
            align: TextAlign::Left,
            shadow: None,
            outline: None,
            font: FontId::BUILTIN,
        }
    }
}

impl TextStyle {
    /// The size, clamped to [`SIZE_RANGE`] (non-finite sizes become 24).
    #[must_use]
    pub fn clamped_size(&self) -> f32 {
        if self.size.is_finite() {
            self.size.clamp(SIZE_RANGE.0, SIZE_RANGE.1)
        } else {
            24.0
        }
    }

    /// How far the shadow sits from the text, and the outline's width.
    #[must_use]
    pub fn effect_offset(&self) -> f32 {
        (self.clamped_size() / 16.0).round().max(1.0)
    }
}

/// One glyph placed by [`layout`]: its id, and its origin on the baseline
/// in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    /// The font's glyph.
    pub id: GlyphId,
    /// Origin, relative to the anchor point.
    pub origin: Vec2,
    /// The glyph comes from the fallback font (the main one lacks it).
    pub fallback: bool,
}

/// A laid-out string.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    /// Every visible glyph.
    pub glyphs: Vec<PlacedGlyph>,
    /// Width of the widest line.
    pub width: f32,
    /// Height of all lines together.
    pub height: f32,
}

/// The built-in font.
///
/// # Panics
///
/// Never: the embedded font is a valid TrueType file (checked by tests).
#[must_use]
pub fn builtin_font() -> FontRef<'static> {
    FontRef::try_from_slice(BUILTIN_FONT).expect("the built-in font parses")
}

/// Lays `text` out around the anchor `(0, 0)`.
///
/// Lines (split on `\n`) are stacked and the block is centred vertically on
/// the anchor; each line is placed horizontally by `align`. Glyph origins
/// are snapped to whole pixels so glyphs draw crisply at 1:1.
#[must_use]
pub fn layout(font: &impl Font, text: &str, size: f32, align: TextAlign) -> TextLayout {
    layout_with::<_, FontRef<'static>>(font, None, text, size, align)
}

/// [`layout`], drawing characters `font` lacks from `fallback` (sized to
/// the same pixel height; lines keep `font`'s metrics).
#[must_use]
pub fn layout_with<F: Font, G: Font>(
    font: &F,
    fallback: Option<&G>,
    text: &str,
    size: f32,
    align: TextAlign,
) -> TextLayout {
    let scaled = font.as_scaled(PxScale::from(size));
    let backup = fallback.map(|f| f.as_scaled(PxScale::from(size)));
    let line_height = scaled.height() + scaled.line_gap();
    let lines: Vec<&str> = text.split('\n').collect();
    #[allow(clippy::cast_precision_loss)]
    let height = scaled.height() + line_height * (lines.len() - 1) as f32;
    let top = -height / 2.0;
    let mut glyphs = Vec::new();
    let mut widest = 0.0_f32;
    for (row, line) in lines.iter().enumerate() {
        let mut pen = 0.0_f32;
        let mut prev: Option<(GlyphId, bool)> = None;
        let start = glyphs.len();
        for c in line.chars() {
            let mut id = font.glyph_id(c);
            let mut from_backup = false;
            if id.0 == 0 {
                if let Some(b) = &backup {
                    let other = b.glyph_id(c);
                    if other.0 != 0 {
                        (id, from_backup) = (other, true);
                    }
                }
            }
            match (prev, &backup) {
                (Some((p, false)), _) if !from_backup => pen += scaled.kern(p, id),
                (Some((p, true)), Some(b)) if from_backup => pen += b.kern(p, id),
                _ => {}
            }
            if !c.is_whitespace() {
                glyphs.push(PlacedGlyph {
                    id,
                    origin: Vec2::new(pen, 0.0),
                    fallback: from_backup,
                });
            }
            pen += match &backup {
                Some(b) if from_backup => b.h_advance(id),
                _ => scaled.h_advance(id),
            };
            prev = Some((id, from_backup));
        }
        widest = widest.max(pen);
        let shift = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => -pen / 2.0,
            TextAlign::Right => -pen,
        };
        #[allow(clippy::cast_precision_loss)]
        let baseline = top + scaled.ascent() + line_height * row as f32;
        for g in &mut glyphs[start..] {
            g.origin = Vec2::new((g.origin.x + shift).round(), baseline.round());
        }
    }
    TextLayout {
        glyphs,
        width: widest,
        height,
    }
}

/// The pixel size of `text` drawn at `size` with the built-in font.
#[must_use]
pub fn measure(text: &str, size: f32) -> (f32, f32) {
    let l = layout(&builtin_font(), text, size, TextAlign::Left);
    (l.width, l.height)
}

/// The pixel size of `text` drawn at `size` with `font` (and the built-in
/// font for characters it lacks), as [`TextAtlas::sprites`] draws it.
#[must_use]
pub fn measure_font(font: &FontArc, text: &str, size: f32) -> (f32, f32) {
    let l = layout_with(font, Some(&builtin_font()), text, size, TextAlign::Left);
    (l.width, l.height)
}

/// Where a rasterized glyph lives in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AtlasGlyph {
    /// `(u0, v0, u1, v1)`.
    uv: [f32; 4],
    /// The bitmap's top-left, relative to the glyph origin.
    offset: Vec2,
    /// Bitmap size in pixels.
    size: Vec2,
}

/// A glyph in the atlas: font, glyph and size key.
type GlyphKey = (FontId, GlyphId, u32);

/// Rasterized glyphs packed into one texture, turned into sprites.
pub struct TextAtlas {
    /// `fonts[0]` is the built-in font.
    fonts: Vec<FontArc>,
    /// Font files by path: their id, or why they didn't load.
    files: HashMap<PathBuf, Result<FontId, String>>,
    /// Load failures not yet handed out by [`TextAtlas::take_font_errors`].
    new_errors: Vec<String>,
    pixels: Vec<u8>,
    /// `None` for glyphs with nothing to draw.
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
    cursor: (u32, u32),
    row_height: u32,
    texture: Option<Texture>,
    dirty: bool,
}

impl std::fmt::Debug for TextAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextAtlas")
            .field("fonts", &self.fonts.len())
            .field("glyphs", &self.glyphs.len())
            .finish_non_exhaustive()
    }
}

impl Default for TextAtlas {
    fn default() -> Self {
        Self::new()
    }
}

/// The atlas key for a size: half-pixel steps.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn size_key(size: f32) -> u32 {
    (size * 2.0).round() as u32
}

impl TextAtlas {
    /// An empty atlas using the built-in font.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fonts: vec![FontArc::new(builtin_font())],
            files: HashMap::new(),
            new_errors: Vec::new(),
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            glyphs: HashMap::new(),
            cursor: (PAD, PAD),
            row_height: 0,
            texture: None,
            dirty: false,
        }
    }

    /// Number of distinct glyph/size pairs rasterized so far.
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Adds a parsed font and returns its id.
    ///
    /// # Panics
    ///
    /// After 65 535 fonts.
    pub fn add_font(&mut self, font: FontArc) -> FontId {
        let id = FontId(u16::try_from(self.fonts.len()).expect("fewer than 65536 fonts"));
        self.fonts.push(font);
        id
    }

    /// The font file at `path`, loaded the first time it's asked for. A
    /// file that doesn't load gives the built-in font, and its error is
    /// reported once through [`take_font_errors`](Self::take_font_errors).
    pub fn font(&mut self, path: &Path) -> FontId {
        if let Some(known) = self.files.get(path) {
            return *known.as_ref().unwrap_or(&FontId::BUILTIN);
        }
        let loaded = load_font_file(path).map(|f| self.add_font(f));
        if let Err(e) = &loaded {
            self.new_errors.push(e.clone());
        }
        let id = *loaded.as_ref().unwrap_or(&FontId::BUILTIN);
        self.files.insert(path.to_path_buf(), loaded);
        id
    }

    /// Font files that failed to load since the last call, one message
    /// each (shown once, not every frame).
    pub fn take_font_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.new_errors)
    }

    /// Number of fonts, the built-in one included.
    #[must_use]
    pub fn font_count(&self) -> usize {
        self.fonts.len()
    }

    fn face(&self, id: FontId) -> &FontArc {
        self.fonts.get(id.index()).unwrap_or(&self.fonts[0])
    }

    /// Lays out `text` in `font`, with the built-in font for characters it
    /// lacks.
    fn layout(&self, font: FontId, text: &str, size: f32, align: TextAlign) -> TextLayout {
        let face = self.face(font);
        if font == FontId::BUILTIN {
            layout(face, text, size, align)
        } else {
            layout_with(face, Some(&self.fonts[0]), text, size, align)
        }
    }

    fn clear(&mut self) {
        self.pixels.fill(0);
        self.glyphs.clear();
        self.cursor = (PAD, PAD);
        self.row_height = 0;
        self.dirty = true;
    }

    /// Rasterizes one glyph into the atlas. `false` when the atlas is full.
    fn add(&mut self, glyph_key: GlyphKey) -> bool {
        let (font, id, key) = glyph_key;
        #[allow(clippy::cast_precision_loss)]
        let size = key as f32 / 2.0;
        let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(0.0, 0.0));
        let Some(outline) = self.face(font).outline_glyph(glyph) else {
            self.glyphs.insert(glyph_key, None);
            return true;
        };
        let bounds = outline.px_bounds();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (w, h) = (bounds.width().ceil() as u32, bounds.height().ceil() as u32);
        if self.cursor.0 + w + PAD > ATLAS_SIZE {
            self.cursor = (PAD, self.cursor.1 + self.row_height + PAD);
            self.row_height = 0;
        }
        if self.cursor.1 + h + PAD > ATLAS_SIZE || w + 2 * PAD > ATLAS_SIZE {
            return false;
        }
        let (x0, y0) = self.cursor;
        let stride = ATLAS_SIZE as usize * 4;
        outline.draw(|x, y, coverage| {
            if x < w && y < h {
                let i = (y0 + y) as usize * stride + (x0 + x) as usize * 4;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let a = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
                self.pixels[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
            }
        });
        #[allow(clippy::cast_precision_loss)]
        let (fx, fy, fw, fh, full) = (x0 as f32, y0 as f32, w as f32, h as f32, ATLAS_SIZE as f32);
        self.glyphs.insert(
            glyph_key,
            Some(AtlasGlyph {
                uv: [fx / full, fy / full, (fx + fw) / full, (fy + fh) / full],
                offset: Vec2::new(bounds.min.x.round(), bounds.min.y.round()),
                size: Vec2::new(fw, fh),
            }),
        );
        self.cursor.0 += w + PAD;
        self.row_height = self.row_height.max(h);
        self.dirty = true;
        true
    }

    /// Makes sure every glyph of these strings (text, size, font) is in
    /// the atlas, and uploads the atlas if it changed. Call once per frame,
    /// before [`sprites`](Self::sprites), with everything you'll draw.
    pub fn prepare<'a>(
        &mut self,
        renderer: &Renderer,
        texts: impl IntoIterator<Item = (&'a str, f32, FontId)>,
    ) {
        // Every glyph this frame draws, once each.
        let mut needed: Vec<GlyphKey> = Vec::new();
        for (text, size, font) in texts {
            let style = TextStyle {
                size,
                ..TextStyle::default()
            };
            let key = size_key(style.clamped_size());
            #[allow(clippy::cast_precision_loss)]
            let laid = self.layout(font, text, key as f32 / 2.0, TextAlign::Left);
            for g in laid.glyphs {
                let face = if g.fallback { FontId::BUILTIN } else { font };
                let entry = (face, g.id, key);
                if !needed.contains(&entry) {
                    needed.push(entry);
                }
            }
        }
        let missing: Vec<_> = needed
            .iter()
            .copied()
            .filter(|k| !self.glyphs.contains_key(k))
            .collect();
        if missing.iter().any(|&k| !self.add(k)) {
            // Full: start over with just this frame's glyphs. If even those
            // don't fit, draw what does.
            self.clear();
            for &k in &needed {
                if !self.add(k) {
                    break;
                }
            }
        }
        if self.dirty || self.texture.is_none() {
            self.texture = Some(renderer.create_texture_rgba(ATLAS_SIZE, ATLAS_SIZE, &self.pixels));
            self.dirty = false;
        }
    }

    /// The sprites that draw `text` anchored at `(x, y)` (see [`layout`]),
    /// shadow and outline first. Glyphs must have been
    /// [`prepare`](Self::prepare)d this frame; missing ones are skipped.
    #[must_use]
    pub fn sprites(&self, text: &str, x: f32, y: f32, style: &TextStyle) -> Vec<Sprite> {
        let Some(texture) = &self.texture else {
            return Vec::new();
        };
        let size = style.clamped_size();
        let key = size_key(size);
        #[allow(clippy::cast_precision_loss)]
        let laid = self.layout(style.font, text, key as f32 / 2.0, style.align);
        let anchor = Vec2::new(x.round(), y.round());
        let mut passes: Vec<(Vec2, [f32; 4])> = Vec::new();
        let d = style.effect_offset();
        if let Some(color) = style.shadow {
            passes.push((Vec2::splat(d), color));
        }
        if let Some(color) = style.outline {
            for (dx, dy) in [
                (-1.0, -1.0),
                (0.0, -1.0),
                (1.0, -1.0),
                (-1.0, 0.0),
                (1.0, 0.0),
                (-1.0, 1.0),
                (0.0, 1.0),
                (1.0, 1.0),
            ] {
                passes.push((Vec2::new(dx * d, dy * d), color));
            }
        }
        passes.push((Vec2::ZERO, style.color));
        let mut out = Vec::with_capacity(passes.len() * laid.glyphs.len());
        for (shift, color) in passes {
            for g in &laid.glyphs {
                let face = if g.fallback {
                    FontId::BUILTIN
                } else {
                    style.font
                };
                let Some(Some(a)) = self.glyphs.get(&(face, g.id, key)) else {
                    continue;
                };
                let top_left = anchor + g.origin + a.offset + shift;
                out.push(
                    Sprite::new(texture.clone())
                        .position(top_left + a.size / 2.0)
                        .size(a.size)
                        .color(color)
                        .uv(a.uv),
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_font_parses_and_has_digits_and_letters() {
        let font = builtin_font();
        for c in "0123456789%ABCxyz".chars() {
            assert_ne!(font.glyph_id(c).0, 0, "missing {c:?}");
        }
    }

    #[test]
    fn alignment_moves_the_line_around_the_anchor() {
        let font = builtin_font();
        let left = layout(&font, "100%", 24.0, TextAlign::Left);
        let center = layout(&font, "100%", 24.0, TextAlign::Center);
        let right = layout(&font, "100%", 24.0, TextAlign::Right);
        assert_eq!(left.glyphs.len(), 4);
        assert!(left.width > 30.0 && left.width < 80.0, "{}", left.width);
        assert!(left.glyphs[0].origin.x.abs() < 1.0);
        let first = center.glyphs[0].origin.x;
        assert!((first + left.width / 2.0).abs() <= 1.0, "{first}");
        let last_end = right.glyphs[3].origin.x;
        assert!(last_end < 0.0 && last_end > -left.width);
        assert!((left.height - 24.0).abs() < 1e-3, "{}", left.height);
    }

    #[test]
    fn lines_stack_and_the_block_centres_on_the_anchor() {
        let font = builtin_font();
        let one = layout(&font, "A", 20.0, TextAlign::Left);
        let two = layout(&font, "A\nA", 20.0, TextAlign::Left);
        assert!(
            two.height > one.height * 1.9,
            "{} {}",
            one.height,
            two.height
        );
        let (a, b) = (two.glyphs[0].origin.y, two.glyphs[1].origin.y);
        assert!(b > a, "the second line is lower");
        // Baselines sit symmetrically around the anchor (up to the font's
        // ascender/descender imbalance), so the block is roughly centred.
        assert!(a < 0.0 && b > 0.0, "{a} {b}");
        assert!(
            one.glyphs[0].origin.y > 0.0,
            "a single line's baseline is below the centre"
        );
    }

    #[test]
    fn whitespace_takes_space_but_draws_nothing() {
        let font = builtin_font();
        let l = layout(&font, "a b", 16.0, TextAlign::Left);
        assert_eq!(l.glyphs.len(), 2);
        assert!(
            l.glyphs[1].origin.x
                > layout(&font, "ab", 16.0, TextAlign::Left).glyphs[1]
                    .origin
                    .x
        );
        assert_eq!(measure("", 16.0).0, 0.0);
    }

    #[test]
    fn alignment_names_round_trip() {
        for a in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            assert_eq!(TextAlign::parse(a.name()), Some(a));
        }
        assert_eq!(TextAlign::parse("Centre"), Some(TextAlign::Center));
        assert_eq!(TextAlign::parse("middle"), None);
    }

    #[test]
    fn glyphs_pack_without_overlap() {
        let mut atlas = TextAtlas::new();
        let font = builtin_font();
        for (c, key) in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
            .chars()
            .zip([96, 116, 136].into_iter().cycle())
        {
            assert!(atlas.add((FontId::BUILTIN, font.glyph_id(c), key)));
        }
        let rects: Vec<[f32; 4]> = atlas.glyphs.values().flatten().map(|g| g.uv).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a[2] <= 1.0 && a[3] <= 1.0);
            for b in &rects[i + 1..] {
                let overlap = a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }

    /// A font that isn't the built-in one (Press Start 2P, OFL), from the
    /// Pause Menu example.
    fn pixel_font_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/oberon/pause_menu/fonts/PressStart2P-Regular.ttf")
    }

    #[test]
    fn font_files_are_known_by_extension_and_parse() {
        assert!(is_font_path(Path::new("fonts/Pixel.TTF")));
        assert!(is_font_path(Path::new("a.otf")));
        assert!(!is_font_path(Path::new("a.woff2")));
        let font = load_font_file(&pixel_font_path()).unwrap();
        // A pixel font: every glyph the same width, unlike Geist.
        let (narrow, _) = measure_font(&font, "iiii", 16.0);
        let (wide, _) = measure_font(&font, "WWWW", 16.0);
        assert!((narrow - wide).abs() < 0.5, "{narrow} {wide}");
        assert!(measure("iiii", 16.0).0 < measure("WWWW", 16.0).0 * 0.6);
        let err = parse_font(b"not a font".to_vec()).unwrap_err();
        assert!(err.contains("TrueType"), "{err}");
        assert!(load_font_file(Path::new("x.png"))
            .unwrap_err()
            .contains(".ttf"));
    }

    #[test]
    fn missing_characters_come_from_the_builtin_font() {
        let pixel = load_font_file(&pixel_font_path()).unwrap();
        let builtin = builtin_font();
        // Press Start 2P has no capital sharp s (ẞ); Geist does.
        assert_eq!(pixel.glyph_id('\u{1e9e}').0, 0);
        assert_ne!(builtin.glyph_id('\u{1e9e}').0, 0);
        let alone = layout(&pixel, "A\u{1e9e}", 16.0, TextAlign::Left);
        let backed = layout_with(&pixel, Some(&builtin), "A\u{1e9e}", 16.0, TextAlign::Left);
        assert!(!alone.glyphs[1].fallback);
        assert!(!backed.glyphs[0].fallback && backed.glyphs[1].fallback);
        assert_eq!(backed.glyphs[1].id, builtin.glyph_id('\u{1e9e}'));
        // Line metrics stay the main font's.
        assert!((alone.height - backed.height).abs() < 1e-3);
    }

    #[test]
    fn an_atlas_loads_each_font_file_once_and_reports_failures_once() {
        let mut atlas = TextAtlas::new();
        let path = pixel_font_path();
        let id = atlas.font(&path);
        assert_ne!(id, FontId::BUILTIN);
        assert_eq!(atlas.font(&path), id, "loaded once");
        assert_eq!(atlas.font_count(), 2);
        let missing = Path::new("fonts/nope.ttf");
        assert_eq!(atlas.font(missing), FontId::BUILTIN);
        assert_eq!(atlas.font(missing), FontId::BUILTIN);
        let errors = atlas.take_font_errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("nope.ttf"), "{errors:?}");
        assert_eq!(atlas.take_font_errors(), Vec::<String>::new());
        // The same glyph in two fonts is two atlas entries.
        let a_builtin = (FontId::BUILTIN, builtin_font().glyph_id('A'), 32);
        let a_pixel = (id, atlas.face(id).glyph_id('A'), 32);
        assert!(atlas.add(a_builtin) && atlas.add(a_pixel));
        assert_eq!(atlas.glyph_count(), 2);
        let (b, p) = (
            atlas.glyphs[&a_builtin].unwrap(),
            atlas.glyphs[&a_pixel].unwrap(),
        );
        assert_ne!(b.uv, p.uv);
    }

    #[test]
    fn a_full_atlas_reports_it() {
        let mut atlas = TextAtlas::new();
        let font = builtin_font();
        // Big "W"s at ever-different sizes: a few dozen fit in 1024², then
        // the atlas says it's full instead of overlapping or overflowing.
        let w = font.glyph_id('W');
        let full = (0..400).find(|i| !atlas.add((FontId::BUILTIN, w, 512 - i)));
        assert!(full.is_some_and(|added| added > 8), "{full:?}");
    }
}
