//! The color picker popup: saturation/value square, hue bar, alpha bar, hex
//! entry, old/new preview, a preset palette, and recently used colors.
//!
//! It edits an entity's color channels through [`Intent::SetColor`]; drags
//! are gestures, so a whole drag is one undo step.

use super::{CursorHint, EditTarget, Intent, PlayState, Rect, TextEdit, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::fields::{self, Field};
use crate::theme::{color, layout};
use tiny_skia::Color;

/// Which color property the picker edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTarget {
    /// Mesh base color (RGBA).
    Mesh,
    /// Mesh emitted color (RGB).
    MeshEmissive,
    /// Light color (RGB).
    Light,
    /// Environment sky ambient (RGB).
    EnvSky,
    /// Environment ground ambient (RGB).
    EnvGround,
    /// Environment background (RGB).
    EnvBackground,
    /// Environment fog color (RGB).
    EnvFog,
    /// Environment sky color straight up (RGB).
    EnvZenith,
    /// Water deep color (RGB).
    WaterDeep,
    /// Water shallow color (RGB).
    WaterShallow,
    /// A custom `Color` attribute, by key index in sorted order.
    Attribute(usize),
}

impl ColorTarget {
    /// The scalar fields behind this color (empty for attributes).
    #[must_use]
    pub fn channels(self) -> Vec<Field> {
        match self {
            Self::Mesh => (0..4).map(Field::MeshColor).collect(),
            Self::MeshEmissive => (0..3).map(Field::MeshEmissive).collect(),
            Self::Light => (0..3).map(Field::LightColor).collect(),
            Self::EnvSky => (0..3).map(Field::EnvSky).collect(),
            Self::EnvGround => (0..3).map(Field::EnvGround).collect(),
            Self::EnvBackground => (0..3).map(Field::EnvBackground).collect(),
            Self::EnvFog => (0..3).map(Field::EnvFogColor).collect(),
            Self::EnvZenith => (0..3).map(Field::EnvZenith).collect(),
            Self::WaterDeep => (0..3).map(Field::WaterDeep).collect(),
            Self::WaterShallow => (0..3).map(Field::WaterShallow).collect(),
            Self::Attribute(_) => Vec::new(),
        }
    }

    /// Whether the color has an alpha channel.
    #[must_use]
    pub fn has_alpha(self) -> bool {
        matches!(self, Self::Mesh | Self::Attribute(_))
    }

