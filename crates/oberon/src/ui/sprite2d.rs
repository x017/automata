//! The Inspector's 2D sections: Sprite (with its animation clips),
//! Tilemap (with the tile palette that paints in the scene view),
//! Camera 2D, Follow Camera 2D and Collider 2D.
//!
//! Tiles are painted with the brush ([`super::brush`]): picking a tile in
//! the palette switches it on with [`BrushTool::Tile`], and a left drag in
//! the scene view paints the cells under the cursor (one undo step a
//! stroke).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{ButtonStyle, CursorHint, EditTarget, Intent, PlayState, Rect, TextEdit, Ui};
use crate::canvas::Canvas;
use crate::doc::{BrushTool, ComponentKind, Document, EntityEntry};
use crate::fields::{Choice, Field, Flag};
use crate::fields2d::{Edit2D, Field2D, Flag2D};
use crate::theme::{color, layout};

/// Tile brush sizes offered (cells a side).
pub const TILE_SIZES: [f32; 3] = [1.0, 2.0, 4.0];

/// A text field of a 2D component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text2D {
    /// A sprite clip's name (clip index).
    ClipName(u8),
}

/// `(file, frame, columns, rows, side in physical pixels)`.
type ThumbKey = (PathBuf, u32, u32, u32, u32);

/// Small pictures of sprite frames and tiles for the Inspector, cut from
/// their images once.
#[derive(Debug, Default)]
pub struct Thumbs2D {
    /// RGBA by [`ThumbKey`], or `None` when the image didn't load.
    map: HashMap<ThumbKey, Option<Vec<u8>>>,
}

impl Thumbs2D {
    /// Frame `frame` of the `columns × rows` sheet `file`, scaled to a
    /// `side`-pixel square (nearest pixel, centred, transparent around).
    fn get(
        &mut self,
        file: &Path,
        frame: u32,
        columns: u32,
        rows: u32,
        side: u32,
    ) -> Option<&[u8]> {
        let key = (file.to_path_buf(), frame, columns, rows, side);
        if self.map.len() > 4096 {
            self.map.clear();
        }
        self.map
            .entry(key)
            .or_insert_with(|| {
                let img = automata_assets::load_image_file(&file.to_string_lossy()).ok()?;
                Some(cut_frame(
                    &img.rgba, img.width, img.height, frame, columns, rows, side,
                ))
            })
            .as_deref()
    }

