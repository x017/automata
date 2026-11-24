//! The Inspector's prefab block (for an entity linked to a `.prefab`
//! file): where it comes from, what this instance overrides, and Apply /
//! Revert / Unlink / Select Instances. Also the multi-selection note.

use automata_core::prefab;

use crate::canvas::Canvas;
use crate::doc::{Document, EntityEntry};

use super::{color, layout, ButtonStyle, Intent, PlayState, Rect, Ui};

impl Ui {
    /// "N selected" under the Inspector's header while several entities
    /// are selected: the values shown are the lead's, and edits go to all.
    pub(super) fn multi_select_note(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let n = doc.selection_len();
        if n < 2 {
            return y;
        }
        let r = Rect::new(x, y, w, 22.0);
        if self.row_visible(r) {
            let mut bg = color::accent();
            bg.apply_opacity(0.14);
            canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, bg);
            canvas.text_clipped(
                r.x + 8.0,
                r.y + 4.0,
                layout::SMALL_SIZE,
                &format!("{n} selected · showing the last picked · edits apply to all"),
                color::text(),
                r.w - 16.0,
            );
            self.tip(
                r,
                "Ctrl+click adds or removes, Shift+click selects a range, Shift+drag in the Scene view boxes",
            );
        }
        y + 28.0
    }

    /// The prefab block for a linked entity. Returns the y after it.
    pub(super) fn prefab_block(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        e: &EntityEntry,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let Some(link) = e.data.prefab.as_ref() else {
            return y;
        };
        let id = e.id;
        let editing = self.play == PlayState::Editing;
        let mut y = y;
        let overrides = doc.prefab_overrides(id);
        let card_top = y;

        // Source line.
        let head = Rect::new(x, y, w, 22.0);
        if self.row_visible(head) {
            canvas.text(
                x + 8.0,
                y + 4.0,
                layout::SMALL_SIZE,
                "Prefab",
                color::text_dim(),
            );
            let path = if link.index > 0 {
                format!("{} #{}", link.path, link.index)
            } else {
                link.path.clone()
            };
            let others = doc.instances_of(&link.path).len().saturating_sub(1);
            let tail = match others {
                0 => String::new(),
                1 => "  · 1 other".to_string(),
                n => format!("  · {n} others"),
            };
            canvas.text_clipped(
                x + 58.0,
                y + 4.0,
                layout::SMALL_SIZE,
                &format!("{path}{tail}"),
                color::axis_z(),
                w - 66.0,
            );
            self.tip(
                Rect::new(x + 58.0, y, w - 58.0, 22.0),
                "Changes to the prefab file reach every instance, except the fields an instance overrides",
            );
        }
        y += 24.0;

        // Actions.
        let row = Rect::new(x + 6.0, y, w - 12.0, 22.0);
        if self.row_visible(row) {
            let has_overrides = overrides.as_ref().is_some_and(|o| !o.is_empty());
            let gap = 4.0;
            let bw = (row.w - 3.0 * gap) / 4.0;
            let style = |on: bool| {
                if on && editing {
                    ButtonStyle::Ghost
                } else {
                    ButtonStyle::Disabled
                }
            };
            let buttons = [
                (
                    "Apply",
                    "Write this instance's overrides into the prefab (every instance gets them)",
                    style(has_overrides),
                    Intent::ApplyPrefab(id),
                ),
                (
                    "Revert",
                    "Drop every override: back to the prefab's values",
                    style(has_overrides),
                    Intent::RevertPrefab { id, paths: None },
                ),
                (
                    "Unlink",
                    "Keep the values but stop following the prefab",
                    style(true),
                    Intent::UnlinkPrefab(id),
                ),
                (
                    "Select All",
                    "Select every instance of this prefab in the scene",
                    ButtonStyle::Ghost,
                    Intent::SelectInstances(id),
                ),
            ];
            let mut bx = row.x;
            for (label, tip, style, intent) in buttons {
                let r = Rect::new(bx, row.y, bw, row.h);
                self.tip(r, tip);
                if self.button(canvas, r, label, style) && style != ButtonStyle::Disabled {
                    self.out.push(intent);
                }
                bx += bw + gap;
            }
        }
        y += 28.0;

        // Overrides.
        match overrides {
            None => {
                if self.row_visible(Rect::new(x, y, w, 16.0)) {
                    canvas.text_clipped(
                        x + 8.0,
                        y,
                        layout::SMALL_SIZE,
                        "The prefab file can't be read (see the Console)",
                        color::warn(),
                        w - 16.0,
                    );
                }
                y += 20.0;
            }
            Some(list) if list.is_empty() => {
                if self.row_visible(Rect::new(x, y, w, 16.0)) {
                    canvas.text(
                        x + 8.0,
                        y,
                        layout::SMALL_SIZE,
                        "No overrides: matches the prefab",
                        color::text_faint(),
                    );
                }
                y += 20.0;
            }
            Some(list) => {
                if self.row_visible(Rect::new(x, y, w, 16.0)) {
                    let n = list.len();
                    canvas.text(
                        x + 8.0,
                        y,
                        layout::SMALL_SIZE,
                        &format!("{n} override{}", if n == 1 { "" } else { "s" }),
                        color::text_dim(),
                    );
                }
                y += 18.0;
                for path in list {
                    let r = Rect::new(x + 6.0, y, w - 12.0, 20.0);
                    if self.row_visible(r) {
                        if self.hot(r) {
                            canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::hover());
                        }
                        canvas.fill_rect(r.x + 4.0, r.y + 4.0, 2.0, r.h - 8.0, color::axis_z());
                        canvas.text_clipped(
                            r.x + 12.0,
                            r.y + 3.0,
                            layout::SMALL_SIZE,
                            &prefab::describe(&path),
                            color::text(),
                            r.w - 76.0,
                        );
                        let revert = Rect::new(r.x + r.w - 58.0, r.y + 1.0, 56.0, r.h - 2.0);
                        let style = if editing {
                            ButtonStyle::Ghost
                        } else {
                            ButtonStyle::Disabled
                        };
                        self.tip(revert, "Back to the prefab's value for this field");
                        if self.button(canvas, revert, "Revert", style) && editing {
                            self.out.push(Intent::RevertPrefab {
                                id,
                                paths: Some(vec![path.clone()]),
                            });
                        }
                    }
                    y += 22.0;
                }
            }
        }
        // A frame around the block.
        let card = Rect::new(x, card_top, w, y - card_top + 4.0);
        canvas.stroke_rect(card.x, card.y, card.w, card.h, color::border(), 1.0);
        y + 12.0
    }
}
