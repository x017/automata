//! The 2D drawing surface Oberon renders the editor UI into.
//!
//! A thin immediate-mode wrapper over a [`tiny_skia::Pixmap`]: filled and
//! stroked rects, lines, text (via `ab_glyph`), and blitting RGBA images
//! (the viewport's engine frames). The window shell uploads the pixmap to
//! the screen each frame.

use tiny_skia::{
    Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

use crate::text::TextRenderer;

/// A 2D canvas Oberon draws the editor into each frame.
pub struct Canvas {
    /// The pixel buffer, RGBA premultiplied, row-major.
    pub pixmap: Pixmap,
    text: TextRenderer,
}

impl Canvas {
    /// Creates a canvas of `width`×`height` physical pixels.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixmap: Pixmap::new(width.max(1), height.max(1)).expect("pixmap allocation"),
            text: TextRenderer::new(),
        }
    }

    /// The canvas size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.pixmap.width(), self.pixmap.height())
    }

    /// Resizes the pixel buffer (contents are not preserved).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.pixmap = Pixmap::new(width.max(1), height.max(1)).expect("pixmap allocation");
    }

    /// Clears the whole canvas to `color`.
    pub fn clear(&mut self, color: Color) {
        self.pixmap.fill(color);
    }

    /// Fills a rectangle (clamped to the canvas).
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color(color);
        self.pixmap
            .fill_rect(rect, &paint, Transform::identity(), None);
    }

    /// Strokes a rectangle outline.
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color, width: f32) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        let Some(path) = pb.finish() else { return };
        let mut paint = Paint::default();
        paint.set_color(color);
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    /// Fills a rounded rectangle.
    pub fn fill_rounded(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let Some(path) = rounded_rect_path(rect, radius) else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        self.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
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
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    /// Draws a straight line from `(x0, y0)` to `(x1, y1)`.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, width: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        let Some(path) = pb.finish() else { return };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    /// Draws text at `(x, y)` (top-left), returning the advance width.
    pub fn text(&mut self, x: f32, y: f32, size: f32, text: &str, color: Color) -> f32 {
        self.text.draw(&mut self.pixmap, x, y, size, text, color)
    }

    /// Measures text width at `size` without drawing.
    pub fn measure_text(&mut self, size: f32, text: &str) -> f32 {
        self.text.measure(size, text)
    }

    /// Blits an RGBA8 image (row-major, straight alpha) at `(x, y)`.
    pub fn blit_rgba(&mut self, x: f32, y: f32, width: u32, height: u32, rgba: &[u8]) {
        if rgba.len() < (width * height * 4) as usize {
            return;
        }
        // tiny-skia pixmaps are premultiplied; premultiply the source.
        let mut premul = vec![0u8; (width * height * 4) as usize];
        for (dst, src) in premul.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            let a = src[3] as u32;
            dst[0] = ((src[0] as u32 * src[3] as u32 + 127) / 255) as u8;
            dst[1] = ((src[1] as u32 * src[3] as u32 + 127) / 255) as u8;
            dst[2] = ((src[2] as u32 * src[3] as u32 + 127) / 255) as u8;
            dst[3] = src[3];
            let _ = a;
        }
        let Some(src) =
            Pixmap::from_vec(premul, tiny_skia::IntSize::from_wh(width, height).unwrap())
        else {
            return;
        };
        let paint = PixmapPaint::default();
        self.pixmap.draw_pixmap(
            x as i32,
            y as i32,
            src.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
    }

    /// The raw premultiplied RGBA pixels (for the window upload).
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
