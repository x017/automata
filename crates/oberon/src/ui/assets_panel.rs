//! The Assets tab of the bottom dock: every model, sound and font in the
//! project as a grid of tiles. Models show a rendered thumbnail, sounds
//! their waveform and length, fonts "Aa" in the font.
//!
//! Tiles behave like rows of the Files panel: drag one onto the scene to add
//! it, onto an entity in the Inspector to swap its model or clip;
//! double-click adds a model / previews a sound; right-click for the file's
//! menu. The filter narrows by name or folder.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::canvas::Canvas;
use crate::doc::Document;
use crate::thumbs::ThumbCache;

use super::project_panel::{asset_kind, open_intent, AssetKind, FileDrag};
use super::{color, layout, ButtonStyle, CursorHint, EditTarget, PlayState, Rect, TextEdit, Ui};

/// Which assets the browser lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetFilter {
    /// Model files (`.glb`, `.gltf`, `.obj`).
    #[default]
    Models,
    /// Sound files (`.ogg`, `.wav`, `.mp3`, `.flac`).
    Sounds,
    /// Font files (`.ttf`, `.otf`) for script text.
    Fonts,
}

/// The Assets browser's state.
#[derive(Default)]
pub struct AssetBrowser {
    /// The kind of asset shown.
    pub filter: AssetFilter,
    /// Name filter (case-insensitive substring of the relative path).
    pub query: String,
    /// The highlighted tile.
    pub selected: Option<PathBuf>,
    scroll: f32,
    /// Listing: project root, when read, and the models, sounds and fonts.
    listing: Option<(PathBuf, Instant, Listing)>,
    /// Rendered model thumbnails.
    pub thumbs: ThumbCache,
    /// "Aa" previews of fonts.
    fonts: crate::fonts::FontPreviews,
}

/// The project's assets by kind.
#[derive(Default)]
struct Listing {
    models: Vec<PathBuf>,
    sounds: Vec<PathBuf>,
    fonts: Vec<PathBuf>,
}

impl Listing {
    fn of(&self, filter: AssetFilter) -> &[PathBuf] {
        match filter {
            AssetFilter::Models => &self.models,
            AssetFilter::Sounds => &self.sounds,
            AssetFilter::Fonts => &self.fonts,
        }
    }
}

impl AssetBrowser {
    /// Re-reads the project on the next frame (files were added or removed).
    pub fn invalidate(&mut self) {
        self.listing = None;
    }

    /// The project's models, sounds and fonts, re-read every few seconds.
    fn lists(&mut self, root: &Path) -> &Listing {
        let stale = self
            .listing
            .as_ref()
            .is_none_or(|(r, at, _)| r != root || at.elapsed() > RELIST);
        if stale || self.listing.is_none() {
            let listing = Listing {
                models: crate::models::project_models(root, 2000),
                sounds: crate::sounds::project_sounds(root, 2000),
                fonts: crate::fonts::project_fonts(root, 2000),
            };
            self.listing = Some((root.to_path_buf(), Instant::now(), listing));
        }
        &self.listing.as_ref().expect("listed above").2
    }
}

/// How long a listing is trusted before the folders are read again.
const RELIST: Duration = Duration::from_secs(3);
/// Tile size (logical pixels): the picture is square, the name below it.
const TILE_W: f32 = 104.0;
const PIC: f32 = 80.0;
const TILE_H: f32 = PIC + 30.0;
const GAP: f32 = 8.0;
/// Thumbnails drawn per frame at most (the rest show a placeholder and
/// fill in over the next frames).
const THUMBS_PER_FRAME: usize = 3;

impl Ui {
    /// Shows the Assets tab in the bottom dock (or hides the dock when it
    /// is already showing it).
    pub fn toggle_assets(&mut self) {
        if self.panels.show_bottom && self.bottom_tab == super::BottomTab::Assets {
            self.panels.show_bottom = false;
        } else {
            self.panels.show_bottom = true;
            self.bottom_tab = super::BottomTab::Assets;
        }
    }

    /// Shows the Assets tab's fonts (a font was just imported).
    pub fn show_fonts(&mut self) {
        self.panels.show_bottom = true;
        self.bottom_tab = super::BottomTab::Assets;
        self.assets.filter = AssetFilter::Fonts;
        self.assets.scroll = 0.0;
        self.assets.invalidate();
    }

