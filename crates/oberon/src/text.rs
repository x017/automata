//! Text rendering for Oberon: glyph rasterization into the canvas pixmap.
//!
//! Uses `ab_glyph` with an embedded font (the vendored Cantarell regular)
//! so the editor renders text identically on any platform with no system
//! font lookup. Glyphs are rasterized as coverage and blended into the
//! pixmap; this is per-frame UI text, not a shaping engine.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use tiny_skia::{Color, Pixmap};

/// The embedded UI font (vendored Cantarell regular).
const FONT_BYTES: &[u8] =
    include_bytes!("../../../vendor/sctk-adwaita/src/title/Cantarell-Regular.ttf");

/// Rasterizes and draws text into a pixmap.
pub struct TextRenderer {
    font: FontRef<'static>,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Loads the embedded font. Panics only if the vendored font is corrupt.
    pub fn new() -> Self {
        let font = FontRef::try_from_slice(FONT_BYTES).expect("embedded font parses");
        Self { font }
    }

    /// Measures the advance width of `text` at `size` (pixels).
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

    /// The line ascent at `size` (baseline offset from the top).
    pub fn ascent(&self, size: f32) -> f32 {
        self.font.as_scaled(PxScale::from(size)).ascent()
    }

    /// Draws `text` with its top-left at `(x, y)`, returning the width drawn.
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        x: f32,
        y: f32,
        size: f32,
        text: &str,
        color: Color,
    ) -> f32 {
        let scaled = self.font.as_scaled(PxScale::from(size));
        let baseline = y + scaled.ascent();
        let mut pen_x = x;
        let mut prev = None;
        let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);

        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                pen_x += scaled.kern(p, id);
            }
            let glyph =
                id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen_x, baseline));
            if let Some(outlined) = self.font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                let min_x = bounds.min.x as i32;
                let min_y = bounds.min.y as i32;
                outlined.draw(|gx, gy, coverage| {
                    let px = min_x + gx as i32;
                    let py = min_y + gy as i32;
                    if px < 0 || py < 0 || px >= pw || py >= ph {
                        return;
                    }
                    blend_pixel(pixmap, px as u32, py as u32, color, coverage);
                });
            }
            pen_x += scaled.h_advance(id);
            prev = Some(id);
        }
        let _ = baseline;
        pen_x - x
    }
}

/// Alpha-blends a coverage-weighted `color` onto one premultiplied pixel.
fn blend_pixel(pixmap: &mut Pixmap, x: u32, y: u32, color: Color, coverage: f32) {
    let idx = ((y * pixmap.width() + x) * 4) as usize;
    let data = pixmap.data_mut();
    let (r, g, b, a) = (
        (color.red() * 255.0) as u32,
        (color.green() * 255.0) as u32,
        (color.blue() * 255.0) as u32,
        (color.alpha() * 255.0) as u32,
    );
    // Source is the text color at `coverage` alpha, over the destination.
    let src_a = (a as f32 * coverage) as u32;
    let inv = 255u32.saturating_sub(src_a);
    data[idx] = (((r * src_a) / 255) + (data[idx] as u32 * inv) / 255) as u8;
    data[idx + 1] = (((g * src_a) / 255) + (data[idx + 1] as u32 * inv) / 255) as u8;
    data[idx + 2] = (((b * src_a) / 255) + (data[idx + 2] as u32 * inv) / 255) as u8;
    data[idx + 3] = (src_a + (data[idx + 3] as u32 * inv) / 255).min(255) as u8;
}
