//! The 2D drawing surface Oberon renders the editor UI into.
//!
//! A thin immediate-mode wrapper over a [`tiny_skia::Pixmap`]: filled and
//! stroked rects, lines, circles, polygons, text (via `ab_glyph`), and
//! blitting RGBA images (the viewport's engine frames).
//!
//! All coordinates are **logical** pixels. The canvas carries a DPI scale
//! factor (the window's `scale_factor`) and maps logical coordinates to the
//! physical pixmap, so the editor is crisp and correctly sized on `HiDPI`
//! displays.

#![forbid(unsafe_code)]

use tiny_skia::{
    Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

pub use crate::text::Face;
use crate::text::TextRenderer;

/// A 2D canvas Oberon draws the editor into each frame.
pub struct Canvas {
    /// The pixel buffer, RGBA premultiplied, row-major (physical pixels).
    pub pixmap: Pixmap,
    text: TextRenderer,
    /// Logical → physical scale (the window's DPI scale factor).
    scale: f32,
    /// Scratch buffer for premultiplying blitted images (reused per frame).
    blit_scratch: Vec<u8>,
    /// Physical clip rectangle `x0, y0, x1, y1` for text and rectangles.
    clip: Option<[i32; 4]>,
    /// The face `text` / `measure_text` use.
    face: Face,
}

impl Canvas {
    /// Creates a canvas of `width`×`height` **physical** pixels at scale 1.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixmap: Pixmap::new(width.max(1), height.max(1)).expect("pixmap allocation"),
            text: TextRenderer::new(),
            scale: 1.0,
            blit_scratch: Vec::new(),
            clip: None,
            face: Face::Ui,
        }
    }

    /// Sets the face for following text calls; returns the previous one.
    pub fn set_face(&mut self, face: Face) -> Face {
        std::mem::replace(&mut self.face, face)
    }

    /// Draws text in `face` (restoring the current face afterwards).
    pub fn text_in(
        &mut self,
        face: Face,
        x: f32,
        y: f32,
        size: f32,
        text: &str,
        color: Color,
    ) -> f32 {
        let prev = self.set_face(face);
        let w = self.text(x, y, size, text, color);
        self.face = prev;
        w
    }

    /// Measures text in `face`.
    pub fn measure_in(&mut self, face: Face, size: f32, text: &str) -> f32 {
        let prev = self.set_face(face);
        let w = self.measure_text(size, text);
        self.face = prev;
        w
    }

    /// The canvas size in physical pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.pixmap.width(), self.pixmap.height())
    }

    /// The canvas size in logical pixels.
    #[must_use]
    pub fn logical_size(&self) -> (f32, f32) {
        (
            self.pixmap.width() as f32 / self.scale,
            self.pixmap.height() as f32 / self.scale,
        )
    }

    /// The logical → physical scale factor.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Sets the logical → physical scale factor (clamped to a sane range).
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = if scale.is_finite() {
            scale.clamp(0.5, 4.0)
        } else {
            1.0
        };
    }

    /// Resizes the pixel buffer (contents are not preserved).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.pixmap = Pixmap::new(width.max(1), height.max(1)).expect("pixmap allocation");
    }

    fn transform(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }

    /// Restricts text and rectangle drawing to a logical rectangle (`None`
    /// lifts the restriction). Returns the previous clip so callers can
    /// restore it.
    pub fn set_clip(&mut self, clip: Option<(f32, f32, f32, f32)>) -> Option<[i32; 4]> {
        let s = self.scale;
        let prev = self.clip;
        self.clip = clip.map(|(x, y, w, h)| {
            [
                (x * s).floor() as i32,
                (y * s).floor() as i32,
                ((x + w) * s).ceil() as i32,
                ((y + h) * s).ceil() as i32,
            ]
        });
        prev
    }

    /// Restores a clip returned by [`Canvas::set_clip`].
    pub fn restore_clip(&mut self, clip: Option<[i32; 4]>) {
        self.clip = clip;
    }

    /// The width of one character cell at `size` (logical; monospaced font).
    pub fn cell_width(&mut self, size: f32) -> f32 {
        let s = self.scale;
        self.text.cell_width(size * s) / s
    }

    /// Clears the whole canvas to `color`.
    pub fn clear(&mut self, color: Color) {
        self.pixmap.fill(color);
    }

    /// Fills a rectangle.
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        if let Some([x0, y0, x1, y1]) = self.clip {
            let s = self.scale;
            let (cx0, cy0, cx1, cy1) = (x0 as f32 / s, y0 as f32 / s, x1 as f32 / s, y1 as f32 / s);
            let (nx0, ny0) = (x.max(cx0), y.max(cy0));
            let (nx1, ny1) = ((x + w).min(cx1), (y + h).min(cy1));
            if nx1 <= nx0 || ny1 <= ny0 {
                return;
            }
            (x, y, w, h) = (nx0, ny0, nx1 - nx0, ny1 - ny0);
        }
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = false;
        let transform = self.transform();
        self.pixmap.fill_rect(rect, &paint, transform, None);
    }

    /// Strokes a rectangle outline.
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color, width: f32) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        let Some(path) = pb.finish() else { return };
        self.stroke_path(&path, color, width);
    }

    /// Fills a rounded rectangle.
    pub fn fill_rounded(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let Some(path) = rounded_rect_path(rect, radius) else {
            return;
        };
        self.fill_path(&path, color);
    }

    /// Strokes a rounded rectangle outline.
    pub fn stroke_rounded(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: Color,
        width: f32,
    ) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let Some(path) = rounded_rect_path(rect, radius) else {
            return;
        };
        self.stroke_path(&path, color, width);
    }

    /// Draws a straight line from `(x0, y0)` to `(x1, y1)`.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, width: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        let Some(path) = pb.finish() else { return };
        self.stroke_path(&path, color, width);
    }

    /// Draws an open polyline through `points`.
    pub fn polyline(&mut self, points: &[(f32, f32)], color: Color, width: f32) {
        if points.len() < 2 {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for p in &points[1..] {
            pb.line_to(p.0, p.1);
        }
        let Some(path) = pb.finish() else { return };
        self.stroke_path(&path, color, width);
    }

    /// Fills a closed polygon through `points`.
    pub fn fill_polygon(&mut self, points: &[(f32, f32)], color: Color) {
        if points.len() < 3 {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for p in &points[1..] {
            pb.line_to(p.0, p.1);
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        self.fill_path(&path, color);
    }

    /// Fills a circle.
    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        let Some(path) = PathBuilder::from_circle(cx, cy, r.max(0.1)) else {
            return;
        };
        self.fill_path(&path, color);
    }

    /// Strokes a circle outline.
    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color, width: f32) {
        let Some(path) = PathBuilder::from_circle(cx, cy, r.max(0.1)) else {
            return;
        };
        self.stroke_path(&path, color, width);
    }

    fn fill_path(&mut self, path: &tiny_skia::Path, color: Color) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let transform = self.transform();
        self.pixmap
            .fill_path(path, &paint, FillRule::Winding, transform, None);
    }

    fn stroke_path(&mut self, path: &tiny_skia::Path, color: Color, width: f32) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        let transform = self.transform();
        self.pixmap
            .stroke_path(path, &paint, &stroke, transform, None);
    }

    /// Draws text at `(x, y)` (top-left), returning the advance width.
    pub fn text(&mut self, x: f32, y: f32, size: f32, text: &str, color: Color) -> f32 {
        let s = self.scale;
        self.text.draw(
            &mut self.pixmap,
            self.face,
            x * s,
            y * s,
            size * s,
            text,
            color,
            self.clip,
        ) / s
    }

    /// Draws text truncated with an ellipsis so it fits in `max_w`.
    pub fn text_clipped(
        &mut self,
        x: f32,
        y: f32,
        size: f32,
        text: &str,
        color: Color,
        max_w: f32,
    ) -> f32 {
        if self.measure_text(size, text) <= max_w {
            return self.text(x, y, size, text, color);
        }
        let ellipsis = "…";
        let budget = max_w - self.measure_text(size, ellipsis);
        let mut fitted = String::new();
        for ch in text.chars() {
            fitted.push(ch);
            if self.measure_text(size, &fitted) > budget {
                fitted.pop();
                break;
            }
        }
        fitted.push_str(ellipsis);
        self.text(x, y, size, &fitted, color)
    }

    /// Measures text width at `size` (logical pixels) without drawing.
    pub fn measure_text(&mut self, size: f32, text: &str) -> f32 {
        let s = self.scale;
        self.text.measure(self.face, size * s, text) / s
    }

    /// Blits an RGBA8 image (row-major, straight alpha) with its top-left
    /// corner at logical `(x, y)`. The image is copied 1:1 in **physical**
    /// pixels (the viewport is rendered at physical resolution).
    pub fn blit_rgba(&mut self, x: f32, y: f32, width: u32, height: u32, rgba: &[u8]) {
        let len = (width * height * 4) as usize;
        if width == 0 || height == 0 || rgba.len() < len {
            return;
        }
        // tiny-skia pixmaps are premultiplied; premultiply into a reused
        // scratch buffer (opaque pixels pass through unchanged).
        let mut premul = std::mem::take(&mut self.blit_scratch);
        premul.clear();
        premul.extend_from_slice(&rgba[..len]);
        for px in premul.chunks_exact_mut(4) {
            let a = u32::from(px[3]);
            if a != 255 {
                px[0] = ((u32::from(px[0]) * a + 127) / 255) as u8;
                px[1] = ((u32::from(px[1]) * a + 127) / 255) as u8;
                px[2] = ((u32::from(px[2]) * a + 127) / 255) as u8;
            }
        }
        let Some(size) = tiny_skia::IntSize::from_wh(width, height) else {
            self.blit_scratch = premul;
            return;
        };
        let Some(src) = Pixmap::from_vec(premul, size) else {
            return;
        };
        let paint = PixmapPaint::default();
        self.pixmap.draw_pixmap(
            (x * self.scale).round() as i32,
            (y * self.scale).round() as i32,
            src.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
        // Recover the allocation for the next frame.
        self.blit_scratch = src.take();
    }

    /// The raw premultiplied RGBA pixels (for the window upload).
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.data()
    }
}