    /// The Assets tab's controls in the dock's title strip, right-aligned
    /// and ending at `right`: kind toggle, filter, Import.
    pub(super) fn assets_toolbar(&mut self, canvas: &mut Canvas, doc: &Document, strip: Rect) {
        let editing = self.play == PlayState::Editing;
        let root = self.project_root.clone();
        let (models, sounds, fonts) = {
            let l = self.assets.lists(&root);
            (l.models.len(), l.sounds.len(), l.fonts.len())
        };
        let h = strip.h - 8.0;
        let y = strip.y + 4.0;
        let mut x = strip.x + strip.w - 8.0;
        // Import… for the current kind.
        let iw = 70.0;
        x -= iw;
        let import = Rect::new(x, y, iw, h);
        self.tip(import, "Copy files from elsewhere into the project");
        let style = if editing {
            ButtonStyle::Ghost
        } else {
            ButtonStyle::Disabled
        };
        if self.button(canvas, import, "Import…", style) && editing {
            let kind = match self.assets.filter {
                AssetFilter::Models => super::menus::DialogKind::ImportModel(None),
                AssetFilter::Sounds => super::menus::DialogKind::ImportSound(None),
                AssetFilter::Fonts => super::menus::DialogKind::ImportFont,
            };
            self.open_dialog(kind, doc);
        }
        // The filter field.
        let fw = 160.0f32.min((strip.w * 0.25).max(80.0));
        x -= fw + 8.0;
        let field = Rect::new(x, y, fw, h);
        self.asset_query_field(canvas, field);
        // Models / Sounds / Fonts.
        for (filter, label) in [
            (AssetFilter::Fonts, format!("Fonts {fonts}")),
            (AssetFilter::Sounds, format!("Sounds {sounds}")),
            (AssetFilter::Models, format!("Models {models}")),
        ] {
            let bw = canvas.measure_text(layout::SMALL_SIZE, &label) + 20.0;
            x -= bw + 4.0;
            let b = Rect::new(x, y, bw, h);
            let style = if self.assets.filter == filter {
                ButtonStyle::Active
            } else {
                ButtonStyle::Ghost
            };
            if self.button(canvas, b, &label, style) {
                self.assets.filter = filter;
                self.assets.scroll = 0.0;
            }
        }
    }

    fn asset_query_field(&mut self, canvas: &mut Canvas, field: Rect) {
        let editing = self
            .edit
            .as_ref()
            .is_some_and(|e| e.target == EditTarget::AssetQuery);
        canvas.fill_rounded(field.x, field.y, field.w, field.h, 3.0, color::field());
        if editing {
            canvas.stroke_rounded(
                field.x,
                field.y,
                field.w,
                field.h,
                3.0,
                color::accent(),
                1.0,
            );
            self.draw_edit_text(canvas, field, field.x + 6.0);
            return;
        }
        let (text, c) = if self.assets.query.is_empty() {
            ("Filter", color::text_faint())
        } else {
            (self.assets.query.as_str(), color::text())
        };
        canvas.text_clipped(
            field.x + 6.0,
            field.y + (field.h - layout::SMALL_SIZE) / 2.0 - 1.0,
            layout::SMALL_SIZE,
            text,
            c,
            field.w - 12.0,
        );
        if self.hot(field) {
            self.cursor = CursorHint::Text;
            if self.p.pressed {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
                self.edit = Some(TextEdit {
                    id: 0,
                    target: EditTarget::AssetQuery,
                    buffer: self.assets.query.clone(),
                    rect: field,
                    fresh: false,
                    attr: None,
                });
            }
        }
    }

    /// The live filter: the field's text while it's being typed in.
    fn asset_query(&self) -> String {
        match &self.edit {
            Some(e) if e.target == EditTarget::AssetQuery => e.buffer.clone(),
            _ => self.assets.query.clone(),
        }
        .trim()
        .to_lowercase()
    }

