//! Font files in the editor: importing them into the project, listing
//! them, and drawing a preview tile ("Aa" in the font) for the Assets tab.
//!
//! Fonts work like sounds: importing copies the file into the project's
//! `fonts/` folder so scripts name it with a short relative path
//! (`font="fonts/Pixel.ttf"`) and exports carry it along; files already
//! inside the project are used where they are.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
pub use automata_renderer::text::is_font_path;

use crate::models::{free_name, is_inside, project_files};

/// The project folder fonts are imported into.
pub const FONTS_DIR: &str = "fonts";

/// Font files under `root`, at most `limit`, sorted by path.
#[must_use]
pub fn project_fonts(root: &Path, limit: usize) -> Vec<PathBuf> {
    project_files(root, limit, is_font_path)
}

/// Copies a font into `project/fonts/` and returns where it landed (a file
/// already inside the project is returned unchanged).
///
/// # Errors
///
/// Fails when `src` isn't a font file that parses, or copying fails.
pub fn import_font(src: &Path, project: &Path) -> Result<PathBuf, String> {
    automata_renderer::text::load_font_file(src)?;
    if is_inside(src, project) {
        return Ok(src.to_path_buf());
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("the font has no file name")?;
    let dir = project.join(FONTS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = free_name(&dir, &name);
    std::fs::copy(src, &dest).map_err(|e| format!("copying {}: {e}", src.display()))?;
    Ok(dest)
}

/// How a script names a font file: its path relative to the scene folder,
/// with forward slashes (`fonts/Pixel.ttf`).
#[must_use]
pub fn script_path(font: &Path, scene_dir: &Path) -> String {
    font.strip_prefix(scene_dir)
        .unwrap_or(font)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// "Aa" drawn in `font`, centred in a `px`-square straight-alpha RGBA image
/// in `color`.
#[must_use]
pub fn preview(font: &FontArc, px: u32, color: [u8; 3]) -> Vec<u8> {
    let side = px.max(8);
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    #[allow(clippy::cast_precision_loss)]
    let full = side as f32;
    let ids: Vec<_> = "Aa".chars().map(|c| font.glyph_id(c)).collect();
    let span = |size: f32| {
        let scaled = font.as_scaled(PxScale::from(size));
        ids.iter().map(|&id| scaled.h_advance(id)).sum::<f32>() + scaled.kern(ids[0], ids[1])
    };
    // Half the tile tall, but wide faces shrink to leave a margin.
    let size = (full * 0.5).min(full * 0.75 / span(1.0).max(0.01));
    let scaled = font.as_scaled(PxScale::from(size));
    let width = span(size);
    let mut pen = (full - width) / 2.0;
    let baseline = (full + scaled.ascent() + scaled.descent()) / 2.0;
    for (i, &id) in ids.iter().enumerate() {
        if i > 0 {
            pen += scaled.kern(ids[i - 1], id);
        }
        let glyph = id.with_scale_and_position(size, ab_glyph::point(pen, baseline));
        pen += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let b = outline.px_bounds();
        outline.draw(|x, y, c| {
            #[allow(clippy::cast_possible_truncation)]
            let (gx, gy) = (b.min.x as i64 + i64::from(x), b.min.y as i64 + i64::from(y));
            if gx < 0 || gy < 0 || gx >= i64::from(side) || gy >= i64::from(side) {
                return;
            }
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let at = ((gy as u32 * side + gx as u32) * 4) as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let a = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            if a > rgba[at + 3] {
                rgba[at..at + 4].copy_from_slice(&[color[0], color[1], color[2], a]);
            }
        });
    }
    rgba
}

/// Font previews for the Assets tab, redrawn when a file changes.
#[derive(Default)]
pub struct FontPreviews {
    entries: HashMap<PathBuf, Preview>,
}

/// One file's preview and what it was drawn from.
struct Preview {
    modified: Option<SystemTime>,
    px: u32,
    color: [u8; 3],
    /// `None`: the file doesn't load.
    image: Option<Vec<u8>>,
}

impl std::fmt::Debug for FontPreviews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontPreviews")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl FontPreviews {
    /// The `px`-square preview of the font at `path`, or `None` when the
    /// file isn't a font that loads.
    pub fn get(&mut self, path: &Path, px: u32, color: [u8; 3]) -> Option<&[u8]> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let fresh = self
            .entries
            .get(path)
            .is_some_and(|p| p.modified == modified && p.px == px && p.color == color);
        if !fresh {
            let image = automata_renderer::text::load_font_file(path)
                .ok()
                .map(|f| preview(&f, px, color));
            let entry = Preview {
                modified,
                px,
                color,
                image,
            };
            self.entries.insert(path.to_path_buf(), entry);
        }
        self.entries.get(path).and_then(|p| p.image.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_font() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/oberon/pause_menu/fonts/PressStart2P-Regular.ttf")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon-fonts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn importing_copies_into_fonts_and_rejects_non_fonts() {
        let project = scratch("import");
        let dest = import_font(&pixel_font(), &project).unwrap();
        assert_eq!(dest, project.join("fonts").join("PressStart2P-Regular.ttf"));
        assert_eq!(project_fonts(&project, 10), vec![dest.clone()]);
        assert_eq!(
            script_path(&dest, &project),
            "fonts/PressStart2P-Regular.ttf"
        );
        // Already inside: used where it is. A second import gets a new name.
        assert_eq!(import_font(&dest, &project).unwrap(), dest);
        let fake = project.join("fake.ttf");
        std::fs::write(&fake, b"not a font").unwrap();
        let err = import_font(&fake, &project).unwrap_err();
        assert!(err.contains("TrueType"), "{err}");
        assert!(import_font(&project.join("x.png"), &project).is_err());
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn previews_draw_aa_and_are_cached() {
        let font = automata_renderer::text::load_font_file(&pixel_font()).unwrap();
        let img = preview(&font, 64, [200, 210, 220]);
        assert_eq!(img.len(), 64 * 64 * 4);
        let lit: Vec<(u32, u32)> = (0..64 * 64)
            .filter(|i| img[*i as usize * 4 + 3] > 128)
            .map(|i| (i % 64, i / 64))
            .collect();
        assert!(lit.len() > 100, "{} lit pixels", lit.len());
        // Roughly centred: lit pixels on both halves, none on the border.
        assert!(lit.iter().any(|p| p.0 < 28) && lit.iter().any(|p| p.0 > 36));
        assert!(lit
            .iter()
            .all(|p| p.0 > 0 && p.0 < 63 && p.1 > 0 && p.1 < 63));
        let mut cache = FontPreviews::default();
        assert!(cache.get(&pixel_font(), 64, [255; 3]).is_some());
        assert!(cache.get(Path::new("missing.ttf"), 64, [255; 3]).is_none());
    }
}
