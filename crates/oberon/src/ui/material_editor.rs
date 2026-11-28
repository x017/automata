//! The material editor: `.mat.ron` files open here instead of as text.
//!
//! The left side is a live preview (a sphere, cube or plane wearing the
//! material, rendered on the GPU in a world of its own); the right side
//! edits the colour, surface, glow, the four texture slots and their
//! tiling. Like the UI editor, every edit rewrites the tab's text (so
//! Ctrl+Z, Ctrl+S and "Edit as Text" all work on the real file), and the
//! unsaved version shows on every object using the material at once
//! ([`automata_assets::set_material_preview`], driven by the app from
//! [`Ui::material_tab_previews`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use automata_assets::material::{relative_map_path, MaterialFile};
use automata_assets::MapSlot;

use super::ui_editor::{hex, num, rgba, LABEL_W, ROW_H, STATUS_H, TOOLBAR_H};
use super::{color, layout, ButtonStyle, CursorHint, EditTarget, Intent, LogLevel, Rect, Ui};
use crate::canvas::Canvas;
use crate::code::{Key, Mods};

/// Where new materials go, under the project folder.
pub const MATERIALS_DIR: &str = "materials";

/// Where new shaders go, under the project folder.
pub const SHADERS_DIR: &str = "shaders";

/// Writes a new shader file from the template of its kind (folders made as
/// needed).
///
/// # Errors
/// The I/O error, as text.
pub fn create_shader(path: &Path, kind: automata_renderer::ShaderKind) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let template = match kind {
        automata_renderer::ShaderKind::Material => automata_renderer::MATERIAL_TEMPLATE,
        automata_renderer::ShaderKind::Post => automata_renderer::POST_TEMPLATE,
    };
    std::fs::write(path, template).map_err(|e| format!("{}: {e}", path.display()))
}

/// The shape the material preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaterialShape {
    /// A sphere: shows roughness and metalness best.
    #[default]
    Sphere,
    /// A tilted cube: shows how textures meet at edges.
    Cube,
    /// A flat square: shows tiling.
    Plane,
}

impl MaterialShape {
    /// Every shape, in toolbar order.
    pub const ALL: [Self; 3] = [Self::Sphere, Self::Cube, Self::Plane];

    /// The toolbar label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Sphere => "Sphere",
            Self::Cube => "Cube",
            Self::Plane => "Plane",
        }
    }
}

/// A number or colour of the material being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatProp {
    /// Base colour (`#rrggbb`, alpha allowed).
    Color,
    /// Roughness, 0–1.
    Roughness,
    /// Metalness, 0–1.
    Metallic,
    /// Glow colour (`#rrggbb`).
    Emissive,
    /// Glow brightness.
    EmissiveStrength,
    /// How much the glow lights its surroundings.
    GlowLight,
    /// How strongly the normal map bends the light.
    NormalStrength,
    /// Texture repeats across.
    TilingX,
    /// Texture repeats down.
    TilingY,
    /// Texture shift across.
    OffsetX,
    /// Texture shift down.
    OffsetY,
    /// One of the custom shader's numbers (`0..8`: `p0.x` … `p1.w`).
    ShaderParam(u8),
}

impl MatProp {
    fn is_colour(self) -> bool {
        matches!(self, Self::Color | Self::Emissive)
    }

    /// The value of a number property.
    fn get(self, f: &MaterialFile) -> f32 {
        match self {
            Self::Roughness => f.roughness,
            Self::Metallic => f.metallic,
            Self::EmissiveStrength => f.emissive_strength,
            Self::GlowLight => f.glow_light,
            Self::NormalStrength => f.normal_strength,
            Self::TilingX => f.tiling[0],
            Self::TilingY => f.tiling[1],
            Self::OffsetX => f.offset[0],
            Self::OffsetY => f.offset[1],
            Self::ShaderParam(i) => f.shader_params.get(usize::from(i)).copied().unwrap_or(0.0),
            Self::Color | Self::Emissive => 0.0,
        }
    }

    /// Sets a number property, kept in range.
    fn set(self, f: &mut MaterialFile, v: f32) {
        if !v.is_finite() {
            return;
        }
        match self {
            Self::Roughness => f.roughness = v.clamp(0.0, 1.0),
            Self::Metallic => f.metallic = v.clamp(0.0, 1.0),
            Self::EmissiveStrength => f.emissive_strength = v.max(0.0),
            Self::GlowLight => f.glow_light = v.max(0.0),
            Self::NormalStrength => f.normal_strength = v.clamp(0.0, 4.0),
            Self::TilingX => f.tiling[0] = v.clamp(-1000.0, 1000.0),
            Self::TilingY => f.tiling[1] = v.clamp(-1000.0, 1000.0),
            Self::OffsetX => f.offset[0] = v,
            Self::OffsetY => f.offset[1] = v,
            Self::ShaderParam(i) => {
                let i = usize::from(i);
                if i < automata_renderer::SHADER_PARAMS {
                    if f.shader_params.len() <= i {
                        f.shader_params.resize(i + 1, 0.0);
                    }
                    f.shader_params[i] = v;
                }
            }
            Self::Color | Self::Emissive => {}
        }
    }

    /// The slider's range (the field takes any value the property allows).
    fn range(self) -> (f32, f32) {
        match self {
            Self::EmissiveStrength => (0.0, 8.0),
            Self::GlowLight | Self::NormalStrength => (0.0, 2.0),
            _ => (0.0, 1.0),
        }
    }
}

/// The material editor's state (one editor, following the active tab).
#[derive(Default)]
pub struct MaterialEditor {
    /// The last text parsed and what it gave.
    parsed: Option<(String, Result<MaterialFile, String>)>,
    /// Material files shown as text instead.
    source: HashSet<PathBuf>,
    props_scroll: f32,
    shape: MaterialShape,
    /// The rendered preview (pixels, physical width, height).
    preview: Option<(Vec<u8>, u32, u32)>,
    /// Where the preview goes (logical), from the last frame.
    preview_rect: Option<Rect>,
    /// A slider being dragged, with its live value: shown everywhere but
    /// written to the file (one undo step) on release.
    slider: Option<(MatProp, f32)>,
    /// Where each slider's track was drawn last frame.
    slider_tracks: Vec<(MatProp, Rect)>,
    /// Texture thumbnails by image (kept alive with it) and pixel size.
    thumbs: HashMap<(usize, u32), (Arc<automata_assets::Image>, Vec<u8>)>,
}

