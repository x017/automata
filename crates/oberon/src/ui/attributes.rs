//! Inspector sections for custom attributes and the attached script.
//!
//! Attributes are free-form named values (`health: 100`, `team: "red"`)
//! saved in the scene and readable from Python as `entity.attributes[...]`.

use automata_core::AttributeValue;

use super::menus::{Action, DialogKind, MenuItem, UiAction};
use super::{
    eval_number, ButtonStyle, ColorTarget, CursorHint, EditTarget, Icon, Intent, PlayState, Rect,
    TextEdit, Ui,
};
use crate::canvas::Canvas;
use crate::doc::{Document, EntityEntry};
use crate::fields;
use crate::theme::{color, layout};

/// The attribute types offered by "+ Attribute", in menu order.
pub const TYPES: [&str; 6] = ["Float", "Int", "Bool", "Text", "Vec3", "Color"];

/// A fresh value of the type at `index` in [`TYPES`].
#[must_use]
pub fn default_value(index: usize) -> AttributeValue {
    match index {
        1 => AttributeValue::Int(0),
        2 => AttributeValue::Bool(false),
        3 => AttributeValue::Text(String::new()),
        4 => AttributeValue::Vec3([0.0; 3]),
        5 => AttributeValue::Color([1.0, 1.0, 1.0, 1.0]),
        _ => AttributeValue::Float(0.0),
    }
}

/// Parses typed text into a new value of `old`'s type (vector `component`
/// for Vec3). Numbers accept the inspector's math expressions.
#[must_use]
pub fn parse_value(old: &AttributeValue, component: u8, text: &str) -> Option<AttributeValue> {
    Some(match old {
        AttributeValue::Int(_) => AttributeValue::Int(match text.trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => f64::from(eval_number(text)?).round() as i64,
        }),
        AttributeValue::Float(_) => AttributeValue::Float(match text.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => v,
            _ => f64::from(eval_number(text)?),
        }),
        AttributeValue::Text(_) => AttributeValue::Text(text.to_string()),
        AttributeValue::Vec3(v) => {
            let mut v = *v;
            v[usize::from(component.min(2))] = eval_number(text)?;
            AttributeValue::Vec3(v)
        }
        AttributeValue::Bool(_) | AttributeValue::Color(_) => return None,
    })
}

/// The display text of a scalar attribute value.
fn display(value: &AttributeValue, component: u8) -> String {
    match value {
        AttributeValue::Int(v) => v.to_string(),
        AttributeValue::Float(v) => fields::format_value(*v as f32),
        AttributeValue::Text(t) => t.clone(),
        AttributeValue::Vec3(v) => fields::format_value(v[usize::from(component.min(2))]),
        AttributeValue::Bool(b) => b.to_string(),
        AttributeValue::Color(_) => String::new(),
    }
}