    /// Forgets every picture (images changed on disk).
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// Cuts frame `frame` out of a `columns × rows` sheet and fits it in a
/// `side`-pixel square.
#[must_use]
pub fn cut_frame(
    rgba: &[u8],
    width: u32,
    height: u32,
    frame: u32,
    columns: u32,
    rows: u32,
    side: u32,
) -> Vec<u8> {
    let side = side.max(1);
    let mut out = vec![0u8; (side * side * 4) as usize];
    let (columns, rows) = (columns.max(1), rows.max(1));
    let frame = frame % (columns * rows);
    let (fw, fh) = ((width / columns).max(1), (height / rows).max(1));
    let (x0, y0) = ((frame % columns) * fw, (frame / columns) * fh);
    let scale = side as f32 / fw.max(fh) as f32;
    let (dw, dh) = ((fw as f32 * scale) as u32, (fh as f32 * scale) as u32);
    let (ox, oy) = ((side - dw.min(side)) / 2, (side - dh.min(side)) / 2);
    for y in 0..dh.min(side) {
        for x in 0..dw.min(side) {
            let sx = x0 + ((x as f32 / scale) as u32).min(fw - 1);
            let sy = y0 + ((y as f32 / scale) as u32).min(fh - 1);
            let si = ((sy * width + sx) * 4) as usize;
            let di = (((oy + y) * side + ox + x) * 4) as usize;
            if let (Some(src), Some(dst)) = (rgba.get(si..si + 4), out.get_mut(di..di + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    out
}

/// The PNG and JPEG files under `root` (scene-relative, `/`-separated,
/// sorted), skipping hidden folders; at most 300.
#[must_use]
pub fn project_images(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if out.len() >= 300 {
                return;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "build" || name == "__pycache__" {
                continue;
            }
            if path.is_dir() {
                if depth < 6 {
                    walk(root, &path, depth + 1, out);
                }
            } else if matches!(
                path.extension()
                    .and_then(|x| x.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("png" | "jpg" | "jpeg")
            ) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out
}

impl Ui {
    /// The Sprite, Tilemap, Camera 2D, Follow Camera 2D and Collider 2D
    /// sections of `e` (those it has).
    pub(super) fn sections_2d(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        if e.data.sprite.is_some() {
            let kind = ComponentKind::Sprite;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), e.id, Some(kind), true);
            y = ny;
            if open {
                y = self.sprite_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }
        if e.data.tilemap.is_some() {
            let kind = ComponentKind::Tilemap;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), e.id, Some(kind), true);
            y = ny;
            if open {
                y = self.tilemap_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }
        if e.data.camera_2d.is_some() {
            let kind = ComponentKind::Camera2D;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), e.id, Some(kind), true);
            y = ny;
            if open {
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Half Height",
                    Field::TwoD(Field2D::CameraHalfHeight),
                );
                y = self.dim_lines(
                    canvas,
                    x,
                    y,
                    w,
                    &["Sees twice this many units top to bottom."],
                );
                y += 6.0;
            }
        }
        if e.data.camera_follow_2d.is_some() {
            let kind = ComponentKind::CameraFollow2D;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), e.id, Some(kind), true);
            y = ny;
            if open {
                y = self.follow_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }
        if e.data.collider_2d.is_some() {
            let kind = ComponentKind::Collider2D;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), e.id, Some(kind), true);
            y = ny;
            if open {
                y = self.choice_row(canvas, x, y, w, e, "Shape", Choice::Collider2DShape);
                y = self.fields_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Size",
                    &[
                        Field::TwoD(Field2D::ColliderExtents(0)),
                        Field::TwoD(Field2D::ColliderExtents(1)),
                    ],
                );
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Radius",
                    Field::TwoD(Field2D::ColliderRadius),
                );
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Half Height",
                    Field::TwoD(Field2D::ColliderHalfHeight),
                );
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Density",
                    Field::TwoD(Field2D::ColliderDensity),
                );
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Friction",
                    Field::TwoD(Field2D::ColliderFriction),
                );
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Bounciness",
                    Field::TwoD(Field2D::ColliderRestitution),
                );
                y = self.flag_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Is Trigger",
                    Flag::TwoD(Flag2D::ColliderSensor),
                );
                y = self.flag_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Enabled",
                    Flag::TwoD(Flag2D::ColliderEnabled),
                );
                if e.data.rigid_body.is_none() {
                    y = self.dim_lines(
                        canvas,
                        x,
                        y,
                        w,
                        &["No Rigidbody: solid ground that stays put."],
                    );
                }
                y += 6.0;
            }
        }
        y
    }

    /// Several number fields side by side under one label.
    fn fields_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        label: &str,
        fields: &[Field],
    ) -> f32 {
        let values: Vec<Option<f32>> = fields
            .iter()
            .map(|f| crate::fields::get(&e.data, *f))
            .collect();
        if values.iter().all(Option::is_none) {
            return y;
        }
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let gap = 4.0;
            let n = fields.len().max(1) as f32;
            let fw = (w - layout::LABEL_W - (n - 1.0) * gap) / n;
            let tags = [
                color::axis_x(),
                color::axis_y(),
                color::axis_z(),
                color::text_dim(),
            ];
            for (i, (f, v)) in fields.iter().zip(values).enumerate() {
                if let Some(v) = v {
                    let fx = x + layout::LABEL_W + i as f32 * (fw + gap);
                    let tag = (fields.len() > 1).then(|| tags[i.min(3)]);
                    self.float_field(
                        canvas,
                        Rect::new(fx, y, fw, layout::FIELD_H),
                        e.id,
                        *f,
                        v,
                        tag,
                    );
                }
            }
        }
        y + layout::ROW_STEP
    }

    /// A picture row: the image's path, opening a menu of the project's
    /// images.
    fn image_row(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        (x, y, w): (f32, f32, f32),
        label: &str,
        current: &str,
        make: impl Fn(String) -> Edit2D,
        id: u64,
    ) -> f32 {
        let missing = !current.is_empty() && !doc.resolve(current).exists();
        let text = if current.is_empty() {
            "None (plain colour)".to_string()
        } else if missing {
            format!("{current} (missing)")
        } else {
            current.to_string()
        };
        let root = doc.base_dir();
        let current = current.to_string();
        self.menu_row(canvas, x, y, w, label, (&text, missing), || {
            let item = |label: String, path: String| super::menus::MenuItem {
                label: if path == current {
                    format!("✓ {label}")
                } else {
                    format!("   {label}")
                },
                shortcut: "",
                action: Some(super::menus::Action::Intent(Intent::Edit2D {
                    id,
                    edit: make(path),
                })),
                enabled: true,
            };
            let mut items = vec![item("None".into(), String::new())];
            let images = project_images(&root);
            if images.is_empty() {
                items.push(super::menus::MenuItem {
                    label: "   (no PNG or JPEG files in the project)".into(),
                    shortcut: "",
                    action: None,
                    enabled: false,
                });
            } else {
                items.push(super::menus::MenuItem::separator());
            }
            items.extend(images.into_iter().map(|p| item(p.clone(), p)));
            items
        })
    }

    /// A picture of frame `frame` of `image` (`columns × rows`), `side`
    /// logical pixels square, at `(x, y)`; a checker when there's none.
    fn frame_thumb(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        (x, y, side): (f32, f32, f32),
        image: &str,
        (frame, columns, rows): (u32, u32, u32),
        tint: [f32; 4],
    ) {
        canvas.fill_rounded(x, y, side, side, 3.0, color::field());
        let px = (side * canvas.scale()).round().max(1.0) as u32;
        let file = doc.resolve(image);
        let pic = if image.is_empty() {
            None
        } else {
            self.thumbs_2d
                .get(&file, frame, columns, rows, px)
                .map(<[u8]>::to_vec)
        };
        match pic {
            Some(rgba) => canvas.blit_rgba(x, y, px, px, &rgba),
            None => {
                let c = |v: f32| v.clamp(0.0, 1.0);
                let fill = tiny_skia::Color::from_rgba(c(tint[0]), c(tint[1]), c(tint[2]), 1.0)
                    .unwrap_or(tiny_skia::Color::WHITE);
                canvas.fill_rounded(x + 3.0, y + 3.0, side - 6.0, side - 6.0, 2.0, fill);
            }
        }
    }

    /// The Sprite section's rows: picture, sheet layout, look, clips.
    fn sprite_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(s) = e.data.sprite.clone() else {
            return y;
        };
        let id = e.id;
        y = self.image_row(
            canvas,
            doc,
            (x, y, w),
            "Image",
            &s.image,
            Edit2D::SpriteImage,
            id,
        );
        // The frame it shows while editing.
        let side = 56.0;
        if self.row_visible(Rect::new(x, y, w, side)) {
            self.frame_thumb(
                canvas,
                doc,
                (x + layout::LABEL_W, y, side),
                &s.image,
                (s.frame, s.columns, s.rows),
                s.color,
            );
            let frames = s.columns.max(1) * s.rows.max(1);
            let note = format!("frame {} of {frames}", s.frame.min(frames - 1) + 1);
            canvas.text(
                x + layout::LABEL_W + side + 8.0,
                y + side / 2.0 - 7.0,
                layout::SMALL_SIZE,
                &note,
                color::text_dim(),
            );
        }
        y += side + 6.0;
        let f = |f: Field2D| Field::TwoD(f);
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Size",
            &[f(Field2D::SpriteSize(0)), f(Field2D::SpriteSize(1))],
        );
        y = self.color_row(canvas, x, y, w, e, "Color", super::ColorTarget::Sprite);
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Sheet",
            &[f(Field2D::SpriteColumns), f(Field2D::SpriteRows)],
        );
        y = self.float_row(canvas, x, y, w, e, "Frame", f(Field2D::SpriteFrame));
        y = self.float_row(canvas, x, y, w, e, "Layer", f(Field2D::SpriteLayer));
        y = self.flag_row(
            canvas,
            x,
            y,
            w,
            e,
            "Flip X",
            Flag::TwoD(Flag2D::SpriteFlipX),
        );
        y = self.flag_row(
            canvas,
            x,
            y,
            w,
            e,
            "Flip Y",
            Flag::TwoD(Flag2D::SpriteFlipY),
        );
        y = self.flag_row(
            canvas,
            x,
            y,
            w,
            e,
            "Pixel Art",
            Flag::TwoD(Flag2D::SpritePixelated),
        );
        y = self.clip_rows(canvas, doc, x, y + 4.0, w, e);
        y
    }

    /// The sprite's clips: each one's name, frames, speed and whether it
    /// loops; Add Clip; and the clip it plays from the start.
    fn clip_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(s) = e.data.sprite.clone() else {
            return y;
        };
        let id = e.id;
        let editing = self.play == PlayState::Editing;
        if self.row_visible(Rect::new(x, y, w, 18.0)) {
            canvas.text_in(
                crate::canvas::Face::Medium,
                x,
                y + 2.0,
                layout::FONT_SIZE,
                "Clips",
                color::text(),
            );
        }
        y += 22.0;
        for (i, clip) in s.clips.iter().enumerate().take(32) {
            let n = i as u8;
            let r = Rect::new(x, y, w, layout::FIELD_H);
            if self.row_visible(r) {
                // A thumbnail of the clip's first frame, its name, Loop
                // and remove.
                let side = layout::FIELD_H;
                self.frame_thumb(
                    canvas,
                    doc,
                    (x, y, side),
                    &s.image,
                    (clip.first, s.columns, s.rows),
                    s.color,
                );
                let rm = Rect::new(x + w - 24.0, y, 24.0, layout::FIELD_H);
                let name_r = Rect::new(x + side + 6.0, y, w - side - 6.0 - 30.0, layout::FIELD_H);
                self.text2d_field(canvas, name_r, id, Text2D::ClipName(n), &clip.name);
                let style = self.editable_style(ButtonStyle::Danger);
                self.tip(rm, "Remove this clip");
                if self.button(canvas, rm, "✕", style) && editing {
                    self.out.push(Intent::Edit2D {
                        id,
                        edit: Edit2D::RemoveClip(i),
                    });
                }
            }
            y += layout::ROW_STEP;
            y = self.fields_row(
                canvas,
                x,
                y,
                w,
                e,
                "Frames",
                &[
                    Field::TwoD(Field2D::ClipFirst(n)),
                    Field::TwoD(Field2D::ClipLast(n)),
                ],
            );
            y = self.float_row(canvas, x, y, w, e, "FPS", Field::TwoD(Field2D::ClipFps(n)));
            y = self.flag_row(canvas, x, y, w, e, "Loop", Flag::TwoD(Flag2D::ClipLoop(n)));
            y += 4.0;
        }
        let add = Rect::new(x, y, w, 24.0);
        if self.row_visible(add) {
            let style = self.editable_style(ButtonStyle::Normal);
            self.tip(
                add,
                "A clip plays frames first..last of the sheet, in order",
            );
            if self.button(canvas, add, "+ Add Clip", style) && editing {
                self.out.push(Intent::Edit2D {
                    id,
                    edit: Edit2D::AddClip,
                });
            }
        }
        y += 30.0;
        if !s.clips.is_empty() {
            let current = s.play.clone();
            let missing = !current.is_empty() && s.clip(&current).is_none();
            let text = if current.is_empty() {
                "None (shows its frame)".to_string()
            } else {
                current.clone()
            };
            let names: Vec<String> = s.clips.iter().map(|c| c.name.clone()).collect();
            y = self.menu_row(canvas, x, y, w, "Plays First", (&text, missing), || {
                let item = |label: &str, name: &str| super::menus::MenuItem {
                    label: if name == current {
                        format!("✓ {label}")
                    } else {
                        format!("   {label}")
                    },
                    shortcut: "",
                    action: Some(super::menus::Action::Intent(Intent::Edit2D {
                        id,
                        edit: Edit2D::SpritePlay(name.to_string()),
                    })),
                    enabled: true,
                };
                let mut items = vec![item("None", "")];
                items.extend(names.iter().map(|n| item(n, n)));
                items
            });
        }
        self.dim_lines(
            canvas,
            x,
            y,
            w,
            &["Scripts: entity.sprite.play(\"run\"), .flip_x = True."],
        )
    }

    /// A one-line text field of a 2D component (commits on Enter).
    fn text2d_field(&mut self, canvas: &mut Canvas, r: Rect, id: u64, which: Text2D, text: &str) {
        let target = EditTarget::Text2D(which);
        let editing = self.is_editing(id, target);
        let hot = self.hot(r);
        let bg = if editing {
            color::viewport_bg()
        } else if hot {
            color::hover()
        } else {
            color::field()
        };
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, bg);
        if editing {
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.0);
            self.draw_edit_text(canvas, r, r.x + 6.0);
            self.cursor = CursorHint::Text;
            return;
        }
        canvas.text_clipped(
            r.x + 6.0,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            text,
            color::text(),
            r.w - 10.0,
        );
        if hot && self.play == PlayState::Editing {
            self.cursor = CursorHint::Text;
            if self.p.pressed {
                self.edit = Some(TextEdit {
                    id,
                    target,
                    buffer: text.to_string(),
                    rect: r,
                    fresh: true,
                    attr: None,
                });
            }
        }
    }

    /// What committing a 2D text field does.
    pub(super) fn commit_text_2d(id: u64, which: Text2D, buffer: &str) -> Option<Intent> {
        let text = buffer.trim();
        if text.is_empty() {
            return None;
        }
        Some(match which {
            Text2D::ClipName(n) => Intent::Edit2D {
                id,
                edit: Edit2D::RenameClip(usize::from(n), text.to_string()),
            },
        })
    }

    /// The Tilemap section's rows: the palette and brush, the grid's
    /// size, the tileset and its layout, look and physics.
    fn tilemap_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(m) = e.data.tilemap.clone() else {
            return y;
        };
        let id = e.id;
        let editing = self.play == PlayState::Editing;
        y = self.image_row(
            canvas,
            doc,
            (x, y, w),
            "Tileset",
            &m.tileset,
            Edit2D::Tileset,
            id,
        );
        let f = |f: Field2D| Field::TwoD(f);
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Tiles",
            &[f(Field2D::TileColumns), f(Field2D::TileRows)],
        );
        // The palette: Erase, every tile, then the auto-tiles.
        if self.row_visible(Rect::new(x, y, w, 18.0)) {
            canvas.text_in(
                crate::canvas::Face::Medium,
                x,
                y + 2.0,
                layout::FONT_SIZE,
                "Paint",
                color::text(),
            );
        }
        y += 22.0;
        let cell = 34.0;
        let per_row = ((w + 4.0) / (cell + 4.0)).floor().max(1.0) as usize;
        let tiles = m.tile_count().min(1024);
        let mut slots: Vec<(BrushTool, Option<u32>, String)> =
            vec![(BrushTool::Tile(0), None, "Erase".into())];
        for t in 0..tiles {
            slots.push((
                BrushTool::Tile(i32::try_from(t).unwrap_or(0) + 1),
                Some(t),
                format!("Tile {t}"),
            ));
        }
        for (k, a) in m.autotiles.iter().enumerate() {
            // An auto-tile's middle piece: neighbours on every side.
            slots.push((
                BrushTool::Tile(-i32::try_from(k).unwrap_or(0) - 1),
                Some(a.first + 15),
                format!(
                    "Auto-tile \"{}\": picks tiles {}..{} by its neighbours",
                    a.name,
                    a.first,
                    a.first + 15
                ),
            ));
        }
        for (i, (tool, tile, hint)) in slots.iter().enumerate() {
            let (col, row) = (i % per_row, i / per_row);
            let r = Rect::new(
                x + col as f32 * (cell + 4.0),
                y + row as f32 * (cell + 4.0),
                cell,
                cell,
            );
            if !self.row_visible(r) {
                continue;
            }
            let on = self.brush.tool == Some(*tool);
            match tile {
                Some(t) => self.frame_thumb(
                    canvas,
                    doc,
                    (r.x, r.y, cell),
                    &m.tileset,
                    (*t, m.columns, m.rows),
                    m.color,
                ),
                None => {
                    canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::field());
                    canvas.line(
                        r.x + 8.0,
                        r.y + 8.0,
                        r.x + r.w - 8.0,
                        r.y + r.h - 8.0,
                        color::danger(),
                        2.0,
                    );
                    canvas.line(
                        r.x + r.w - 8.0,
                        r.y + 8.0,
                        r.x + 8.0,
                        r.y + r.h - 8.0,
                        color::danger(),
                        2.0,
                    );
                }
            }
            if let BrushTool::Tile(v) = tool {
                if *v < 0 {
                    canvas.text(
                        r.x + 3.0,
                        r.y + 1.0,
                        layout::SMALL_SIZE,
                        "A",
                        color::accent(),
                    );
                }
            }
            if on {
                canvas.stroke_rounded(
                    r.x - 1.0,
                    r.y - 1.0,
                    r.w + 2.0,
                    r.h + 2.0,
                    4.0,
                    color::accent(),
                    2.0,
                );
            }
            self.tip(r, hint);
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed && editing {
                    self.pick_tile(*tool);
                }
            }
        }
        let rows_used = slots.len().div_ceil(per_row);
        y += rows_used as f32 * (cell + 4.0) + 4.0;
        y = self.tile_size_row(canvas, x, y, w);
        let half = (w - 6.0) * 0.5;
        let auto = Rect::new(x, y, half, 24.0);
        let clear = Rect::new(x + half + 6.0, y, half, 24.0);
        if self.row_visible(auto) {
            let style = self.editable_style(ButtonStyle::Normal);
            self.tip(
                auto,
                "An auto-tile paints with 16 tiles in a row, picked by which neighbours match",
            );
            if self.button(canvas, auto, "+ Auto-tile", style) && editing {
                self.out.push(Intent::Edit2D {
                    id,
                    edit: Edit2D::AddAutoTile,
                });
            }
            self.tip(clear, "Empty every cell");
            if self.button(canvas, clear, "Clear All", style) && editing {
                self.out.push(Intent::Edit2D {
                    id,
                    edit: Edit2D::ClearTiles,
                });
            }
        }
        y += 30.0;
        for (k, a) in m.autotiles.iter().enumerate().take(16) {
            let label = format!("{} First", a.name);
            let r = Rect::new(x, y, w, layout::FIELD_H);
            if self.row_visible(r) {
                self.row_label(canvas, x, y, &label);
                let fr = Rect::new(
                    x + layout::LABEL_W,
                    y,
                    w - layout::LABEL_W - 30.0,
                    layout::FIELD_H,
                );
                let field = Field::TwoD(Field2D::AutoFirst(k as u8));
                self.float_field(canvas, fr, id, field, a.first as f32, None);
                let rm = Rect::new(x + w - 24.0, y, 24.0, layout::FIELD_H);
                let style = self.editable_style(ButtonStyle::Danger);
                self.tip(rm, "Remove this auto-tile (its cells are emptied)");
                if self.button(canvas, rm, "✕", style) && editing {
                    self.out.push(Intent::Edit2D {
                        id,
                        edit: Edit2D::RemoveAutoTile(k),
                    });
                }
            }
            y += layout::ROW_STEP;
        }
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Grid",
            &[f(Field2D::TileWidth), f(Field2D::TileHeight)],
        );
        y = self.float_row(canvas, x, y, w, e, "Cell Size", f(Field2D::TileSize));
        y = self.color_row(canvas, x, y, w, e, "Color", super::ColorTarget::Tilemap);
        y = self.float_row(canvas, x, y, w, e, "Layer", f(Field2D::TileLayer));
        y = self.flag_row(canvas, x, y, w, e, "Solid", Flag::TwoD(Flag2D::TileCollide));
        y = self.flag_row(
            canvas,
            x,
            y,
            w,
            e,
            "Pixel Art",
            Flag::TwoD(Flag2D::TilePixelated),
        );
        let filled = m.cells.iter().filter(|c| **c != 0).count();
        let note = format!(
            "{} × {} cells, {filled} painted · grows right and up from the entity",
            m.width, m.height
        );
        self.gfx_note(canvas, x, y, w, &note)
    }

    /// Picks a tile brush (or switches the brush off if it's picked).
    pub(super) fn pick_tile(&mut self, tool: BrushTool) {
        if self.brush.tool == Some(tool) {
            self.brush.tool = None;
            return;
        }
        if !self.brush.tool.is_some_and(BrushTool::is_tile) {
            self.brush.radius = TILE_SIZES[0];
        }
        self.brush.tool = Some(tool);
    }

    /// The tile brush's size presets (cells a side).
    fn tile_size_row(&mut self, canvas: &mut Canvas, x: f32, y: f32, w: f32) -> f32 {
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "Brush");
            let bx = x + layout::LABEL_W;
            let n = TILE_SIZES.len() as f32;
            let bw = (w - layout::LABEL_W - 4.0 * (n - 1.0)) / n;
            for (i, &v) in TILE_SIZES.iter().enumerate() {
                let b = Rect::new(bx + i as f32 * (bw + 4.0), y, bw, layout::FIELD_H);
                let style = if (self.brush.radius - v).abs() < 1e-4 {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Normal
                };
                if self.button(canvas, b, &format!("{v}×{v}"), style) {
                    self.brush.radius = v;
                }
            }
        }
        y + layout::ROW_STEP
    }

    /// The Follow Camera 2D rows.
    fn follow_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(follow) = e.data.camera_follow_2d.clone() else {
            return y;
        };
        let id = e.id;
        let current = follow.target.clone();
        let missing = !doc.entities.iter().any(|o| o.name() == current);
        let text = if missing {
            format!("{current} (missing)")
        } else {
            current.clone()
        };
        let mut names: Vec<String> = doc
            .entities
            .iter()
            .filter(|o| o.id != id && o.data.name.is_some())
            .map(|o| o.name().to_string())
            .collect();
        names.sort_unstable();
        names.dedup();
        y = self.menu_row(canvas, x, y, w, "Target", (&text, missing), || {
            names
                .iter()
                .map(|n| super::menus::MenuItem {
                    label: if *n == current {
                        format!("✓ {n}")
                    } else {
                        format!("   {n}")
                    },
                    shortcut: "",
                    action: Some(super::menus::Action::Intent(Intent::Edit2D {
                        id,
                        edit: Edit2D::FollowTarget(n.clone()),
                    })),
                    enabled: true,
                })
                .collect()
        });
        let f = |f: Field2D| Field::TwoD(f);
        y = self.float_row(
            canvas,
            x,
            y,
            w,
            e,
            "Smoothing (s)",
            f(Field2D::FollowSmoothing),
        );
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Offset",
            &[f(Field2D::FollowOffset(0)), f(Field2D::FollowOffset(1))],
        );
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Dead Zone",
            &[f(Field2D::FollowDeadZone(0)), f(Field2D::FollowDeadZone(1))],
        );
        y = self.fields_row(
            canvas,
            x,
            y,
            w,
            e,
            "Bounds",
            &[
                f(Field2D::FollowBounds(0)),
                f(Field2D::FollowBounds(1)),
                f(Field2D::FollowBounds(2)),
                f(Field2D::FollowBounds(3)),
            ],
        );
        self.dim_lines(
            canvas,
            x,
            y,
            w,
            &["Bounds: left, bottom, right, top (all 0: no limit)."],
        )
    }
}

