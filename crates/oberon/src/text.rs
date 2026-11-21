//! Text rendering for Oberon: glyph rasterization into the canvas pixmap.
//!
//! Uses `ab_glyph` with embedded fonts (Geist Regular / Medium for the
//! interface, Fira Code for code and logs; all SIL OFL, vendored into the
//! crate) so the editor renders text identically on any platform with no
//! system font lookup. Glyphs a face lacks fall back to Fira Code. Glyphs are rasterized as coverage and blended into the
//! pixmap; this is per-frame UI text, not a shaping engine.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use tiny_skia::{Color, Pixmap};

/// The code font: Fira Code Regular (SIL OFL, `assets/FiraCode-LICENSE.txt`).
const MONO_BYTES: &[u8] = include_bytes!("../assets/FiraCode-Regular.ttf");
/// The interface font: Geist Regular (SIL OFL, `assets/Geist-LICENSE.txt`).
const UI_BYTES: &[u8] = include_bytes!("../assets/Geist-Regular.ttf");
/// Headings and emphasis: Geist Medium.
const UI_MEDIUM_BYTES: &[u8] = automata_renderer::text::BUILTIN_FONT;

/// A type face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Face {
    /// Interface text (Geist Regular).
    #[default]
    Ui,
    /// Headings and emphasis (Geist Medium).
    Medium,
    /// Code, logs and other fixed-width text (Fira Code).
    Mono,
}

impl Face {
    fn index(self) -> usize {
        match self {
            Self::Ui => 0,
            Self::Medium => 1,
            Self::Mono => 2,
        }
    }
}

/// A rasterized glyph's coverage mask, positioned relative to the pen.
struct CachedGlyph {
    /// Offset of the mask's top-left from (pen x, baseline), in pixels.
    dx: i32,
    dy: i32,
    w: u32,
    h: u32,
    coverage: Vec<u8>,
}

