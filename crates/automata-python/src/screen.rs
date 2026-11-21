//! `ctx.screen`: the window's size, a simple HUD layer, and cursor lock.
//!
//! HUD calls (`image`, `rect`, `text`) are recorded during the update callback and
//! drawn on top of the 3D scene that frame, in screen pixels with the
//! origin at the top-left (`+y` down). Shapes are positioned by their
//! centre. Draw them every frame you want them visible, like any immediate
//! mode UI.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use automata_renderer::{Renderer, Sprite, TextAlign, TextAtlas, TextStyle, Texture};
use glam::Vec2;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// One recorded HUD shape.
#[derive(Debug, Clone)]
pub enum HudItem {
    /// An image file, drawn centred at `center`.
    Image {
        /// The file.
        path: PathBuf,
        /// Centre in pixels.
        center: (f32, f32),
        /// Size in pixels (`None` = the image's own size).
        size: Option<(f32, f32)>,
        /// Tint.
        color: [f32; 4],
        /// Counter-clockwise rotation in radians.
        rotation: f32,
    },
    /// A solid rectangle, centred at `center`.
    Rect {
        /// Centre in pixels.
        center: (f32, f32),
        /// Size in pixels.
        size: (f32, f32),
        /// Fill colour.
        color: [f32; 4],
    },
    /// A string in the built-in font, anchored at `anchor` (see
    /// [`automata_renderer::text::layout`]).
    Text {
        /// The string (`\n` starts a new line).
        text: String,
        /// Anchor in pixels.
        anchor: (f32, f32),
        /// Size, colour, alignment and effects.
        style: TextStyle,
    },
}

/// What the screen view records during a frame.
#[derive(Debug, Default)]
pub struct ScreenFrame {
    /// HUD shapes, in draw order.
    pub items: Vec<HudItem>,
    /// A cursor-lock change requested this frame.
    pub lock: Option<bool>,
}

/// Shared between the view and the game loop.
pub type ScreenQueue = Rc<RefCell<ScreenFrame>>;

/// The screen: size, HUD drawing and the cursor (`ctx.screen`).
#[pyclass(name = "Screen", module = "automata._core", unsendable)]
#[derive(Debug, Clone)]
pub struct PyScreen {
    size: (u32, u32),
    locked: bool,
    root: Option<PathBuf>,
    queue: ScreenQueue,
}

impl PyScreen {
    /// A view for one frame.
    pub(crate) fn new(
        size: (u32, u32),
        locked: bool,
        root: Option<PathBuf>,
        queue: ScreenQueue,
    ) -> Self {
        Self {
            size,
            locked,
            root,
            queue,
        }
    }
}

fn color4(c: (f32, f32, f32, f32)) -> [f32; 4] {
    [c.0, c.1, c.2, c.3]
}

#[pymethods]
impl PyScreen {
    /// The drawable area in pixels, `(width, height)`.
    #[getter]
    fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The centre of the screen in pixels.
    #[getter]
    fn center(&self) -> (f32, f32) {
        #[allow(clippy::cast_precision_loss)]
        (self.size.0 as f32 / 2.0, self.size.1 as f32 / 2.0)
    }

    /// Whether the cursor is locked (hidden and captured for mouse-look).
    #[getter]
    fn cursor_locked(&self) -> bool {
        self.queue.borrow().lock.unwrap_or(self.locked)
    }

    /// Locks (hides and captures) the cursor, or releases it.
    #[pyo3(signature = (locked=true))]
    fn lock_cursor(&self, locked: bool) {
        self.queue.borrow_mut().lock = Some(locked);
    }

    /// Draws an image file centred at `(x, y)` this frame. `size` defaults
    /// to the image's own; relative paths resolve against the scene folder.
    #[pyo3(signature = (path, x, y, size=None, color=(1.0, 1.0, 1.0, 1.0), rotation=0.0))]
    fn image(
        &self,
        path: &str,
        x: f32,
        y: f32,
        size: Option<(f32, f32)>,
        color: (f32, f32, f32, f32),
        rotation: f32,
    ) {
        let rel = Path::new(path);
        let path = match &self.root {
            Some(root) if rel.is_relative() => path
                .replace('\\', "/")
                .split('/')
                .fold(root.clone(), |p, part| p.join(part)),
            _ => rel.to_path_buf(),
        };
        self.queue.borrow_mut().items.push(HudItem::Image {
            path,
            center: (x, y),
            size,
            color: color4(color),
            rotation,
        });
    }

    /// Draws a solid rectangle centred at `(x, y)` this frame.
    #[pyo3(signature = (x, y, width, height, color=(1.0, 1.0, 1.0, 1.0)))]
    fn rect(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: (f32, f32, f32, f32),
    ) -> PyResult<()> {
        if !(width.is_finite() && height.is_finite()) {
            return Err(PyValueError::new_err("rect size must be finite"));
        }
        self.queue.borrow_mut().items.push(HudItem::Rect {
            center: (x, y),
            size: (width.max(0.0), height.max(0.0)),
            color: color4(color),
        });
        Ok(())
    }