/// The flat 2D view's grid: lines a round number of units apart (at
/// least ~24 px), the axes brighter.
pub(super) fn draw_grid_2d(canvas: &mut Canvas, proj: &crate::view::Projector, rect: Rect) {
    let (o, d) = proj.ray(rect.x, rect.y);
    let (o2, d2) = proj.ray(rect.x + rect.w, rect.y + rect.h);
    if d.z.abs() < 1e-6 || d2.z.abs() < 1e-6 {
        return;
    }
    let a = o + d * (-o.z / d.z);
    let b = o2 + d2 * (-o2.z / d2.z);
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    let per_px = (x1 - x0) / rect.w.max(1.0);
    let mut step = 1.0_f32;
    for s in [
        0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 1000.0,
    ] {
        step = s;
        if s / per_px >= 24.0 {
            break;
        }
    }
    let mut faint = color::border();
    faint.apply_opacity(0.55);
    let screen = |x: f32, y: f32| proj.project(glam::Vec3::new(x, y, 0.0));
    let mut gx = (x0 / step).floor() * step;
    let mut n = 0;
    while gx <= x1 && n < 400 {
        if let (Some(p), Some(q)) = (screen(gx, y0), screen(gx, y1)) {
            let c = if gx.abs() < step * 0.01 {
                color::axis_y()
            } else {
                faint
            };
            canvas.line(p.0, p.1, q.0, q.1, c, 1.0);
        }
        gx += step;
        n += 1;
    }
    let mut gy = (y0 / step).floor() * step;
    while gy <= y1 && n < 800 {
        if let (Some(p), Some(q)) = (screen(x0, gy), screen(x1, gy)) {
            let c = if gy.abs() < step * 0.01 {
                color::axis_x()
            } else {
                faint
            };
            canvas.line(p.0, p.1, q.0, q.1, c, 1.0);
        }
        gy += step;
        n += 1;
    }
}