/// Builds a rounded-rectangle path for `rect` with corner `radius`.
fn rounded_rect_path(rect: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    let (x0, y0, x1, y1) = (rect.x(), rect.y(), rect.right(), rect.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + r, y0);
    pb.line_to(x1 - r, y0);
    pb.quad_to(x1, y0, x1, y0 + r);
    pb.line_to(x1, y1 - r);
    pb.quad_to(x1, y1, x1 - r, y1);
    pb.line_to(x0 + r, y1);
    pb.quad_to(x0, y1, x0, y1 - r);
    pb.line_to(x0, y0 + r);
    pb.quad_to(x0, y0, x0 + r, y0);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_size_follows_scale() {
        let mut c = Canvas::new(200, 100);
        c.set_scale(2.0);
        assert_eq!(c.logical_size(), (100.0, 50.0));
    }

    #[test]
    fn scaled_fill_covers_physical_pixels() {
        let mut c = Canvas::new(40, 40);
        c.set_scale(2.0);
        c.clear(Color::BLACK);
        c.fill_rect(0.0, 0.0, 10.0, 10.0, Color::WHITE);
        // Logical (5,5) → physical (10,10): inside the white square.
        let px = c.pixmap.pixel(10, 10).unwrap();
        assert_eq!(px.red(), 255);
        // Physical (30,30) is outside it.
        assert_eq!(c.pixmap.pixel(30, 30).unwrap().red(), 0);
    }

    #[test]
    fn blit_reuses_its_scratch_buffer() {
        let mut c = Canvas::new(8, 8);
        let img = vec![255u8; 4 * 4 * 4];
        c.blit_rgba(0.0, 0.0, 4, 4, &img);
        c.blit_rgba(0.0, 0.0, 4, 4, &img);
        assert!(c.blit_scratch.capacity() >= img.len());
    }
}