    /// The tile grid.
    #[allow(clippy::too_many_lines)]
    pub(super) fn assets_body(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        let root = self.project_root.clone();
        let editing = self.play == PlayState::Editing;
        let query = self.asset_query();
        let filter = self.assets.filter;
        let rel = |p: &Path| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let files: Vec<PathBuf> = {
            self.assets
                .lists(&root)
                .of(filter)
                .iter()
                .filter(|p| query.is_empty() || rel(p).to_lowercase().contains(&query))
                .cloned()
                .collect()
        };
        let inner = Rect::new(
            rect.x + 10.0,
            rect.y + 8.0,
            (rect.w - 20.0).max(0.0),
            rect.h,
        );
        if files.is_empty() {
            let line = match (filter, query.is_empty()) {
                (_, false) => format!("Nothing matches \u{201c}{query}\u{201d}."),
                (AssetFilter::Models, true) => {
                    "No models yet. Drop .glb, .gltf or .obj files on the window, or use Import\u{2026}"
                        .to_string()
                }
                (AssetFilter::Sounds, true) => {
                    "No sounds yet. Drop .ogg, .wav, .mp3 or .flac files on the window, or use Import\u{2026}"
                        .to_string()
                }
                (AssetFilter::Fonts, true) => {
                    "No fonts yet. Drop .ttf or .otf files on the window, or use Import\u{2026}. \
                     Scripts draw with them: ctx.hud.text(..., font=\"fonts/Name.ttf\")"
                        .to_string()
                }
            };
            canvas.text_clipped(
                inner.x,
                inner.y + 4.0,
                layout::SMALL_SIZE,
                &line,
                color::text_dim(),
                inner.w,
            );
            return;
        }
        let cols = (((inner.w + GAP) / (TILE_W + GAP)).floor() as usize).max(1);
        let rows = files.len().div_ceil(cols);
        let content_h = rows as f32 * (TILE_H + GAP) + 8.0;
        let view_h = (rect.h - 8.0).max(0.0);
        if self.hot(rect) && self.p.scroll != 0.0 {
            self.assets.scroll -= self.p.scroll * 40.0;
        }
        self.assets.scroll = self.assets.scroll.clamp(0.0, (content_h - view_h).max(0.0));

        let prev = canvas.set_clip(Some((rect.x, rect.y, rect.w, rect.h)));
        let saved_clip = self.clip;
        self.clip = (rect.y, rect.y + rect.h);
        let scale = canvas.scale();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let px = (PIC * scale).round().max(8.0) as u32;
        let mut budget = THUMBS_PER_FRAME;
        let mut clicked = false;
        for (i, path) in files.iter().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let tile = Rect::new(
                inner.x + c as f32 * (TILE_W + GAP),
                inner.y + r as f32 * (TILE_H + GAP) - self.assets.scroll,
                TILE_W,
                TILE_H,
            );
            if tile.y + tile.h < rect.y || tile.y > rect.y + rect.h {
                continue;
            }
            let selected = self.assets.selected.as_deref() == Some(path.as_path());
            let hot = self.hot(tile);
            if selected || hot {
                let bg = if selected {
                    color::select()
                } else {
                    color::hover()
                };
                canvas.fill_rounded(tile.x, tile.y, tile.w, tile.h, layout::RADIUS, bg);
            }
            let pic = Rect::new(tile.x + (TILE_W - PIC) / 2.0, tile.y + 4.0, PIC, PIC);
            canvas.fill_rounded(pic.x, pic.y, pic.w, pic.h, 3.0, color::field());
            match filter {
                AssetFilter::Models => {
                    if self.assets.thumbs.get(path, px).is_none() && budget > 0 {
                        budget -= 1;
                        self.assets.thumbs.draw(path, px);
                    }
                    match self.assets.thumbs.get(path, px) {
                        Some(Some((rgba, w, h))) => canvas.blit_rgba(pic.x, pic.y, *w, *h, rgba),
                        Some(None) => {
                            canvas.text(
                                pic.x + 8.0,
                                pic.y + PIC / 2.0 - 7.0,
                                layout::SMALL_SIZE,
                                "Can't load",
                                color::danger(),
                            );
                        }
                        None => {}
                    }
                }
                AssetFilter::Sounds => self.draw_sound_tile(canvas, path, pic),
                AssetFilter::Fonts => {
                    let ink = color::text().to_color_u8();
                    let ink = [ink.red(), ink.green(), ink.blue()];
                    match self.assets.fonts.get(path, px, ink) {
                        Some(rgba) => canvas.blit_rgba(pic.x, pic.y, px, px, rgba),
                        None => {
                            canvas.text(
                                pic.x + 8.0,
                                pic.y + PIC / 2.0 - 7.0,
                                layout::SMALL_SIZE,
                                "Can't load",
                                color::danger(),
                            );
                        }
                    }
                }
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let tw = canvas
                .measure_text(layout::SMALL_SIZE, &name)
                .min(TILE_W - 8.0);
            canvas.text_clipped(
                tile.x + (TILE_W - tw) / 2.0,
                pic.y + PIC + 6.0,
                layout::SMALL_SIZE,
                &name,
                if selected {
                    color::text()
                } else {
                    color::text_dim()
                },
                TILE_W - 8.0,
            );
            if hot {
                let hint = match filter {
                    AssetFilter::Models => "drag onto the scene or an entity · double-click to add",
                    AssetFilter::Sounds => {
                        "drag onto the scene or an entity · double-click to hear it"
                    }
                    AssetFilter::Fonts => "double-click to copy its path for font=\"…\" in scripts",
                };
                self.tip(tile, &format!("{}\n{hint}", rel(path)));
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    clicked = true;
                    self.assets.selected = Some(path.clone());
                    if editing && !self.p.double_click && filter != AssetFilter::Fonts {
                        self.file_drag = Some(FileDrag {
                            path: path.clone(),
                            origin: (self.p.x, self.p.y),
                            active: false,
                        });
                    }
                    if self.p.double_click {
                        self.file_drag = None;
                        let kind = asset_kind(path, false);
                        let allowed = editing || matches!(kind, AssetKind::Audio | AssetKind::Font);
                        if let Some(intent) = open_intent(path, false).filter(|_| allowed) {
                            self.out.push(intent);
                        }
                    }
                }
                if self.p.right_pressed {
                    clicked = true;
                    self.assets.selected = Some(path.clone());
                    let menu = self.asset_menu(doc, path, editing);
                    self.open_menu(self.p.x, self.p.y, menu);
                }
            }
        }
        if !clicked && self.hot(rect) && self.p.pressed {
            self.assets.selected = None;
        }
        self.clip = saved_clip;
        canvas.restore_clip(prev);
    }

    /// A sound tile's picture: its waveform and length.
    fn draw_sound_tile(&mut self, canvas: &mut Canvas, path: &Path, pic: Rect) {
        let (peaks, length) = match self.clips.get(path) {
            Ok(info) => (info.peaks.clone(), Some(info.duration)),
            Err(_) => (Vec::new(), None),
        };
        let Some(length) = length else {
            canvas.text(
                pic.x + 8.0,
                pic.y + PIC / 2.0 - 7.0,
                layout::SMALL_SIZE,
                "Can't read",
                color::danger(),
            );
            return;
        };
        // Every other peak keeps bars legible at tile size.
        let bars: Vec<f32> = peaks
            .chunks(2)
            .map(|c| c.iter().copied().fold(0.0, f32::max))
            .collect();
        let n = bars.len().max(1) as f32;
        let wave = Rect::new(pic.x + 6.0, pic.y + 12.0, pic.w - 12.0, pic.h - 36.0);
        let step = wave.w / n;
        let mid = wave.y + wave.h / 2.0;
        for (i, peak) in bars.iter().enumerate() {
            let h = (peak * wave.h).max(1.0);
            canvas.fill_rect(
                wave.x + i as f32 * step,
                mid - h / 2.0,
                (step - 0.6).max(0.6),
                h,
                color::accent_soft(),
            );
        }
        let text = format_length(length);
        let tw = canvas.measure_text(layout::SMALL_SIZE, &text);
        canvas.text(
            pic.x + (pic.w - tw) / 2.0,
            pic.y + pic.h - 20.0,
            layout::SMALL_SIZE,
            &text,
            color::text_dim(),
        );
    }
}

/// `0:03.25`-style clip length.
fn format_length(seconds: f64) -> String {
    let s = seconds.max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let minutes = (s / 60.0).floor() as u32;
    format!("{minutes}:{:05.2}", s - f64::from(minutes) * 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_as_minutes_and_seconds() {
        assert_eq!(format_length(3.25), "0:03.25");
        assert_eq!(format_length(75.5), "1:15.50");
        assert_eq!(format_length(-1.0), "0:00.00");
    }
}