/// How scenes name the material at `path`: relative to the scene folder
/// (the asset root), else `materials/<file>`.
fn scene_path(path: &Path) -> String {
    automata_assets::root()
        .and_then(|root| {
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            abs.strip_prefix(root).ok().map(|p| {
                p.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
        })
        .unwrap_or_else(|| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{MATERIALS_DIR}/{name}")
        })
}

/// An RGBA image shrunk (nearest pixel) to `size`×`size`, letterboxed.
fn thumbnail(img: &automata_assets::Image, size: u32) -> Vec<u8> {
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    if img.width == 0 || img.height == 0 || img.rgba.len() < (img.width * img.height * 4) as usize {
        return out;
    }
    #[allow(clippy::cast_precision_loss)]
    let k = (img.width.max(img.height) as f32) / size as f32;
    for y in 0..n {
        for x in 0..n {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let (sx, sy) = (((x as f32 + 0.5) * k) as u32, ((y as f32 + 0.5) * k) as u32);
            if sx < img.width && sy < img.height {
                let si = ((sy * img.width + sx) * 4) as usize;
                let di = (y * n + x) * 4;
                out[di..di + 4].copy_from_slice(&img.rgba[si..si + 4]);
                out[di + 3] = 255;
            }
        }
    }
    out
}

impl Ui {
    /// The path of the visible tab when it's a material file.
    #[must_use]
    pub fn material_editor_path(&self) -> Option<PathBuf> {
        self.code
            .current()
            .filter(|t| automata_assets::is_material_path(&t.path))
            .map(|t| t.path.clone())
    }

    /// Whether the material editor is the visible center tab.
    #[must_use]
    pub fn material_editor_shown(&self) -> bool {
        self.material_editor_path()
            .is_some_and(|p| !self.mated.source.contains(&p))
    }

    /// The material the editor shows (a slider being dragged included),
    /// as last parsed; `None` while it doesn't parse or isn't shown.
    #[must_use]
    pub fn material_editor_file(&self) -> Option<MaterialFile> {
        self.material_editor_path()?;
        let Some((_, Ok(file))) = &self.mated.parsed else {
            return None;
        };
        let mut file = file.clone();
        if let Some((prop, v)) = self.mated.slider {
            prop.set(&mut file, v);
        }
        Some(file)
    }

    /// Shows the active material as text (`true`) or in the editor.
    pub fn set_material_source(&mut self, text: bool) {
        let Some(path) = self.material_editor_path() else {
            return;
        };
        if text {
            self.mated.source.insert(path);
        } else {
            self.mated.source.remove(&path);
        }
        self.mated.slider = None;
    }

    /// Picks the preview shape.
    pub fn set_material_shape(&mut self, shape: MaterialShape) {
        self.mated.shape = shape;
    }

    /// Every open material tab whose text differs from its file, with
    /// what it reads as (`None`: saved, or it doesn't read). The app
    /// shows these everywhere with [`automata_assets::set_material_preview`].
    #[must_use]
    pub fn material_tab_previews(&self) -> Vec<(PathBuf, Option<MaterialFile>)> {
        let active = self.material_editor_path();
        self.code
            .tabs
            .iter()
            .filter(|t| automata_assets::is_material_path(&t.path))
            .map(|t| {
                let sliding = self.mated.slider.is_some() && active.as_ref() == Some(&t.path);
                let file = (t.buf.dirty() || sliding)
                    .then(|| MaterialFile::parse(&t.buf.text()).ok())
                    .flatten()
                    .map(|mut f| {
                        if let Some((prop, v)) = self.mated.slider.filter(|_| sliding) {
                            prop.set(&mut f, v);
                        }
                        f
                    });
                (t.path.clone(), file)
            })
            .collect()
    }

    /// The material preview the editor wants drawn: the file, the shape
    /// and the physical size. `None` when the editor isn't showing.
    #[must_use]
    pub fn material_preview_request(
        &self,
        scale: f32,
    ) -> Option<(PathBuf, MaterialShape, u32, u32)> {
        if !self.material_editor_shown() {
            return None;
        }
        let r = self.mated.preview_rect?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let px = |v: f32| (v * scale).round().clamp(16.0, 4096.0) as u32;
        Some((
            self.material_editor_path()?,
            self.mated.shape,
            px(r.w),
            px(r.h),
        ))
    }

    /// Hands the editor the rendered preview (`None` clears it).
    pub fn set_material_preview_image(&mut self, image: Option<(&[u8], u32, u32)>) {
        match image {
            Some((rgba, w, h)) => {
                let slot = self.mated.preview.get_or_insert_with(|| (Vec::new(), 0, 0));
                slot.0.clear();
                slot.0.extend_from_slice(rgba);
                slot.1 = w;
                slot.2 = h;
            }
            None => self.mated.preview = None,
        }
    }

    /// Re-parses the tab's text when it changed.
    fn refresh_material(&mut self, idx: usize) {
        let text = self.code.tabs[idx].buf.text();
        if self.mated.parsed.as_ref().map(|(t, _)| t) != Some(&text) {
            let file = MaterialFile::parse(&text);
            self.mated.parsed = Some((text, file));
        }
    }

    /// Changes the visible material: `f` edits a copy; a real change
    /// rewrites the tab's text as one undo step.
    pub fn edit_material(&mut self, f: impl FnOnce(&mut MaterialFile)) {
        let Some(idx) = self.code.active.filter(|_| self.material_editor_shown()) else {
            return;
        };
        self.refresh_material(idx);
        let Some((_, Ok(before))) = &self.mated.parsed else {
            return;
        };
        let mut file = before.clone();
        f(&mut file);
        if file == *before {
            return;
        }
        let tab = &mut self.code.tabs[idx];
        let text = file.to_ron(&scene_path(&tab.path));
        tab.buf.set_text(&text);
        self.mated.parsed = Some((text, Ok(file)));
    }

    /// Puts `image` in texture slot `slot` of the visible material (its
    /// path stored relative to the material file), or empties it.
    pub fn set_material_map(&mut self, slot: usize, image: Option<&Path>) {
        let (Some(material), Some(&which)) = (self.material_editor_path(), MapSlot::ALL.get(slot))
        else {
            return;
        };
        if let Some(img) = image {
            if !automata_assets::material::is_image_path(img) {
                self.log(
                    LogLevel::Warn,
                    format!(
                        "{} isn't a PNG or JPEG image",
                        crate::files::display_name(img)
                    ),
                );
                return;
            }
        }
        let rel = image.map(|img| relative_map_path(&material, img));
        self.edit_material(|f| *f.map_mut(which) = rel.unwrap_or_default());
    }

    /// Makes `shader` the visible material's custom shader (its path
    /// stored relative to the material file; a file that doesn't exist yet
    /// is made from the template and opened), or goes back to the standard
    /// surface.
    pub fn set_material_shader(&mut self, shader: Option<&Path>) {
        let Some(material) = self.material_editor_path() else {
            return;
        };
        let Some(file) = shader else {
            self.edit_material(|f| {
                f.shader.clear();
                f.shader_params.clear();
            });
            return;
        };
        if !automata_assets::shader::is_shader_path(file) {
            self.log(
                LogLevel::Warn,
                format!("{} isn't a .wgsl shader", crate::files::display_name(file)),
            );
            return;
        }
        let created = !file.exists();
        if created {
            if let Err(e) = create_shader(file, automata_renderer::ShaderKind::Material) {
                self.log(LogLevel::Error, format!("Couldn't create the shader: {e}"));
                return;
            }
            self.files_changed(None);
        } else if std::fs::read_to_string(file).is_ok_and(|text| {
            automata_assets::shader::kind_of_source(&text) == automata_renderer::ShaderKind::Post
        }) {
            self.log(
                LogLevel::Warn,
                format!(
                    "{} is a post shader (fn post): materials need fn material",
                    crate::files::display_name(file)
                ),
            );
            return;
        }
        let rel = relative_map_path(&material, file);
        self.edit_material(|f| f.shader = rel);
        if created {
            self.log(
                LogLevel::Info,
                format!(
                    "Created {} from the material template · save it to see it change",
                    crate::files::display_name(file)
                ),
            );
            self.open_code(file, None);
        }
    }

    /// Commits a typed property value.
    pub(super) fn commit_material_prop(&mut self, prop: MatProp, text: &str) {
        let trimmed = text.trim();
        if prop.is_colour() {
            let Some(c) = super::picker::parse_hex(trimmed) else {
                self.log(
                    LogLevel::Warn,
                    format!("\u{201c}{trimmed}\u{201d} isn't a colour: try #ff8800"),
                );
                return;
            };
            self.edit_material(|f| match prop {
                MatProp::Color => f.color = c,
                _ => f.emissive = [c[0], c[1], c[2]],
            });
            return;
        }
        let Some(v) = super::eval_number(trimmed) else {
            self.log(
                LogLevel::Warn,
                format!("\u{201c}{trimmed}\u{201d} isn't a number"),
            );
            return;
        };
        self.edit_material(|f| prop.set(f, v));
    }

    /// Keys while the material editor has the keyboard: saving, undo and
    /// closing work on the file's text; everything else is swallowed.
    pub(super) fn material_editor_key(&mut self, key: Key, m: Mods) -> bool {
        match (key, m.ctrl) {
            (Key::Char('s' | 'z' | 'y' | 'w'), true) => self.code.key(key, m),
            (_, true) => false,
            _ => !matches!(key, Key::F3 | Key::F4 | Key::F12),
        }
    }

    // ----- drawing ---------------------------------------------------------

    /// The material editor in the center area.
    pub(super) fn material_doc_view(&mut self, canvas: &mut Canvas, rect: Rect, idx: usize) {
        self.code.check_disk();
        self.drain_code_events();
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::background());
        if self.p.pressed {
            self.code.focused = rect.contains(self.p.x, self.p.y);
        }
        if idx >= self.code.tabs.len() {
            return;
        }
        self.refresh_material(idx);
        let path = self.code.tabs[idx].path.clone();
        let toolbar = Rect::new(rect.x, rect.y, rect.w, TOOLBAR_H);
        let status = Rect::new(rect.x, rect.y + rect.h - STATUS_H, rect.w, STATUS_H);
        let body = Rect::new(
            rect.x,
            toolbar.y + toolbar.h,
            rect.w,
            (status.y - toolbar.y - toolbar.h).max(0.0),
        );
        self.material_slider_drag();
        let parsed = self.mated.parsed.as_ref().map(|(_, f)| f.clone());
        match parsed {
            Some(Ok(_)) => {
                let Some(file) = self.material_editor_file() else {
                    return;
                };
                self.material_toolbar(canvas, toolbar);
                let pw = if body.w < 900.0 { 290.0 } else { 320.0 };
                let props = Rect::new(body.x + body.w - pw, body.y, pw, body.h);
                let area = Rect::new(body.x, body.y, (body.w - pw).max(0.0), body.h);
                self.material_preview_area(canvas, area, &path);
                self.material_props(canvas, props, &path, &file);
                self.material_status(canvas, status, &path, idx);
            }
            Some(Err(e)) => {
                self.mated.preview_rect = None;
                self.material_broken(canvas, body, &e);
                canvas.fill_rect(status.x, status.y, status.w, status.h, color::surface());
            }
            None => {}
        }
    }

    /// Follows a dragged slider; on release its value goes to the file.
    fn material_slider_drag(&mut self) {
        let Some((prop, _)) = self.mated.slider else {
            return;
        };
        if self.raw.down {
            return;
        }
        let file = self.material_editor_file();
        self.mated.slider = None;
        if let Some(v) = file.map(|f| prop.get(&f)) {
            self.edit_material(|f| prop.set(f, v));
        }
    }

    fn material_toolbar(&mut self, canvas: &mut Canvas, r: Rect) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, color::border());
        let y = r.y + 5.0;
        let h = r.h - 10.0;
        let mut x = r.x + 8.0;
        for shape in MaterialShape::ALL {
            let b = Rect::new(x, y, 64.0, h);
            let style = if self.mated.shape == shape {
                ButtonStyle::Active
            } else {
                ButtonStyle::Ghost
            };
            if self.button(canvas, b, shape.label(), style) {
                self.mated.shape = shape;
            }
            x += b.w + 4.0;
        }
        let tb = Rect::new(r.x + r.w - 104.0, y, 96.0, h);
        if self.button(canvas, tb, "Edit as Text", ButtonStyle::Ghost) {
            self.set_material_source(true);
        }
        let hint = "Drag the preview onto an object to use it";
        let hw = canvas.measure_text(layout::SMALL_SIZE, hint);
        if tb.x - hw - 16.0 > x {
            canvas.text(
                tb.x - hw - 12.0,
                r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
                layout::SMALL_SIZE,
                hint,
                color::text_dim(),
            );
        }
    }

    /// The preview: the material on the chosen shape. Dragging it carries
    /// the material file like a drag from the Files panel.
    fn material_preview_area(&mut self, canvas: &mut Canvas, area: Rect, path: &Path) {
        canvas.fill_rect(area.x, area.y, area.w, area.h, color::viewport_bg());
        let side = (area.w.min(area.h) - 48.0).clamp(0.0, 720.0);
        if side < 40.0 {
            self.mated.preview_rect = None;
            return;
        }
        let r = Rect::new(
            area.x + (area.w - side) / 2.0,
            area.y + (area.h - side) / 2.0 - 10.0,
            side,
            side,
        );
        self.mated.preview_rect = Some(r);
        canvas.fill_rounded(
            r.x - 1.0,
            r.y - 1.0,
            r.w + 2.0,
            r.h + 2.0,
            layout::RADIUS,
            color::border(),
        );
        let s = canvas.scale();
        match &self.mated.preview {
            Some((rgba, w, h)) => {
                #[allow(clippy::cast_precision_loss)]
                let (lw, lh) = (*w as f32 / s, *h as f32 / s);
                canvas.fill_rect(r.x, r.y, r.w, r.h, color::background());
                canvas.blit_rgba(r.x + (r.w - lw) / 2.0, r.y + (r.h - lh) / 2.0, *w, *h, rgba);
            }
            None => {
                canvas.fill_rect(r.x, r.y, r.w, r.h, color::background());
                if let Some(file) = self.material_editor_file() {
                    // Without a GPU: the base colour as a flat disc.
                    let c = file.color;
                    canvas.fill_circle(
                        r.x + r.w / 2.0,
                        r.y + r.h / 2.0,
                        r.w * 0.36,
                        rgba([c[0], c[1], c[2], 1.0]),
                    );
                }
                let note = "The live preview needs a GPU";
                let tw = canvas.measure_text(layout::SMALL_SIZE, note);
                canvas.text(
                    r.x + (r.w - tw) / 2.0,
                    r.y + r.h - 24.0,
                    layout::SMALL_SIZE,
                    note,
                    color::text_faint(),
                );
            }
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tw = canvas.measure_text(layout::FONT_SIZE, &name);
        canvas.text(
            r.x + (r.w - tw) / 2.0,
            r.y + r.h + 10.0,
            layout::FONT_SIZE,
            &name,
            color::text_dim(),
        );
        if self.hot(r) {
            self.cursor = CursorHint::Grab;
            self.tip(
                r,
                "Drag onto an object in the Hierarchy or the Inspector to use it",
            );
            if self.p.pressed && self.file_drag.is_none() {
                self.file_drag = Some(super::project_panel::FileDrag {
                    path: path.to_path_buf(),
                    origin: (self.raw.x, self.raw.y),
                    active: false,
                });
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn material_props(&mut self, canvas: &mut Canvas, r: Rect, path: &Path, file: &MaterialFile) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y, 1.0, r.h, color::border());
        let x = r.x + 12.0;
        let w = r.w - 24.0;
        if self.hot(r) && self.p.scroll != 0.0 {
            self.mated.props_scroll =
                (self.mated.props_scroll - self.p.scroll * ROW_H * 3.0).max(0.0);
        }
        let prev = canvas.set_clip(Some((r.x, r.y, r.w, r.h)));
        let clip = self.clip;
        self.clip = (r.y.max(clip.0), (r.y + r.h).min(clip.1));
        let top = r.y + 10.0 - self.mated.props_scroll;
        let mut y = top;

        y = self.section_head(canvas, x, y, "Surface");
        y = self.mat_colour(canvas, x, y, w, "Color", MatProp::Color, file.color);
        y = self.mat_slider(canvas, x, y, w, "Roughness", MatProp::Roughness, file);
        y = self.mat_slider(canvas, x, y, w, "Metallic", MatProp::Metallic, file);

        y = self.section_head(canvas, x, y, "Textures");
        let asset = automata_assets::load_material(&path.to_string_lossy()).ok();
        for (i, slot) in MapSlot::ALL.into_iter().enumerate() {
            let image = asset.as_ref().and_then(|a| a.map(slot).cloned());
            y = self.mat_map_row(canvas, x, y, w, i, file.map(slot), image);
            if slot == MapSlot::Normal && !file.normal_map.trim().is_empty() {
                y = self.mat_slider(canvas, x, y, w, "Strength", MatProp::NormalStrength, file);
            }
        }
        y = self.mat_number(
            canvas,
            x,
            y,
            w,
            "Tiling",
            [MatProp::TilingX, MatProp::TilingY],
            file,
        );
        y = self.mat_number(
            canvas,
            x,
            y,
            w,
            "Offset",
            [MatProp::OffsetX, MatProp::OffsetY],
            file,
        );
        let (ny, clicked) = self.prop_check(canvas, x, y, w, "Pixelated", file.pixelated);
        y = ny;
        if clicked {
            self.edit_material(|f| f.pixelated = !f.pixelated);
        }

        y = self.section_head(canvas, x, y, "Glow");
        let e = file.emissive;
        y = self.mat_colour(
            canvas,
            x,
            y,
            w,
            "Emission",
            MatProp::Emissive,
            [e[0], e[1], e[2], 1.0],
        );
        y = self.mat_slider(canvas, x, y, w, "Strength", MatProp::EmissiveStrength, file);
        y = self.mat_slider(canvas, x, y, w, "Glow Light", MatProp::GlowLight, file);
        let (ny, clicked) = self.prop_check(canvas, x, y, w, "Glow Shadows", file.glow_shadows);
        y = ny;
        if clicked {
            self.edit_material(|f| f.glow_shadows = !f.glow_shadows);
        }

        y = self.section_head(canvas, x, y, "Shader");
        y = self.mat_shader_row(canvas, x, y, w, path, file);
        if !file.shader.trim().is_empty() {
            for (i, label) in ["p0.x  y", "p0.z  w", "p1.x  y", "p1.z  w"]
                .into_iter()
                .enumerate()
            {
                #[allow(clippy::cast_possible_truncation)]
                let first = (i * 2) as u8;
                y = self.mat_number(
                    canvas,
                    x,
                    y,
                    w,
                    label,
                    [MatProp::ShaderParam(first), MatProp::ShaderParam(first + 1)],
                    file,
                );
            }
        }

        // Problems loading maps and the shader, under everything.
        if let Some(a) = asset.as_ref().filter(|a| !a.problems.is_empty()) {
            y += 8.0;
            for p in a.problems.iter().take(4) {
                canvas.text_clipped(x, y, layout::SMALL_SIZE, p, color::warn(), w);
                y += 18.0;
            }
        }
        let content = y - top + 20.0;
        let max = (content - r.h).max(0.0);
        self.mated.props_scroll = self.mated.props_scroll.min(max);
        self.clip = clip;
        canvas.restore_clip(prev);
    }

    #[allow(clippy::too_many_arguments)]
    fn mat_colour(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        prop: MatProp,
        c: [f32; 4],
    ) -> f32 {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let sw = Rect::new(x + LABEL_W, y + 2.0, ROW_H - 8.0, ROW_H - 8.0);
        canvas.fill_rounded(sw.x, sw.y, sw.w, sw.h, 3.0, rgba(c));
        canvas.stroke_rounded(sw.x, sw.y, sw.w, sw.h, 3.0, color::border(), 1.0);
        let field = Rect::new(sw.x + sw.w + 6.0, y, w - LABEL_W - sw.w - 6.0, ROW_H - 4.0);
        self.field_box(canvas, field, EditTarget::MatProp(prop), false, &hex(c), "");
        y + ROW_H
    }

    /// A 0–1-ish property: a slider and a field for exact values.
    #[allow(clippy::too_many_arguments)]
    fn mat_slider(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        prop: MatProp,
        file: &MaterialFile,
    ) -> f32 {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let fw = 56.0;
        let track = Rect::new(x + LABEL_W, y, w - LABEL_W - fw - 8.0, ROW_H - 4.0);
        let field = Rect::new(track.x + track.w + 8.0, y, fw, ROW_H - 4.0);
        let v = prop.get(file);
        let (lo, hi) = prop.range();
        let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
        let cy = track.y + track.h / 2.0;
        let hot = self.hot(track);
        let dragging = self.mated.slider.is_some_and(|(p, _)| p == prop);
        self.mated.slider_tracks.retain(|(p, _)| *p != prop);
        self.mated.slider_tracks.push((prop, track));
        canvas.fill_rounded(track.x, cy - 2.0, track.w, 4.0, 2.0, color::field());
        canvas.fill_rounded(
            track.x,
            cy - 2.0,
            track.w * t,
            4.0,
            2.0,
            color::accent_soft(),
        );
        let knob = if hot || dragging { 7.0 } else { 6.0 };
        canvas.fill_circle(track.x + track.w * t, cy, knob, color::text());
        if hot {
            self.cursor = CursorHint::Pointer;
            if self.p.pressed {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
                self.mated.slider = Some((prop, v));
            }
        }
        if dragging || (hot && self.p.pressed) {
            let t = ((self.raw.x - track.x) / track.w.max(1.0)).clamp(0.0, 1.0);
            let snap = |v: f32| (v * 100.0).round() / 100.0;
            self.mated.slider = Some((prop, snap(lo + t * (hi - lo))));
        }
        self.field_box(canvas, field, EditTarget::MatProp(prop), true, &num(v), "");
        y + ROW_H
    }

    /// Two numbers side by side (tiling, offset).
    #[allow(clippy::too_many_arguments)]
    fn mat_number(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        props: [MatProp; 2],
        file: &MaterialFile,
    ) -> f32 {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
        let fw = (w - LABEL_W - 6.0) / 2.0;
        for (i, prop) in props.into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let field = Rect::new(x + LABEL_W + i as f32 * (fw + 6.0), y, fw, ROW_H - 4.0);
            self.field_box(
                canvas,
                field,
                EditTarget::MatProp(prop),
                true,
                &num(prop.get(file)),
                "",
            );
        }
        y + ROW_H
    }

    /// A texture slot: a thumbnail, the file, Choose… and ×. An image
    /// dragged from the Files panel drops here too.
    #[allow(clippy::too_many_arguments)]
    fn mat_map_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        slot: usize,
        rel: &str,
        image: Option<Arc<automata_assets::Image>>,
    ) -> f32 {
        let h = 46.0;
        let row = Rect::new(x - 4.0, y - 2.0, w + 8.0, h);
        let which = MapSlot::ALL[slot];
        let rel = rel.trim();
        // A dragged image highlights the slot and lands on release.
        let dragged = self
            .file_drag
            .as_ref()
            .filter(|d| d.active && automata_assets::material::is_image_path(&d.path))
            .map(|d| d.path.clone());
        let over = dragged.is_some() && row.contains(self.raw.x, self.raw.y);
        if over {
            canvas.fill_rounded(row.x, row.y, row.w, row.h, 4.0, color::accent_soft());
            if !self.raw.down {
                self.file_drag = None;
                self.set_material_map(slot, dragged.as_deref());
            }
        }
        let pic = Rect::new(x, y, 40.0, 40.0);
        canvas.fill_rounded(pic.x, pic.y, pic.w, pic.h, 3.0, color::field());
        if let Some(img) = image {
            let s = canvas.scale();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let px = (40.0 * s).round() as u32;
            let key = (Arc::as_ptr(&img) as usize, px);
            let thumb = self
                .mated
                .thumbs
                .entry(key)
                .or_insert_with(|| {
                    let t = thumbnail(&img, px);
                    (img.clone(), t)
                })
                .1
                .clone();
            canvas.blit_rgba(pic.x, pic.y, px, px, &thumb);
        }
        canvas.stroke_rounded(pic.x, pic.y, pic.w, pic.h, 3.0, color::border(), 1.0);
        let tx = pic.x + pic.w + 10.0;
        canvas.text(tx, y + 2.0, layout::FONT_SIZE, which.label(), color::text());
        let (shown, c) = if rel.is_empty() {
            ("None".to_string(), color::text_faint())
        } else {
            let name = Path::new(rel)
                .file_name()
                .map_or_else(|| rel.to_string(), |n| n.to_string_lossy().into_owned());
            (name, color::text_dim())
        };
        let bw = 64.0;
        let clear_w = if rel.is_empty() { 0.0 } else { 24.0 };
        let text_w = x + w - tx - bw - clear_w - 10.0;
        canvas.text_clipped(tx, y + 21.0, layout::SMALL_SIZE, &shown, c, text_w);
        let choose = Rect::new(x + w - bw - clear_w, y + 8.0, bw, 24.0);
        if self.button(canvas, choose, "Choose…", ButtonStyle::Normal) {
            self.open_file_dialog(super::menus::DialogKind::ChooseMaterialMap(slot), None);
        }
        if !rel.is_empty() {
            let clear = Rect::new(choose.x + bw + 2.0, y + 8.0, 22.0, 24.0);
            if self.button(canvas, clear, "×", ButtonStyle::Ghost) {
                self.set_material_map(slot, None);
            }
            self.tip(clear, "Remove the texture");
        }
        if !rel.is_empty() {
            self.tip(Rect::new(tx, y + 18.0, text_w, 18.0), rel);
        }
        y + h + 4.0
    }

    /// The custom shader: the file, Choose…, Edit and ×. A `.wgsl` dragged
    /// from the Files panel drops here too.
    fn mat_shader_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        path: &Path,
        file: &MaterialFile,
    ) -> f32 {
        let rel = file.shader.trim();
        let row = Rect::new(x - 4.0, y - 2.0, w + 8.0, ROW_H);
        let dragged = self
            .file_drag
            .as_ref()
            .filter(|d| d.active && automata_assets::shader::is_shader_path(&d.path))
            .map(|d| d.path.clone());
        if dragged.is_some() && row.contains(self.raw.x, self.raw.y) {
            canvas.fill_rounded(row.x, row.y, row.w, row.h, 4.0, color::accent_soft());
            if !self.raw.down {
                self.file_drag = None;
                self.set_material_shader(dragged.as_deref());
            }
        }
        let full = (!rel.is_empty()).then(|| automata_assets::material::map_path(path, rel));
        // A shader that breaks the engine's rules draws the standard
        // surface; say so here as well as in the Console. (Cached: the
        // file is read again only once it changes.)
        let problem = full.as_ref().filter(|f| f.exists()).and_then(|f| {
            automata_assets::shader::load_shader_file(
                f,
                rel,
                automata_assets::shader::ShaderKind::Material,
            )
            .map_or_else(Some, |a| a.error.as_ref().map(ToString::to_string))
        });
        let (shown, c) = match &full {
            None => (
                "None · the standard surface".to_string(),
                color::text_faint(),
            ),
            Some(_) if problem.is_some() => (format!("{rel} (has errors)"), color::danger()),
            Some(f) if f.exists() => (rel.to_string(), color::accent_soft()),
            Some(_) => (format!("{rel} (missing)"), color::danger()),
        };
        let bw = 64.0;
        let extra = if rel.is_empty() { 0.0 } else { 40.0 + 24.0 };
        let text_w = w - bw - extra - 8.0;
        canvas.text_clipped(x, y + 5.0, layout::FONT_SIZE, &shown, c, text_w);
        if let Some(problem) = &problem {
            self.tip(
                Rect::new(x, y, text_w, ROW_H - 4.0),
                &format!("{problem} · the standard surface is drawn until it's fixed"),
            );
        } else if !rel.is_empty() {
            self.tip(
                Rect::new(x, y, text_w, ROW_H - 4.0),
                "Your WGSL fn material(in: MaterialInput) -> MaterialOutput · \
                 saved changes show at once",
            );
        }
        let choose = Rect::new(x + w - bw - extra, y, bw, ROW_H - 4.0);
        if self.button(canvas, choose, "Choose…", ButtonStyle::Normal) {
            self.open_file_dialog(
                super::menus::DialogKind::ChooseShader(super::menus::ShaderTarget::Material),
                None,
            );
        }
        self.tip(
            choose,
            "Pick a .wgsl shader, or name a new one to start from the template",
        );
        if let Some(f) = full.filter(|_| !rel.is_empty()) {
            let edit = Rect::new(choose.x + bw + 2.0, y, 38.0, ROW_H - 4.0);
            if self.button(canvas, edit, "Edit", ButtonStyle::Ghost) && f.exists() {
                self.out.push(Intent::OpenScript(f.clone()));
            }
            self.tip(edit, "Open the shader in the code editor");
            let clear = Rect::new(edit.x + 40.0, y, 22.0, ROW_H - 4.0);
            if self.button(canvas, clear, "×", ButtonStyle::Ghost) {
                self.set_material_shader(None);
            }
            self.tip(clear, "Use the standard surface again");
        }
        y + ROW_H
    }

    fn material_status(&mut self, canvas: &mut Canvas, r: Rect, path: &Path, idx: usize) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y, r.w, 1.0, color::border());
        let rel = scene_path(path);
        let mut text = format!("material \u{201c}{rel}\u{201d} · shared by every object using it");
        if self.code.tabs.get(idx).is_some_and(|t| t.buf.dirty()) {
            text.push_str(" · unsaved, previewing live (Ctrl+S)");
        }
        canvas.text_clipped(
            r.x + 10.0,
            r.y + 4.0,
            layout::SMALL_SIZE,
            &text,
            color::text_dim(),
            r.w - 20.0,
        );
    }

    fn material_broken(&mut self, canvas: &mut Canvas, r: Rect, error: &str) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::viewport_bg());
        let w = (r.w - 80.0).clamp(200.0, 640.0);
        let x = r.x + (r.w - w) / 2.0;
        let mut y = r.y + 60.0;
        canvas.text_in(
            crate::canvas::Face::Medium,
            x,
            y,
            16.0,
            "This material doesn't read",
            color::text(),
        );
        y += 30.0;
        for line in super::code_view::wrap(error, 80).iter().take(8) {
            canvas.text_clipped(x, y, layout::FONT_SIZE, line, color::danger(), w);
            y += 20.0;
        }
        y += 14.0;
        if self.button(
            canvas,
            Rect::new(x, y, 140.0, 30.0),
            "Fix it as Text",
            ButtonStyle::Accent,
        ) {
            self.set_material_source(true);
        }
    }

    /// The bar above a material shown as text.
    pub(super) fn material_source_bar(&mut self, canvas: &mut Canvas, r: Rect) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, color::border());
        canvas.text(
            r.x + 12.0,
            r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
            layout::SMALL_SIZE,
            "The material as text",
            color::text_dim(),
        );
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 128.0, r.y + 4.0, 120.0, r.h - 8.0),
            "Back to the Editor",
            ButtonStyle::Ghost,
        ) {
            self.set_material_source(false);
        }
    }

    /// The Inspector's Material row for a Mesh Renderer: the file (click
    /// to pick another, double-click to edit it), Edit and ×. Returns the
    /// next row's y.
    pub(super) fn material_row(
        &mut self,
        canvas: &mut Canvas,
        doc: &crate::doc::Document,
        x: f32,
        y: f32,
        w: f32,
        id: u64,
        rel: &str,
    ) -> f32 {
        let editing = self.play == super::PlayState::Editing;
        let rel = rel.trim();
        let file = (!rel.is_empty()).then(|| doc.resolve(rel));
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "Material");
            let extra = if rel.is_empty() { 0.0 } else { 40.0 + 26.0 };
            let r = Rect::new(
                x + layout::LABEL_W,
                y,
                w - layout::LABEL_W - extra,
                layout::FIELD_H,
            );
            let hot = editing && self.hot(r);
            canvas.fill_rounded(
                r.x,
                r.y,
                r.w,
                r.h,
                3.0,
                if hot { color::hover() } else { color::field() },
            );
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::border(), 1.0);
            let (text, c) = if rel.is_empty() {
                (
                    "None · drop a .mat.ron here".to_string(),
                    color::text_faint(),
                )
            } else if automata_assets::load_material(rel).is_ok() {
                (rel.to_string(), color::accent_soft())
            } else {
                (rel.to_string(), color::danger())
            };
            canvas.text_clipped(
                r.x + 6.0,
                r.y + 3.0,
                layout::FONT_SIZE,
                &text,
                c,
                r.w - 12.0,
            );
            self.tip(
                r,
                "A material file shared by every object using it · click to choose · \
                 double-click to edit",
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.double_click {
                    if let Some(f) = file.as_ref().filter(|f| f.exists()) {
                        self.out.push(Intent::OpenScript(f.clone()));
                    }
                } else if self.p.pressed {
                    self.open_file_dialog(super::menus::DialogKind::ChooseMaterial(id), None);
                }
            }
            if !rel.is_empty() {
                let style = if editing {
                    ButtonStyle::Ghost
                } else {
                    ButtonStyle::Disabled
                };
                let edit = Rect::new(r.x + r.w + 2.0, y, 38.0, layout::FIELD_H);
                if self.button(canvas, edit, "Edit", style) {
                    if let Some(f) = file.as_ref().filter(|f| f.exists()) {
                        self.out.push(Intent::OpenScript(f.clone()));
                    }
                }
                self.tip(edit, "Edit the material");
                let clear = Rect::new(edit.x + 40.0, y, 24.0, layout::FIELD_H);
                if self.button(canvas, clear, "×", style) {
                    self.out.push(Intent::SetMaterial { id, path: None });
                }
                self.tip(clear, "Use the object's own colour again");
            }
        }
        y + layout::ROW_STEP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    use crate::ui::Pointer;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon-mated-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("materials")).unwrap();
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        dir
    }

    fn frame(ui: &mut Ui, canvas: &mut Canvas, doc: &Document, p: Pointer) -> Vec<Intent> {
        ui.frame(canvas, doc, p, None)
    }

    #[test]
    fn props_parse_and_clamp() {
        let mut f = MaterialFile::default();
        MatProp::Roughness.set(&mut f, 3.0);
        assert!((f.roughness - 1.0).abs() < 1e-6);
        MatProp::TilingY.set(&mut f, 4.0);
        assert!((MatProp::TilingY.get(&f) - 4.0).abs() < 1e-6);
        MatProp::Metallic.set(&mut f, f32::NAN);
        assert!(f.metallic.is_finite());
        let img = automata_assets::Image {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let t = thumbnail(&img, 4);
        assert_eq!(t.len(), 4 * 4 * 4);
        assert_eq!(&t[0..4], &[255, 0, 0, 255], "left half red");
        assert_eq!(&t[12..16], &[0, 0, 255, 255], "right half blue");
        assert_eq!(t[2 * 4 * 4 + 3], 0, "below the image stays clear");
    }

    #[test]
    fn a_material_opens_in_the_editor_and_edits_round_trip_through_the_file() {
        let dir = temp("edit");
        let path = dir.join("materials/brick.mat.ron");
        MaterialFile::default().save(&path).unwrap();
        std::fs::write(dir.join("textures/brick.png"), b"not really").unwrap();
        let mut ui = Ui::new();
        let doc = Document::new();
        let mut canvas = Canvas::new(1400, 900);
        ui.open_code(&path, None);
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        assert!(ui.material_editor_shown());
        assert_eq!(
            ui.material_editor_file(),
            Some(MaterialFile::default()),
            "the file as it reads"
        );
        // Nothing unsaved: nothing to preview.
        assert_eq!(ui.material_tab_previews(), vec![(path.clone(), None)]);

        ui.commit_material_prop(MatProp::Color, "#ff8000");
        ui.commit_material_prop(MatProp::Roughness, "0.25");
        ui.commit_material_prop(MatProp::TilingX, "3");
        ui.commit_material_prop(MatProp::Metallic, "lots");
        ui.set_material_map(0, Some(&dir.join("textures/brick.png")));
        let f = ui.material_editor_file().unwrap();
        assert!((f.color[0] - 1.0).abs() < 1e-6 && (f.color[1] - 0.502).abs() < 0.01);
        assert!((f.roughness - 0.25).abs() < 1e-6);
        assert!((f.tiling[0] - 3.0).abs() < 1e-6);
        assert!((f.metallic - MaterialFile::default().metallic).abs() < 1e-6);
        assert_eq!(f.base_map, "../textures/brick.png");
        // Unsaved edits are what the app previews everywhere.
        let previews = ui.material_tab_previews();
        assert_eq!(previews[0].1.as_ref(), Some(&f));
        // Not an image: refused, with a reason.
        ui.set_material_map(1, Some(&dir.join("level.ron")));
        assert_eq!(ui.material_editor_file().unwrap().normal_map, "");
        // The tab's text is the file's format: saving writes it.
        let text = ui.code.tabs[0].buf.text();
        assert_eq!(MaterialFile::parse(&text).unwrap(), f);
        assert!(text.contains("base_map: \"../textures/brick.png\""));
        // Undo goes back a step through the text.
        ui.code.tabs[0].buf.undo();
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        assert_eq!(ui.material_editor_file().unwrap().base_map, "");
        // Clearing a slot.
        ui.code.tabs[0].buf.redo();
        ui.set_material_map(0, None);
        assert_eq!(ui.material_editor_file().unwrap().base_map, "");
        // The text view and back.
        ui.set_material_source(true);
        assert!(!ui.material_editor_shown());
        ui.set_material_source(false);
        assert!(ui.material_editor_shown());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dragging_a_slider_previews_live_and_commits_once() {
        let dir = temp("slider");
        let path = dir.join("materials/metal.mat.ron");
        MaterialFile::default().save(&path).unwrap();
        let mut ui = Ui::new();
        let doc = Document::new();
        let mut canvas = Canvas::new(1400, 900);
        ui.open_code(&path, None);
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        let track = ui
            .mated
            .slider_tracks
            .iter()
            .find(|(p, _)| *p == MatProp::Metallic)
            .map(|(_, r)| *r)
            .expect("the Metallic slider is drawn");
        let undo_before = ui.code.tabs[0].buf.undo_depth();
        let (x0, y) = (track.x + 2.0, track.y + track.h / 2.0);
        let press = Pointer {
            x: x0,
            y,
            down: true,
            pressed: true,
            ..Pointer::default()
        };
        frame(&mut ui, &mut canvas, &doc, press);
        let hold = |x: f32| Pointer {
            x,
            y,
            down: true,
            ..Pointer::default()
        };
        frame(&mut ui, &mut canvas, &doc, hold(x0 + 60.0));
        frame(&mut ui, &mut canvas, &doc, hold(x0 + 2000.0));
        frame(&mut ui, &mut canvas, &doc, hold(x0 + 2000.0));
        let live = ui.material_editor_file().unwrap();
        assert!((live.metallic - 1.0).abs() < 1e-6, "{}", live.metallic);
        assert_eq!(
            ui.material_tab_previews()[0].1.as_ref().map(|f| f.metallic),
            Some(1.0),
            "the drag shows everywhere before it's written"
        );
        assert_eq!(
            ui.code.tabs[0].buf.undo_depth(),
            undo_before,
            "not yet written"
        );
        frame(
            &mut ui,
            &mut canvas,
            &doc,
            Pointer {
                x: x0 + 2000.0,
                y,
                ..Pointer::default()
            },
        );
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        assert_eq!(
            ui.code.tabs[0].buf.undo_depth(),
            undo_before + 1,
            "one step"
        );
        assert!((ui.material_editor_file().unwrap().metallic - 1.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_material_gets_a_shader_made_from_the_template_and_its_numbers() {
        let dir = temp("shader");
        let path = dir.join("materials/glow.mat.ron");
        MaterialFile::default().save(&path).unwrap();
        let mut ui = Ui::new();
        let doc = Document::new();
        let mut canvas = Canvas::new(1400, 900);
        ui.open_code(&path, None);
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        assert!(ui.material_editor_shown());

        // A post shader isn't a material shader: refused.
        let post = dir.join("shaders/crt.wgsl");
        create_shader(&post, automata_renderer::ShaderKind::Post).unwrap();
        ui.set_material_shader(Some(&post));
        assert_eq!(ui.material_editor_file().unwrap().shader, "");

        // A new name is made from the template, stored relative to the
        // material, and opened.
        let shader = dir.join("shaders/glow.wgsl");
        ui.set_material_shader(Some(&shader));
        let text = std::fs::read_to_string(&shader).unwrap();
        assert_eq!(text, automata_renderer::MATERIAL_TEMPLATE);
        assert!(
            ui.code.tabs.iter().any(|t| t.path == shader),
            "opened to edit"
        );
        // Back on the material's tab, its numbers edit.
        let mat_tab = ui.code.tabs.iter().position(|t| t.path == path).unwrap();
        ui.code.active = Some(mat_tab);
        frame(&mut ui, &mut canvas, &doc, Pointer::default());
        ui.commit_material_prop(MatProp::ShaderParam(2), "0.5");
        ui.commit_material_prop(MatProp::ShaderParam(9), "1");
        let f = ui.material_editor_file().unwrap();
        assert_eq!(f.shader, "../shaders/glow.wgsl");
        assert_eq!(f.shader_params, vec![0.0, 0.0, 0.5]);
        let tab_text = ui.code.tabs[mat_tab].buf.text();
        assert!(
            tab_text.contains("shader: \"../shaders/glow.wgsl\""),
            "{tab_text}"
        );

        // × goes back to the standard surface.
        ui.set_material_shader(None);
        let f = ui.material_editor_file().unwrap();
        assert!(f.shader.is_empty() && f.shader_params.is_empty());
    }
}