/// A name not yet used by `existing`: `attribute`, `attribute_2`, …
#[must_use]
pub fn unique_name<'a, I>(base: &str, existing: &I) -> String
where
    I: Iterator<Item = &'a String> + Clone,
{
    let taken = |n: &str| existing.clone().any(|k| k == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..1000)
        .map(|i| format!("{base}_{i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

/// An attribute number being dragged (scrubbed).
#[derive(Debug, Clone)]
pub struct AttrScrub {
    id: u64,
    key: String,
    value: AttributeValue,
    component: u8,
    start_x: f32,
    moved: bool,
    rect: Rect,
    index: usize,
}

impl Ui {
    /// Continues (or finishes) an attribute scrub.
    pub(super) fn update_attr_scrub(&mut self) {
        let Some(mut s) = self.attr_scrub.take() else {
            return;
        };
        let p = self.raw;
        if p.down {
            let dx = p.x - s.start_x;
            s.moved = s.moved || dx.abs() > 3.0;
            if s.moved {
                let mut step = 0.02f64;
                if p.shift {
                    step *= 0.1;
                }
                if p.ctrl {
                    step *= 10.0;
                }
                let delta = f64::from(dx) * step;
                let value = match &s.value {
                    AttributeValue::Int(v) => {
                        AttributeValue::Int(v + (f64::from(dx) * step * 10.0).round() as i64)
                    }
                    AttributeValue::Float(v) => AttributeValue::Float(v + delta),
                    AttributeValue::Vec3(v) => {
                        let mut v = *v;
                        v[usize::from(s.component)] += delta as f32;
                        AttributeValue::Vec3(v)
                    }
                    other => other.clone(),
                };
                self.out.push(Intent::SetAttribute {
                    id: s.id,
                    key: s.key.clone(),
                    value,
                    gesture: true,
                });
                self.cursor = CursorHint::ResizeH;
            }
            self.attr_scrub = Some(s);
        } else if s.moved {
            self.out.push(Intent::EndGesture);
        } else {
            // A click: type a value instead.
            self.edit = Some(TextEdit {
                id: s.id,
                target: EditTarget::AttrValue(s.index, s.component),
                buffer: display(&s.value, s.component),
                rect: s.rect,
                fresh: true,
                attr: Some((s.key, s.value)),
            });
        }
    }

    /// The attached-script section. Returns the y after it.
    pub(super) fn script_section(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        e: &EntityEntry,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let Some(script) = &e.data.script else {
            return y;
        };
        let (mut y, open) = self.section(canvas, x, y, w, "Script", e.id, None, false);
        if !open {
            return y;
        }
        let editing = self.play == PlayState::Editing;
        let path = doc.resolve(&script.path);
        let exists = path.is_file();
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "File");
            let r = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
            canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::field());
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::border(), 1.0);
            canvas.text_clipped(
                r.x + 6.0,
                r.y + 3.0,
                layout::FONT_SIZE,
                &script.path,
                if exists {
                    color::accent_soft()
                } else {
                    color::danger()
                },
                r.w - 12.0,
            );
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
                if self.p.double_click {
                    self.out.push(Intent::OpenScript(path.clone()));
                }
            }
        }
        y += layout::ROW_STEP;
        if !exists && self.row_visible(Rect::new(x, y, w, 16.0)) {
            canvas.text(
                x,
                y,
                layout::SMALL_SIZE,
                "File not found (Edit creates it)",
                color::danger(),
            );
            y += 18.0;
        }
        let bw = (w - layout::LABEL_W - 6.0) / 2.0;
        let edit_r = Rect::new(x + layout::LABEL_W, y, bw, 22.0);
        if self.row_visible(edit_r) {
            if self.button(canvas, edit_r, "Edit Script", ButtonStyle::Accent) {
                self.out.push(if exists {
                    Intent::OpenScript(path)
                } else {
                    Intent::NewScript(path)
                });
            }
            let detach = Rect::new(edit_r.x + bw + 6.0, y, bw, 22.0);
            let style = if editing {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, detach, "Detach", style) {
                self.out.push(Intent::DetachScript(e.id));
            }
        }
        y += 28.0;
        for line in [
            "update(entity, ctx) runs every frame in Play;",
            "start(entity, ctx) once.",
        ] {
            if self.row_visible(Rect::new(x, y, w, 16.0)) {
                canvas.text(x, y, layout::SMALL_SIZE, line, color::text_dim());
            }
            y += 16.0;
        }
        y + 8.0
    }

    /// The custom attributes section. Returns the y after it.
    #[allow(clippy::too_many_lines)]
    pub(super) fn attributes_section(
        &mut self,
        canvas: &mut Canvas,
        e: &EntityEntry,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let (mut y, open) = self.section(canvas, x, y, w, "Attributes", e.id, None, false);
        if !open {
            return y;
        }
        let editing = self.play == PlayState::Editing;
        let id = e.id;
        let key_w = layout::LABEL_W - 6.0;
        for (index, (key, value)) in e.data.attributes.iter().enumerate() {
            let row = Rect::new(x, y, w, layout::FIELD_H);
            if self.row_visible(row) {
                // Name (click to rename).
                let kr = Rect::new(x, y, key_w, layout::FIELD_H);
                let editing_key = self.is_editing(id, EditTarget::AttrKey(index));
                canvas.fill_rounded(
                    kr.x,
                    kr.y,
                    kr.w,
                    kr.h,
                    3.0,
                    if editing_key {
                        color::viewport_bg()
                    } else {
                        color::surface()
                    },
                );
                if editing_key {
                    canvas.stroke_rounded(kr.x, kr.y, kr.w, kr.h, 3.0, color::accent(), 1.0);
                    self.draw_edit_text(canvas, kr, kr.x + 4.0);
                } else {
                    canvas.text_clipped(
                        kr.x + 4.0,
                        kr.y + 3.0,
                        layout::FONT_SIZE,
                        key,
                        color::text_dim(),
                        kr.w - 8.0,
                    );
                    if self.hot(kr) && editing {
                        self.cursor = CursorHint::Text;
                        if self.p.pressed {
                            self.edit = Some(TextEdit {
                                id,
                                target: EditTarget::AttrKey(index),
                                buffer: key.clone(),
                                rect: kr,
                                fresh: false,
                                attr: Some((key.clone(), value.clone())),
                            });
                        }
                    }
                }
                // Value editor.
                let vx = x + layout::LABEL_W;
                let vw = w - layout::LABEL_W - 26.0;
                match value {
                    AttributeValue::Bool(on) => {
                        let b = Rect::new(vx, y + 1.0, 18.0, 18.0);
                        canvas.fill_rounded(
                            b.x,
                            b.y,
                            b.w,
                            b.h,
                            3.0,
                            if *on { color::accent() } else { color::field() },
                        );
                        canvas.stroke_rounded(b.x, b.y, b.w, b.h, 3.0, color::border(), 1.0);
                        if *on {
                            canvas.polyline(
                                &[
                                    (b.x + 4.0, b.y + 9.5),
                                    (b.x + 7.5, b.y + 13.0),
                                    (b.x + 14.0, b.y + 5.0),
                                ],
                                color::on_accent(),
                                2.0,
                            );
                        }
                        if self.hot(b) && editing {
                            self.cursor = CursorHint::Pointer;
                            if self.p.pressed {
                                self.out.push(Intent::SetAttribute {
                                    id,
                                    key: key.clone(),
                                    value: AttributeValue::Bool(!on),
                                    gesture: false,
                                });
                            }
                        }
                    }
                    AttributeValue::Color(c) => {
                        let sw = Rect::new(vx, y, vw, layout::FIELD_H);
                        self.color_swatch(canvas, sw, id, ColorTarget::Attribute(index), *c);
                    }
                    AttributeValue::Vec3(_) => {
                        let gap = 4.0;
                        let fw = (vw - 2.0 * gap) / 3.0;
                        let tags = [color::axis_x(), color::axis_y(), color::axis_z()];
                        for (c, tag) in tags.into_iter().enumerate() {
                            let r = Rect::new(vx + c as f32 * (fw + gap), y, fw, layout::FIELD_H);
                            self.attr_field(canvas, r, id, index, key, value, c as u8, Some(tag));
                        }
                    }
                    _ => {
                        let r = Rect::new(vx, y, vw, layout::FIELD_H);
                        self.attr_field(canvas, r, id, index, key, value, 0, None);
                    }
                }
                // Remove.
                let xr = Rect::new(x + w - 20.0, y + 1.0, 20.0, 18.0);
                let style = if editing {
                    ButtonStyle::Normal
                } else {
                    ButtonStyle::Disabled
                };
                if self.icon_button(canvas, xr, Icon::Close, style) {
                    self.out.push(Intent::RemoveAttribute {
                        id,
                        key: key.clone(),
                    });
                }
            }
            y += layout::ROW_STEP;
        }
        if e.data.attributes.is_empty() && self.row_visible(Rect::new(x, y, w, 16.0)) {
            canvas.text(
                x,
                y,
                layout::SMALL_SIZE,
                "Custom values your scripts can read:",
                color::text_dim(),
            );
            y += 16.0;
            canvas.text(
                x,
                y,
                layout::SMALL_SIZE,
                "entity.attributes[\"health\"]",
                color::accent_soft(),
            );
            y += 20.0;
        }
        let add = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, 22.0);
        if self.row_visible(add) {
            let style = if editing {
                ButtonStyle::Ghost
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, add, "+ Add attribute", style) {
                let items = TYPES
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let value = default_value(i);
                        let base = t.to_lowercase();
                        let key = unique_name(&base, &e.data.attributes.keys());
                        MenuItem {
                            label: (*t).to_string(),
                            shortcut: "",
                            action: Some(Action::Intent(Intent::SetAttribute {
                                id,
                                key,
                                value,
                                gesture: false,
                            })),
                            enabled: true,
                        }
                    })
                    .collect();
                self.open_menu(add.x, add.y + add.h + 2.0, items);
            }
        }
        y + 32.0
    }

    /// A number / text field for one attribute value (component `c`).
    #[allow(clippy::too_many_arguments)]
    fn attr_field(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        id: u64,
        index: usize,
        key: &str,
        value: &AttributeValue,
        c: u8,
        tag: Option<tiny_skia::Color>,
    ) {
        let target = EditTarget::AttrValue(index, c);
        let editing_now = self.is_editing(id, target);
        let hot = self.hot(r);
        canvas.fill_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            3.0,
            if editing_now {
                color::viewport_bg()
            } else if hot {
                color::hover()
            } else {
                color::field()
            },
        );
        if editing_now {
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.0);
        }
        let mut tx = r.x + 5.0;
        if let Some(t) = tag {
            tx = super::draw_axis_tag(canvas, r, t);
        }
        if editing_now {
            self.draw_edit_text(canvas, r, tx);
            self.cursor = CursorHint::Text;
            return;
        }
        let text = display(value, c);
        let is_text = matches!(value, AttributeValue::Text(_));
        canvas.text_clipped(
            tx,
            r.y + 3.0,
            layout::FONT_SIZE,
            if is_text && text.is_empty() {
                "(empty)"
            } else {
                &text
            },
            if is_text && text.is_empty() {
                color::text_dim()
            } else {
                color::text()
            },
            r.x + r.w - tx - 3.0,
        );
        if self.play != PlayState::Editing || !hot {
            return;
        }
        if is_text {
            self.cursor = CursorHint::Text;
            if self.p.pressed {
                self.edit = Some(TextEdit {
                    id,
                    target,
                    buffer: text,
                    rect: r,
                    fresh: false,
                    attr: Some((key.to_string(), value.clone())),
                });
            }
        } else {
            self.cursor = CursorHint::ResizeH;
            if self.p.pressed && self.attr_scrub.is_none() {
                self.attr_scrub = Some(AttrScrub {
                    id,
                    key: key.to_string(),
                    value: value.clone(),
                    component: c,
                    start_x: self.p.x,
                    moved: false,
                    rect: r,
                    index,
                });
            }
        }
    }

    /// The "Script…" entry of the Add Component menu.
    pub(super) fn script_menu_item(&self, id: u64, has_script: bool) -> MenuItem {
        MenuItem {
            label: if has_script {
                "Script ✓".into()
            } else {
                "Script…".into()
            },
            shortcut: "",
            action: Some(Action::Ui(UiAction::Dialog(DialogKind::AttachScript(id)))),
            enabled: self.play == PlayState::Editing && !has_script,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_parses_into_the_attribute_type() {
        assert_eq!(
            parse_value(&AttributeValue::Int(1), 0, "42"),
            Some(AttributeValue::Int(42))
        );
        assert_eq!(
            parse_value(&AttributeValue::Int(1), 0, "10/4"),
            Some(AttributeValue::Int(3))
        );
        assert_eq!(
            parse_value(&AttributeValue::Float(0.0), 0, "2*3"),
            Some(AttributeValue::Float(6.0))
        );
        assert_eq!(
            parse_value(&AttributeValue::Vec3([1.0, 2.0, 3.0]), 1, "5"),
            Some(AttributeValue::Vec3([1.0, 5.0, 3.0]))
        );
        assert_eq!(
            parse_value(&AttributeValue::Text(String::new()), 0, "red team"),
            Some(AttributeValue::Text("red team".into()))
        );
        assert_eq!(parse_value(&AttributeValue::Float(0.0), 0, "abc"), None);
    }

    #[test]
    fn unique_names_skip_taken_ones() {
        let keys = ["float".to_string(), "float_2".to_string()];
        assert_eq!(unique_name("float", &keys.iter()), "float_3");
        assert_eq!(unique_name("int", &keys.iter()), "int");
    }
}