    /// Reads the current RGBA (alpha 1 for RGB colors).
    #[must_use]
    pub fn read(self, data: &automata_core::EntityData) -> Option<[f32; 4]> {
        match self {
            Self::Attribute(i) => match data.attributes.values().nth(i)? {
                automata_core::AttributeValue::Color(c) => Some(*c),
                _ => None,
            },
            _ => {
                let ch = self.channels();
                let mut out = [1.0; 4];
                for (i, f) in ch.iter().enumerate() {
                    out[i] = fields::get(data, *f)?;
                }
                Some(out)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickDrag {
    None,
    Sv,
    Hue,
    Alpha,
}

/// An open color picker.
#[derive(Debug, Clone)]
pub struct ColorPicker {
    id: u64,
    target: ColorTarget,
    hsv: [f32; 3],
    alpha: f32,
    /// The color when the picker opened (shown as "before").
    original: [f32; 4],
    /// The last color written or seen, to notice outside changes (undo).
    last: [f32; 4],
    x: f32,
    y: f32,
    drag: PickDrag,
    /// Cached SV-square pixels: (hue, physical w, physical h, RGBA).
    sv_cache: Option<(f32, u32, u32, Vec<u8>)>,
}

/// Preset swatches shown under the picker.
const PALETTE: [[f32; 4]; 12] = [
    [1.0, 1.0, 1.0, 1.0],
    [0.62, 0.64, 0.70, 1.0],
    [0.16, 0.17, 0.20, 1.0],
    [0.0, 0.0, 0.0, 1.0],
    [0.88, 0.30, 0.30, 1.0],
    [1.0, 0.56, 0.24, 1.0],
    [1.0, 0.85, 0.25, 1.0],
    [0.40, 0.78, 0.38, 1.0],
    [0.30, 0.75, 0.80, 1.0],
    [0.38, 0.61, 1.0, 1.0],
    [0.58, 0.42, 0.90, 1.0],
    [0.95, 0.45, 0.70, 1.0],
];

const PW: f32 = 244.0;
const SV_H: f32 = 150.0;

/// RGB (0..1) to HSV (h 0..1).
#[must_use]
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if (max - r).abs() <= f32::EPSILON {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if (max - g).abs() <= f32::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    [h, s, max]
}

/// HSV (h 0..1) to RGB (0..1).
#[must_use]
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// `#RRGGBB` or `#RRGGBBAA` for a color.
#[must_use]
pub fn to_hex(c: [f32; 4], alpha: bool) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if alpha && b(c[3]) != 255 {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            b(c[0]),
            b(c[1]),
            b(c[2]),
            b(c[3])
        )
    } else {
        format!("#{:02X}{:02X}{:02X}", b(c[0]), b(c[1]), b(c[2]))
    }
}

/// Parses `#RGB`, `#RRGGBB`, or `#RRGGBBAA` (the `#` is optional).
#[must_use]
pub fn parse_hex(text: &str) -> Option<[f32; 4]> {
    let t = text.trim().trim_start_matches('#');
    let hex = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| f32::from(v) / 255.0);
    match t.len() {
        3 => {
            let mut out = [1.0; 4];
            for (i, ch) in t.chars().enumerate() {
                let s = format!("{ch}{ch}");
                out[i] = hex(&s)?;
            }
            Some(out)
        }
        6 | 8 => {
            let mut out = [1.0; 4];
            for (i, slot) in out.iter_mut().enumerate().take(t.len() / 2) {
                *slot = hex(t.get(i * 2..i * 2 + 2)?)?;
            }
            Some(out)
        }
        _ => None,
    }
}

fn to_color(c: [f32; 4]) -> Color {
    Color::from_rgba(
        c[0].clamp(0.0, 1.0),
        c[1].clamp(0.0, 1.0),
        c[2].clamp(0.0, 1.0),
        c[3].clamp(0.0, 1.0),
    )
    .unwrap_or(Color::WHITE)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| (x - y).abs() < 0.5 / 255.0)
}

impl ColorPicker {
    fn rgba(&self) -> [f32; 4] {
        let [r, g, b] = hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2]);
        [r, g, b, self.alpha]
    }

    fn rect(&self, w: f32, h: f32) -> Rect {
        let ph = if self.target.has_alpha() {
            318.0
        } else {
            296.0
        };
        let x = self.x.min(w - PW - 6.0).max(4.0);
        let y = if self.y + ph > h - 4.0 {
            (self.y - ph - 26.0).max(4.0)
        } else {
            self.y
        };
        Rect::new(x, y, PW, ph)
    }
}

/// Draws a checkerboard (shows transparency behind alpha colors).
fn checker(canvas: &mut Canvas, r: Rect) {
    canvas.fill_rect(r.x, r.y, r.w, r.h, Color::from_rgba8(200, 200, 205, 255));
    let s = 5.0;
    let mut yy = 0;
    let mut y = r.y;
    while y < r.y + r.h {
        let mut x = r.x + if yy % 2 == 0 { 0.0 } else { s };
        while x < r.x + r.w {
            let w = s.min(r.x + r.w - x);
            let h = s.min(r.y + r.h - y);
            canvas.fill_rect(x, y, w, h, Color::from_rgba8(140, 140, 148, 255));
            x += 2.0 * s;
        }
        y += s;
        yy += 1;
    }
}

impl Ui {
    /// Opens the color picker for `target` on entity `id`, under the pointer.
    pub(super) fn open_picker(&mut self, doc: &Document, id: u64, target: ColorTarget) {
        let Some(i) = doc.index_of(id) else {
            return;
        };
        let Some(c) = target.read(&doc.entities[i].data) else {
            return;
        };
        self.open_picker_rgba(id, target, c);
    }

    /// Opens the color picker for `target` (currently `c`) on entity `id`.
    pub(super) fn open_picker_rgba(&mut self, id: u64, target: ColorTarget, c: [f32; 4]) {
        if self.play != PlayState::Editing {
            self.log(super::LogLevel::Warn, "Colors are read-only while playing");
            return;
        }
        self.menu = None;
        self.picker = Some(ColorPicker {
            id,
            target,
            hsv: rgb_to_hsv(c[0], c[1], c[2]),
            alpha: c[3],
            original: c,
            last: c,
            x: self.raw.x - 20.0,
            y: self.raw.y + 14.0,
            drag: PickDrag::None,
            sv_cache: None,
        });
    }

