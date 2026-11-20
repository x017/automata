//! Text rendering for Oberon: glyph rasterization into the canvas pixmap.
//!
//! Uses `ab_glyph` with an embedded font (Fira Code Regular, vendored into
//! the crate) so the editor renders text identically on any platform with no
//! system font lookup. Glyphs are rasterized as coverage and blended into the
//! pixmap; this is per-frame UI text, not a shaping engine.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use tiny_skia::{Color, Pixmap};

/// The embedded UI font: Fira Code Regular (SIL OFL), vendored in-crate so
/// the build never depends on a system font or an external vendor path.
/// The license ships alongside the font at `assets/FiraCode-LICENSE.txt`.
const FONT_BYTES: &[u8] = include_bytes!("../assets/FiraCode-Regular.ttf");

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
    font: FontRef<'static>,
    cache: HashMap<(u16, u32, u8), Option<CachedGlyph>>,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Loads the embedded font. Panics only if the vendored font is corrupt.
    #[must_use]
    pub fn new() -> Self {
        let font = FontRef::try_from_slice(FONT_BYTES).expect("embedded font parses");
        Self {
            font,
            cache: HashMap::new(),
        }
    }

    /// Measures the advance width of `text` at `size` (pixels).
    #[must_use]
    pub fn measure(&self, size: f32, text: &str) -> f32 {
        let scaled = self.font.as_scaled(PxScale::from(size));
        let mut width = 0.0f32;
        let mut prev = None;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                width += scaled.kern(p, id);
            }
            width += scaled.h_advance(id);
            prev = Some(id);
        }
        width
    }

    /// The advance of one character cell at `size` (the font is monospaced).
    #[must_use]
    pub fn cell_width(&self, size: f32) -> f32 {
        let scaled = self.font.as_scaled(PxScale::from(size));
        scaled.h_advance(scaled.glyph_id('M'))
    }

    /// The line ascent at `size` (baseline offset from the top).
    #[must_use]
    pub fn ascent(&self, size: f32) -> f32 {
        self.font.as_scaled(PxScale::from(size)).ascent()
    }

    fn glyph(&mut self, id: ab_glyph::GlyphId, size: f32, sub: u8) -> Option<&CachedGlyph> {
        let key = (id.0, (size * 4.0).round() as u32, sub);
        let font = &self.font;
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
        let scaled = self.font.as_scaled(PxScale::from(size));
        let baseline = (y + scaled.ascent()).round() as i32;
        let mut pen_x = x;
        let mut prev = None;
        let rgba = [
            (color.red() * 255.0) as u32,
            (color.green() * 255.0) as u32,
            (color.blue() * 255.0) as u32,
            (color.alpha() * 255.0) as u32,
        ];
        let mut glyphs = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            let kern = prev.map_or(0.0, |p| scaled.kern(p, id));
            glyphs.push((id, kern, scaled.h_advance(id)));
            prev = Some(id);
        }
        for (id, kern, advance) in glyphs {
            pen_x += kern;
            let base_x = pen_x.floor();
            let sub = ((pen_x - base_x) * 4.0).round().min(3.0) as u8;
            let gx0 = base_x as i32;
            if gx0 > cx1 {
                break;
            }
            if let Some(g) = self.glyph(id, size, sub) {
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