/// What each 2D camera sees (16:9), as a dashed frame in the scene view.
pub(super) fn draw_camera_2d_frames(
    canvas: &mut Canvas,
    doc: &Document,
    proj: &crate::view::Projector,
) {
    for e in &doc.entities {
        let Some(cam) = e.data.camera_2d else {
            continue;
        };
        let at = doc
            .world_matrix(e)
            .map_or(glam::Vec3::ZERO, |m| m.w_axis.truncate());
        let c = glam::Vec2::new(at.x, at.y) + cam.center;
        let (hw, hh) = (cam.half_height * 16.0 / 9.0, cam.half_height);
        let corners = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh), (-hw, -hh)];
        let pts: Vec<(f32, f32)> = corners
            .iter()
            .filter_map(|(x, y)| proj.project(glam::Vec3::new(c.x + x, c.y + y, 0.0)))
            .collect();
        if pts.len() == 5 {
            for w in pts.windows(2) {
                // Dashes: every other tenth of each side.
                for k in (0..10).step_by(2) {
                    let t0 = k as f32 / 10.0;
                    let t1 = t0 + 0.1;
                    let lerp = |t: f32| {
                        (
                            w[0].0 + (w[1].0 - w[0].0) * t,
                            w[0].1 + (w[1].1 - w[0].1) * t,
                        )
                    };
                    let (p, q) = (lerp(t0), lerp(t1));
                    canvas.line(p.0, p.1, q.0, q.1, color::camera_icon(), 1.5);
                }
            }
        }
    }
}