    /// The open picker's rectangle (pointer capture), if any.
    pub(super) fn picker_rect(&self, w: f32, h: f32) -> Option<Rect> {
        self.picker.as_ref().map(|p| p.rect(w, h))
    }

    /// Whether the picker is mid-drag (it then owns the pointer).
    pub(super) fn picker_dragging(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|p| p.drag != PickDrag::None)
    }

    fn emit_color(&mut self, id: u64, target: ColorTarget, rgba: [f32; 4], gesture: bool) {
        self.out.push(Intent::SetColor {
            id,
            target,
            rgba,
            gesture,
        });
    }

    /// Remembers a color in the "recent" row.
    pub(super) fn remember_color(&mut self, c: [f32; 4]) {
        self.recent_colors.retain(|r| !close(*r, c));
        self.recent_colors.insert(0, c);
        self.recent_colors.truncate(12);
    }

    /// Draws the picker and handles its interaction.
    #[allow(clippy::too_many_lines)]
    pub(super) fn draw_picker(&mut self, canvas: &mut Canvas, doc: &Document, w: f32, h: f32) {
        let Some(mut p) = self.picker.take() else {
            return;
        };
        let Some(i) = doc.index_of(p.id) else {
            return;
        };
        let Some(current) = p.target.read(&doc.entities[i].data) else {
            return;
        };
        if !close(current, p.last) && p.drag == PickDrag::None {
            // Changed elsewhere (undo, typing in the inspector): follow it.
            p.hsv = rgb_to_hsv(current[0], current[1], current[2]);
            p.alpha = current[3];
            p.last = current;
        }
        let editable = self.play == PlayState::Editing;
        let r = p.rect(w, h);
        let raw = self.raw;
        canvas.fill_rounded(r.x + 3.0, r.y + 4.0, r.w, r.h, 8.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 8.0, color::raised());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 8.0, color::border(), 1.0);

        // Saturation/value square.
        let sv = Rect::new(r.x + 12.0, r.y + 12.0, r.w - 24.0, SV_H);
        let scale = canvas.scale();
        let (pw, ph) = (
            (sv.w * scale).round().max(1.0) as u32,
            (sv.h * scale).round().max(1.0) as u32,
        );
        let stale = p.sv_cache.as_ref().is_none_or(|(hue, cw, ch, _)| {
            (*hue - p.hsv[0]).abs() > 1e-4 || *cw != pw || *ch != ph
        });
        if stale {
            let mut buf = vec![0u8; (pw * ph * 4) as usize];
            for yy in 0..ph {
                let v = 1.0 - yy as f32 / (ph - 1).max(1) as f32;
                for xx in 0..pw {
                    let s = xx as f32 / (pw - 1).max(1) as f32;
                    let [cr, cg, cb] = hsv_to_rgb(p.hsv[0], s, v);
                    let idx = ((yy * pw + xx) * 4) as usize;
                    buf[idx] = (cr * 255.0) as u8;
                    buf[idx + 1] = (cg * 255.0) as u8;
                    buf[idx + 2] = (cb * 255.0) as u8;
                    buf[idx + 3] = 255;
                }
            }
            p.sv_cache = Some((p.hsv[0], pw, ph, buf));
        }
        if let Some((_, cw, ch, buf)) = &p.sv_cache {
            canvas.blit_rgba(sv.x, sv.y, *cw, *ch, buf);
        }
        canvas.stroke_rect(sv.x, sv.y, sv.w, sv.h, color::border(), 1.0);
        let (mx, my) = (sv.x + p.hsv[1] * sv.w, sv.y + (1.0 - p.hsv[2]) * sv.h);
        canvas.stroke_circle(mx, my, 6.0, Color::BLACK, 2.5);
        canvas.stroke_circle(mx, my, 6.0, Color::WHITE, 1.5);

        // Hue bar.
        let hue = Rect::new(sv.x, sv.y + sv.h + 10.0, sv.w, 14.0);
        let steps = 48;
        for k in 0..steps {
            let t0 = k as f32 / steps as f32;
            let [cr, cg, cb] = hsv_to_rgb(t0, 1.0, 1.0);
            let x0 = hue.x + t0 * hue.w;
            canvas.fill_rect(
                x0,
                hue.y,
                hue.w / steps as f32 + 0.6,
                hue.h,
                to_color([cr, cg, cb, 1.0]),
            );
        }
        canvas.stroke_rect(hue.x, hue.y, hue.w, hue.h, color::border(), 1.0);
        let hx = hue.x + p.hsv[0] * hue.w;
        canvas.fill_rect(hx - 2.0, hue.y - 2.0, 4.0, hue.h + 4.0, Color::WHITE);
        canvas.stroke_rect(hx - 2.0, hue.y - 2.0, 4.0, hue.h + 4.0, Color::BLACK, 1.0);

        // Alpha bar.
        let mut y = hue.y + hue.h + 8.0;
        let alpha_bar = Rect::new(sv.x, y, sv.w, 14.0);
        if p.target.has_alpha() {
            checker(canvas, alpha_bar);
            let base = p.rgba();
            for k in 0..32 {
                let t = k as f32 / 32.0;
                canvas.fill_rect(
                    alpha_bar.x + t * alpha_bar.w,
                    alpha_bar.y,
                    alpha_bar.w / 32.0 + 0.6,
                    alpha_bar.h,
                    to_color([base[0], base[1], base[2], t]),
                );
            }
            canvas.stroke_rect(
                alpha_bar.x,
                alpha_bar.y,
                alpha_bar.w,
                alpha_bar.h,
                color::border(),
                1.0,
            );
            let ax = alpha_bar.x + p.alpha * alpha_bar.w;
            canvas.fill_rect(
                ax - 2.0,
                alpha_bar.y - 2.0,
                4.0,
                alpha_bar.h + 4.0,
                Color::WHITE,
            );
            canvas.stroke_rect(
                ax - 2.0,
                alpha_bar.y - 2.0,
                4.0,
                alpha_bar.h + 4.0,
                Color::BLACK,
                1.0,
            );
            y += 22.0;
        }

        // Before / after preview + hex field + numeric readout.
        let prev = Rect::new(sv.x, y, 54.0, 24.0);
        checker(canvas, prev);
        canvas.fill_rect(prev.x, prev.y, prev.w / 2.0, prev.h, to_color(p.original));
        canvas.fill_rect(
            prev.x + prev.w / 2.0,
            prev.y,
            prev.w / 2.0,
            prev.h,
            to_color(p.rgba()),
        );
        canvas.stroke_rect(prev.x, prev.y, prev.w, prev.h, color::border(), 1.0);
        let revert = prev;
        let hex_r = Rect::new(prev.x + prev.w + 8.0, y + 2.0, 92.0, layout::FIELD_H);
        let editing_hex = self.is_editing(p.id, EditTarget::Hex(p.target));
        canvas.fill_rounded(hex_r.x, hex_r.y, hex_r.w, hex_r.h, 3.0, color::field());
        if editing_hex {
            canvas.stroke_rounded(
                hex_r.x,
                hex_r.y,
                hex_r.w,
                hex_r.h,
                3.0,
                color::accent(),
                1.0,
            );
        }
        if editing_hex {
            self.draw_edit_text(canvas, hex_r, hex_r.x + 5.0);
        } else {
            canvas.text(
                hex_r.x + 6.0,
                hex_r.y + 3.0,
                layout::FONT_SIZE,
                &to_hex(p.rgba(), p.target.has_alpha()),
                color::text(),
            );
        }
        let c = p.rgba();
        let readout = format!(
            "{} {} {}",
            (c[0] * 255.0).round(),
            (c[1] * 255.0).round(),
            (c[2] * 255.0).round()
        );
        canvas.text_clipped(
            hex_r.x + hex_r.w + 8.0,
            y + 6.0,
            layout::SMALL_SIZE,
            &readout,
            color::text_dim(),
            r.x + r.w - (hex_r.x + hex_r.w + 8.0) - 8.0,
        );
        y += 32.0;

        // Palette + recent swatches.
        let sw = (sv.w - 11.0 * 4.0) / 12.0;
        let mut picked = None;
        let recent = self.recent_colors.clone();
        for (row, colors) in [(0, PALETTE.to_vec()), (1, recent)] {
            if row == 1 {
                canvas.text(sv.x, y, layout::SMALL_SIZE, "Recent", color::text_dim());
                y += 15.0;
            }
            for (k, c) in colors.iter().enumerate().take(12) {
                let s = Rect::new(sv.x + k as f32 * (sw + 4.0), y, sw, sw);
                checker(canvas, s);
                canvas.fill_rect(s.x, s.y, s.w, s.h, to_color(*c));
                let hot = s.contains(raw.x, raw.y);
                canvas.stroke_rect(
                    s.x,
                    s.y,
                    s.w,
                    s.h,
                    if hot {
                        color::accent()
                    } else {
                        color::border()
                    },
                    1.0,
                );
                if hot {
                    self.cursor = CursorHint::Pointer;
                    if raw.pressed {
                        picked = Some(*c);
                    }
                }
            }
            y += sw + 8.0;
        }

        // Interaction.
        if editable {
            if raw.pressed && p.drag == PickDrag::None {
                if sv.contains(raw.x, raw.y) {
                    p.drag = PickDrag::Sv;
                } else if Rect::new(hue.x, hue.y - 3.0, hue.w, hue.h + 6.0).contains(raw.x, raw.y) {
                    p.drag = PickDrag::Hue;
                } else if p.target.has_alpha()
                    && Rect::new(
                        alpha_bar.x,
                        alpha_bar.y - 3.0,
                        alpha_bar.w,
                        alpha_bar.h + 6.0,
                    )
                    .contains(raw.x, raw.y)
                {
                    p.drag = PickDrag::Alpha;
                } else if hex_r.contains(raw.x, raw.y) {
                    self.edit = Some(TextEdit {
                        id: p.id,
                        target: EditTarget::Hex(p.target),
                        buffer: to_hex(p.rgba(), p.target.has_alpha()),
                        rect: hex_r,
                        fresh: true,
                        attr: None,
                    });
                } else if revert.contains(raw.x, raw.y) && raw.x < revert.x + revert.w / 2.0 {
                    picked = Some(p.original);
                }
            }
            if p.drag != PickDrag::None {
                self.cursor = CursorHint::Grabbing;
                if raw.down {
                    match p.drag {
                        PickDrag::Sv => {
                            p.hsv[1] = ((raw.x - sv.x) / sv.w).clamp(0.0, 1.0);
                            p.hsv[2] = (1.0 - (raw.y - sv.y) / sv.h).clamp(0.0, 1.0);
                        }
                        PickDrag::Hue => p.hsv[0] = ((raw.x - hue.x) / hue.w).clamp(0.0, 0.9999),
                        PickDrag::Alpha => {
                            p.alpha = ((raw.x - alpha_bar.x) / alpha_bar.w).clamp(0.0, 1.0);
                        }
                        PickDrag::None => {}
                    }
                    let c = p.rgba();
                    if !close(c, p.last) {
                        p.last = c;
                        self.emit_color(p.id, p.target, c, true);
                    }
                } else {
                    p.drag = PickDrag::None;
                    self.out.push(Intent::EndGesture);
                    self.remember_color(p.rgba());
                }
            }
            if let Some(c) = picked {
                p.hsv = rgb_to_hsv(c[0], c[1], c[2]);
                if p.target.has_alpha() {
                    p.alpha = c[3];
                }
                let c = p.rgba();
                p.last = c;
                self.emit_color(p.id, p.target, c, false);
                self.remember_color(c);
            }
        } else if raw.pressed && r.contains(raw.x, raw.y) {
            self.log(super::LogLevel::Warn, "Colors are read-only while playing");
        }
        self.picker = Some(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for c in [
            [1.0, 0.0, 0.0],
            [0.2, 0.6, 0.9],
            [0.5, 0.5, 0.5],
            [0.0, 0.0, 0.0],
            [1.0, 0.56, 0.24],
        ] {
            let [h, s, v] = rgb_to_hsv(c[0], c[1], c[2]);
            let back = hsv_to_rgb(h, s, v);
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-4, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn hex_parses_and_formats() {
        assert_eq!(to_hex([1.0, 0.5333, 0.0, 1.0], true), "#FF8800");
        assert_eq!(to_hex([1.0, 1.0, 1.0, 0.5], true), "#FFFFFF80");
        assert_eq!(
            parse_hex("#f80").map(|c| to_hex(c, false)),
            Some("#FF8800".into())
        );
        assert_eq!(
            parse_hex("ff880080").map(|c| (c[3] * 255.0).round()),
            Some(128.0)
        );
        assert!(parse_hex("#12345").is_none());
        assert!(parse_hex("zzzzzz").is_none());
    }
}