    /// Draws `text` this frame with the built-in font. `size` is the line
    /// height in pixels. `align` says where `x` sits on each line:
    /// `"left"` (its left edge), `"center"` or `"right"`. Lines (split on
    /// `\n`) are centred vertically on `y`. `shadow` and `outline` are
    /// colours, or `None` for no effect.
    #[pyo3(signature = (text, x, y, size=24.0, color=(1.0, 1.0, 1.0, 1.0), align="left", shadow=None, outline=None))]
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: (f32, f32, f32, f32),
        align: &str,
        shadow: Option<(f32, f32, f32, f32)>,
        outline: Option<(f32, f32, f32, f32)>,
    ) -> PyResult<()> {
        let align = TextAlign::parse(align).ok_or_else(|| {
            PyValueError::new_err(format!(
                "align must be \"left\", \"center\" or \"right\", not {align:?}"
            ))
        })?;
        if !size.is_finite() || size <= 0.0 {
            return Err(PyValueError::new_err("text size must be a positive number"));
        }
        self.queue.borrow_mut().items.push(HudItem::Text {
            text: text.to_string(),
            anchor: (x, y),
            style: TextStyle {
                size,
                color: color4(color),
                align,
                shadow: shadow.map(color4),
                outline: outline.map(color4),
            },
        });
        Ok(())
    }

    /// The `(width, height)` in pixels that `text(text, ..., size)` covers.
    #[pyo3(signature = (text, size=24.0))]
    #[allow(clippy::unused_self)] // a method so scripts find it on ctx.screen
    fn measure_text(&self, text: &str, size: f32) -> (f32, f32) {
        automata_renderer::text::measure(text, size)
    }

    fn __repr__(&self) -> String {
        format!("Screen(size={:?})", self.size)
    }
}

/// Image files decoded into textures, by path, and the text atlas.
#[derive(Default)]
pub struct HudTextures {
    loaded: HashMap<PathBuf, Option<Texture>>,
    text: Option<TextAtlas>,
}

impl std::fmt::Debug for HudTextures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HudTextures")
            .field("loaded", &self.loaded.len())
            .field("text", &self.text)
            .finish()
    }
}

impl HudTextures {
    fn get(&mut self, renderer: &Renderer, path: &Path) -> Option<Texture> {
        self.loaded
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let bytes = std::fs::read(path)
                    .map_err(|e| tracing::warn!("HUD image {}: {e}", path.display()))
                    .ok()?;
                renderer
                    .create_texture_from_bytes(&bytes)
                    .map_err(|e| tracing::warn!("HUD image {}: {e}", path.display()))
                    .ok()
            })
            .clone()
    }

    /// Turns recorded HUD items into sprites.
    pub fn sprites(&mut self, renderer: &Renderer, items: &[HudItem]) -> Vec<Sprite> {
        let mut texts = items
            .iter()
            .filter_map(|item| match item {
                HudItem::Text { text, style, .. } => Some((text.as_str(), style.size)),
                _ => None,
            })
            .peekable();
        if texts.peek().is_some() {
            self.text
                .get_or_insert_with(TextAtlas::new)
                .prepare(renderer, texts);
        }
        let mut out = Vec::new();
        for item in items {
            match item {
                HudItem::Text {
                    text,
                    anchor,
                    style,
                } => {
                    if let Some(atlas) = &self.text {
                        out.extend(atlas.sprites(text, anchor.0, anchor.1, style));
                    }
                }
                other => out.extend(self.shape_sprite(renderer, other)),
            }
        }
        out
    }

    /// The sprite for an image or rectangle.
    fn shape_sprite(&mut self, renderer: &Renderer, item: &HudItem) -> Option<Sprite> {
        Some(item).and_then(|item| match item {
            HudItem::Image {
                path,
                center,
                size,
                color,
                rotation,
            } => {
                let texture = self.get(renderer, path)?;
                let mut sprite = Sprite::new(texture)
                    .position(Vec2::new(center.0, center.1))
                    .color(*color)
                    .rotation(*rotation);
                if let Some((w, h)) = size {
                    sprite = sprite.size(Vec2::new(*w, *h));
                }
                Some(sprite)
            }
            HudItem::Rect {
                center,
                size,
                color,
            } => Some(
                Sprite::new(renderer.white_texture())
                    .position(Vec2::new(center.0, center.1))
                    .size(Vec2::new(size.0, size.1))
                    .color(*color),
            ),
            HudItem::Text { .. } => None,
        })
    }
}