/// The selected tilemap's cell lines while a tile brush is on.
pub(super) fn draw_tile_grid(
    canvas: &mut Canvas,
    doc: &Document,
    id: u64,
    proj: &crate::view::Projector,
) {
    let Some(e) = doc.index_of(id).map(|i| &doc.entities[i]) else {
        return;
    };
    let (Some(map), Some(model)) = (e.data.tilemap.as_ref(), doc.world_matrix(e)) else {
        return;
    };
    let mut c = color::accent();
    c.apply_opacity(0.25);
    let s = map.tile_size;
    let (w, h) = (map.width.min(256), map.height.min(256));
    let p = |x: f32, y: f32| proj.project(model.transform_point3(glam::Vec3::new(x, y, 0.0)));
    for i in 0..=w {
        if let (Some(a), Some(b)) = (p(i as f32 * s, 0.0), p(i as f32 * s, h as f32 * s)) {
            canvas.line(a.0, a.1, b.0, b.1, c, 1.0);
        }
    }
    for j in 0..=h {
        if let (Some(a), Some(b)) = (p(0.0, j as f32 * s), p(w as f32 * s, j as f32 * s)) {
            canvas.line(a.0, a.1, b.0, b.1, c, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_cut_from_the_sheet_and_fitted() {
        // A 4×2 sheet of 2×2-pixel frames; frame 5 is red.
        let (w, h) = (8u32, 4u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 2..4 {
            for x in 2..4 {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let out = cut_frame(&rgba, w, h, 5, 4, 2, 4);
        assert_eq!(out.len(), 4 * 4 * 4);
        assert!(
            out.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
            "{out:?}"
        );
        let other = cut_frame(&rgba, w, h, 0, 4, 2, 4);
        assert!(other.chunks_exact(4).all(|p| p[3] == 0));
    }

    #[test]
    fn picking_a_tile_switches_the_brush_to_it_and_paints_the_map() {
        let mut ui = Ui::new();
        let mut doc = Document::new();
        let id = doc.create(crate::doc::CreateKind::Tilemap);
        doc.selected = Some(id);
        ui.pick_tile(BrushTool::Tile(3));
        assert_eq!(ui.brush.tool, Some(BrushTool::Tile(3)));
        assert_eq!(ui.brush_target(&doc), Some(id));
        // The map's corner is at (-8, -2): cell (2, 3) is around (-5.5, 1.5).
        assert!(doc.brush(
            id,
            BrushTool::Tile(3),
            [-5.5, 1.5, 0.0],
            [0.0, 0.0, 1.0],
            1.0,
            1.0,
            [0.0; 3]
        ));
        doc.end_gesture();
        let i = doc.index_of(id).unwrap();
        assert_eq!(doc.entities[i].data.tilemap.as_ref().unwrap().get(2, 3), 3);
        // A 2×2 brush, then undo takes the stroke back.
        assert!(doc.brush(
            id,
            BrushTool::Tile(0),
            [-5.5, 1.5, 0.0],
            [0.0, 0.0, 1.0],
            2.0,
            1.0,
            [0.0; 3]
        ));
        let m = doc.entities[i].data.tilemap.as_ref().unwrap();
        assert_eq!(m.get(2, 3), 0, "erased");
        doc.undo();
        assert_eq!(doc.entities[i].data.tilemap.as_ref().unwrap().get(2, 3), 3);
        ui.pick_tile(BrushTool::Tile(3));
        assert_eq!(ui.brush.tool, None, "picking it again switches it off");
    }

    #[test]
    fn project_images_lists_pictures_but_not_hidden_folders() {
        let dir = std::env::temp_dir().join(format!("oberon_images_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("art")).unwrap();
        std::fs::create_dir_all(dir.join(".oberon")).unwrap();
        for f in ["art/hero.png", "tiles.JPG", ".oberon/x.png", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        assert_eq!(project_images(&dir), ["art/hero.png", "tiles.JPG"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