/// Rasterizes and draws text into a pixmap.
///
/// Glyph coverage masks are cached per (glyph, size, subpixel offset), so a
/// screen of code costs a few memcpy-sized blends per glyph instead of an
/// outline rasterization each frame.
pub struct TextRenderer {
    fonts: [FontRef<'static>; 3],
    cache: HashMap<(u8, u16, u32, u8), Option<CachedGlyph>>,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Loads the embedded fonts. Panics only if a vendored font is corrupt.
    #[must_use]
    pub fn new() -> Self {
        let load = |b: &'static [u8]| FontRef::try_from_slice(b).expect("embedded font parses");
        Self {
            fonts: [load(UI_BYTES), load(UI_MEDIUM_BYTES), load(MONO_BYTES)],
            cache: HashMap::new(),
        }
    }

    /// The face that draws `ch` in `face` (Fira Code when `face` lacks it),
    /// with its glyph id.
    fn resolve(&self, face: Face, ch: char) -> (usize, ab_glyph::GlyphId) {
        let f = face.index();
        let id = self.fonts[f].glyph_id(ch);
        if id.0 == 0 && f != 2 {
            let mono = self.fonts[2].glyph_id(ch);
            if mono.0 != 0 {
                return (2, mono);
            }
        }
        (f, id)
    }

    /// Per-character layout: (font index, glyph, kerning before, advance).
    fn layout(
        &self,
        face: Face,
        size: f32,
        text: &str,
    ) -> Vec<(usize, ab_glyph::GlyphId, f32, f32)> {
        let px = PxScale::from(size);
        let mut out = Vec::with_capacity(text.len());
        let mut prev: Option<(usize, ab_glyph::GlyphId)> = None;
        for ch in text.chars() {
            let (f, id) = self.resolve(face, ch);
            let scaled = self.fonts[f].as_scaled(px);
            let kern = match prev {
                Some((pf, pid)) if pf == f => scaled.kern(pid, id),
                _ => 0.0,
            };
            out.push((f, id, kern, scaled.h_advance(id)));
            prev = Some((f, id));
        }
        out
    }

    /// Measures the advance width of `text` at `size` (pixels).
    #[must_use]
    pub fn measure(&self, face: Face, size: f32, text: &str) -> f32 {
        self.layout(face, size, text)
            .iter()
            .map(|g| g.2 + g.3)
            .sum()
    }

    /// The advance of one character cell at `size` in the code font.
    #[must_use]
    pub fn cell_width(&self, size: f32) -> f32 {
        let scaled = self.fonts[2].as_scaled(PxScale::from(size));
        scaled.h_advance(scaled.glyph_id('M'))
    }

    /// The line ascent of `face` at `size` (baseline offset from the top).
    #[must_use]
    pub fn ascent(&self, face: Face, size: f32) -> f32 {
        self.fonts[face.index()]
            .as_scaled(PxScale::from(size))
            .ascent()
    }

    fn glyph(
        &mut self,
        font: usize,
        id: ab_glyph::GlyphId,
        size: f32,
        sub: u8,
    ) -> Option<&CachedGlyph> {
        let key = (font as u8, id.0, (size * 4.0).round() as u32, sub);
        let font = &self.fonts[font];
        self.cache
            .entry(key)
            .or_insert_with(|| {
                let x = f32::from(sub) / 4.0;
                let glyph =
                    id.with_scale_and_position(PxScale::from(size), ab_glyph::point(x, 0.0));
                let outlined = font.outline_glyph(glyph)?;
                let b = outlined.px_bounds();
                let (w, h) = (b.width() as u32, b.height() as u32);
                let mut coverage = vec![0u8; (w * h) as usize];
                outlined.draw(|gx, gy, c| {
                    if gx < w && gy < h {
                        coverage[(gy * w + gx) as usize] = (c.clamp(0.0, 1.0) * 255.0) as u8;
                    }
                });
                Some(CachedGlyph {
                    dx: b.min.x as i32,
                    dy: b.min.y as i32,
                    w,
                    h,
                    coverage,
                })
            })
            .as_ref()
    }

    /// Draws `text` with its top-left at `(x, y)`, returning the width drawn.
    /// Pixels outside `clip` (physical `x0, y0, x1, y1`) are not touched.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        face: Face,
        x: f32,
        y: f32,
        size: f32,
        text: &str,
        color: Color,
        clip: Option<[i32; 4]>,
    ) -> f32 {
        if self.cache.len() > 8192 {
            self.cache.clear();
        }
        let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
        let [cx0, cy0, cx1, cy1] = clip.unwrap_or([0, 0, pw, ph]);
        let (cx0, cy0, cx1, cy1) = (cx0.max(0), cy0.max(0), cx1.min(pw), cy1.min(ph));
        // Every face shares the primary face's baseline so fallback glyphs
        // line up.
        let baseline = (y + self.ascent(face, size)).round() as i32;
        let mut pen_x = x;
        let rgba = [
            (color.red() * 255.0) as u32,
            (color.green() * 255.0) as u32,
            (color.blue() * 255.0) as u32,
            (color.alpha() * 255.0) as u32,
        ];
        for (font, id, kern, advance) in self.layout(face, size, text) {
            pen_x += kern;
            let base_x = pen_x.floor();
            let sub = ((pen_x - base_x) * 4.0).round().min(3.0) as u8;
            let gx0 = base_x as i32;
            if gx0 > cx1 {
                break;
            }
            if let Some(g) = self.glyph(font, id, size, sub) {
                let ox = gx0 + g.dx;
                let oy = baseline + g.dy;
                let data = pixmap.data_mut();
                for row in 0..g.h as i32 {
                    let py = oy + row;
                    if py < cy0 || py >= cy1 {
                        continue;
                    }
                    for col in 0..g.w as i32 {
                        let px = ox + col;
                        if px < cx0 || px >= cx1 {
                            continue;
                        }
                        let c = g.coverage[(row * g.w as i32 + col) as usize];
                        if c != 0 {
                            let idx = ((py * pw + px) * 4) as usize;
                            blend(&mut data[idx..idx + 4], rgba, u32::from(c));
                        }
                    }
                }
            }
            pen_x += advance;
        }
        pen_x - x
    }
}

/// Blends `rgba` at `coverage` (0..255) over one premultiplied pixel.
fn blend(px: &mut [u8], rgba: [u32; 4], coverage: u32) {
    let src_a = rgba[3] * coverage / 255;
    let inv = 255 - src_a.min(255);
    px[0] = ((rgba[0] * src_a) / 255 + (u32::from(px[0]) * inv) / 255) as u8;
    px[1] = ((rgba[1] * src_a) / 255 + (u32::from(px[1]) * inv) / 255) as u8;
    px[2] = ((rgba[2] * src_a) / 255 + (u32::from(px[2]) * inv) / 255) as u8;
    px[3] = (src_a + (u32::from(px[3]) * inv) / 255).min(255) as u8;
}
