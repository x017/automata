//! 2D bindings: `at.Sprite`, `at.SpriteClip`, `at.Tilemap`, `at.AutoTile`
//! and `at.CameraFollow2D`.
//!
//! ```python
//! hero = at.Sprite("art/hero.png", size=(1, 1), columns=4, rows=2,
//!                  clips=[at.SpriteClip("idle", 0, 1, fps=4), at.SpriteClip("run", 4, 7)],
//!                  play="idle")
//! ```

use automata_scene::{AutoTile, CameraFollow2D, Sprite2D, SpriteClip, Tilemap, MAX_TILEMAP_SIDE};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

type Rgba = (f32, f32, f32, f32);

fn positive(name: &str, v: f32) -> PyResult<f32> {
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(PyValueError::new_err(format!(
            "{name} must be a positive number, not {v}"
        )))
    }
}

fn grid(name: &str, v: u32) -> PyResult<u32> {
    if (1..=MAX_TILEMAP_SIDE).contains(&v) {
        Ok(v)
    } else {
        Err(PyValueError::new_err(format!(
            "{name} must be from 1 to {MAX_TILEMAP_SIDE}, not {v}"
        )))
    }
}

fn py_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

/// A named run of sprite-sheet frames, ``first`` to ``last`` (inclusive),
/// played at ``fps``; ``looping=False`` holds the last frame.
///
/// ```python
/// at.SpriteClip("run", 4, 9, fps=12)
/// ```
#[pyclass(name = "SpriteClip", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PySpriteClip {
    pub(crate) inner: SpriteClip,
}

#[pymethods]
impl PySpriteClip {
    #[new]
    #[pyo3(signature = (name, first=0, last=None, fps=10.0, looping=true))]
    fn new(name: &str, first: u32, last: Option<u32>, fps: f32, looping: bool) -> PyResult<Self> {
        let last = last.unwrap_or(first);
        if last < first {
            return Err(PyValueError::new_err(format!(
                "last ({last}) comes before first ({first})"
            )));
        }
        Ok(Self {
            inner: SpriteClip {
                name: name.to_owned(),
                first,
                last,
                fps: positive("fps", fps)?,
                looping,
            },
        })
    }

    /// What scripts play it by.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// The first frame.
    #[getter]
    fn first(&self) -> u32 {
        self.inner.first
    }

    /// The last frame (inclusive).
    #[getter]
    fn last(&self) -> u32 {
        self.inner.last
    }

    /// Frames per second.
    #[getter]
    fn fps(&self) -> f32 {
        self.inner.fps
    }

    /// Whether it starts over at the end.
    #[getter]
    fn looping(&self) -> bool {
        self.inner.looping
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let c = &self.inner;
        format!(
            "SpriteClip({:?}, {}, {}, fps={}, looping={})",
            c.name,
            c.first,
            c.last,
            c.fps,
            py_bool(c.looping)
        )
    }
}

/// A 2D image on the entity: centred on its position, turned by its Z
/// rotation, ``size`` world units (times its scale). Empty ``image`` draws
/// a plain ``color`` rectangle.
///
/// A sprite sheet is ``columns × rows`` equal frames (counted left to
/// right, top to bottom); ``frame`` picks one and ``clips`` animate them —
/// ``play`` names the clip that starts on its own, and behaviour scripts
/// switch with ``entity.sprite.play("run")``.
///
/// ``layer`` orders sprites (higher draws on top, then higher Z);
/// ``pixelated`` keeps pixel art crisp. Sprites draw through the scene's
/// ``Camera2D``.
///
/// ```python
/// at.Entity("Hero").add(at.Transform(position=(0, 2, 0))).add(
///     at.Sprite("art/hero.png", columns=4, rows=2, play="idle",
///               clips=[at.SpriteClip("idle", 0, 3, fps=6), at.SpriteClip("run", 4, 7, fps=12)]))
/// ```
#[pyclass(name = "Sprite", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PySprite {
    pub(crate) inner: Sprite2D,
}

#[pymethods]
impl PySprite {
    #[new]
    #[pyo3(signature = (
        image="", size=(1.0, 1.0), color=(1.0, 1.0, 1.0, 1.0), flip_x=false, flip_y=false,
        pixelated=true, layer=0, columns=1, rows=1, frame=0, clips=Vec::new(), play=""
    ))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    #[allow(clippy::fn_params_excessive_bools)]
    #[allow(clippy::needless_pass_by_value)] // pyo3 extracts owned values
    fn new(
        image: &str,
        size: (f32, f32),
        color: Rgba,
        flip_x: bool,
        flip_y: bool,
        pixelated: bool,
        layer: i32,
        columns: u32,
        rows: u32,
        frame: u32,
        clips: Vec<Bound<'_, PyAny>>,
        play: &str,
    ) -> PyResult<Self> {
        let clips = clips
            .iter()
            .map(|c| {
                c.cast::<PySpriteClip>()
                    .map(|c| c.get().inner.clone())
                    .map_err(|_| PyValueError::new_err("clips must be SpriteClip values"))
            })
            .collect::<PyResult<Vec<_>>>()?;
        if !play.is_empty() && !clips.iter().any(|c| c.name == play) {
            return Err(PyValueError::new_err(format!(
                "play={play:?} isn't one of the clips"
            )));
        }
        Ok(Self {
            inner: Sprite2D {
                image: image.to_owned(),
                size: [positive("size", size.0)?, positive("size", size.1)?],
                color: [color.0, color.1, color.2, color.3],
                flip_x,
                flip_y,
                pixelated,
                layer,
                columns: grid("columns", columns)?,
                rows: grid("rows", rows)?,
                frame,
                clips,
                play: play.to_owned(),
                ..Sprite2D::default()
            },
        })
    }

    /// The image path ("" for a plain rectangle).
    #[getter]
    fn image(&self) -> String {
        self.inner.image.clone()
    }

    /// Width and height in world units.
    #[getter]
    fn size(&self) -> (f32, f32) {
        (self.inner.size[0], self.inner.size[1])
    }

    /// The tint (linear RGBA).
    #[getter]
    fn color(&self) -> Rgba {
        let c = self.inner.color;
        (c[0], c[1], c[2], c[3])
    }

    /// Mirrored left to right.
    #[getter]
    fn flip_x(&self) -> bool {
        self.inner.flip_x
    }

    /// Mirrored top to bottom.
    #[getter]
    fn flip_y(&self) -> bool {
        self.inner.flip_y
    }

    /// Hard pixel edges.
    #[getter]
    fn pixelated(&self) -> bool {
        self.inner.pixelated
    }

    /// Drawing order (higher on top).
    #[getter]
    fn layer(&self) -> i32 {
        self.inner.layer
    }

    /// Sheet frames across.
    #[getter]
    fn columns(&self) -> u32 {
        self.inner.columns
    }

    /// Sheet frames down.
    #[getter]
    fn rows(&self) -> u32 {
        self.inner.rows
    }

    /// The frame shown.
    #[getter]
    fn frame(&self) -> u32 {
        self.inner.frame
    }

    /// The clips.
    #[getter]
    fn clips(&self) -> Vec<PySpriteClip> {
        self.inner
            .clips
            .iter()
            .map(|c| PySpriteClip { inner: c.clone() })
            .collect()
    }

    /// The clip that plays from the start ("" for none).
    #[getter]
    fn play(&self) -> String {
        self.inner.play.clone()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let s = &self.inner;
        format!(
            "Sprite({:?}, size=({}, {}), columns={}, rows={}, frame={}, clips={}, play={:?}, layer={})",
            s.image,
            s.size[0],
            s.size[1],
            s.columns,
            s.rows,
            s.frame,
            s.clips.len(),
            s.play,
            s.layer
        )
    }
}

/// 16 tiles of a tileset painted as one: each cell picks its tile from
/// which neighbours (up, right, down, left) hold the same auto-tile. The
/// tiles follow each other from ``first``, ordered by the neighbour mask
/// ``up·1 + right·2 + down·4 + left·8``.
#[pyclass(name = "AutoTile", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyAutoTile {
    inner: AutoTile,
}

#[pymethods]
impl PyAutoTile {
    #[new]
    #[pyo3(signature = (name, first=0))]
    fn new(name: &str, first: u32) -> Self {
        Self {
            inner: AutoTile {
                name: name.to_owned(),
                first,
            },
        }
    }

    /// The brush's name.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// The tileset index of the first of the 16 tiles.
    #[getter]
    fn first(&self) -> u32 {
        self.inner.first
    }

    fn __repr__(&self) -> String {
        format!(
            "AutoTile({:?}, first={})",
            self.inner.name, self.inner.first
        )
    }
}

/// A grid of tiles cut from one ``tileset`` image (``columns × rows``
/// tiles), ``width × height`` cells of ``tile_size`` world units starting
/// at the entity's position (the bottom-left corner) and growing right and
/// up. Solid for 2D physics while ``collide`` is on.
///
/// A cell is 0 (empty), ``n`` (tileset tile ``n - 1``, counting left to
/// right, top to bottom) or ``-n`` (auto-tile ``n - 1``: edges and corners
/// picked from the neighbours). Edits return a changed copy:
///
/// ```python
/// level = at.Tilemap.from_rows(
///     ["..........",
///      "....##....",
///      "##########"],
///     legend={"#": -1}, tileset="art/tiles.png", columns=8, rows=4,
///     autotiles=[at.AutoTile("Ground", first=16)])
/// level = level.set(0, 2, 5)
/// ```
#[pyclass(name = "Tilemap", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyTilemap {
    pub(crate) inner: Tilemap,
}

fn autotiles(list: &[Bound<'_, PyAny>]) -> PyResult<Vec<AutoTile>> {
    list.iter()
        .map(|a| {
            a.cast::<PyAutoTile>()
                .map(|a| a.get().inner.clone())
                .map_err(|_| PyValueError::new_err("autotiles must be AutoTile values"))
        })
        .collect()
}

impl PyTilemap {
    fn edited(&self, f: impl FnOnce(&mut Tilemap)) -> Self {
        let mut inner = self.inner.clone();
        f(&mut inner);
        Self { inner }
    }
}

#[pymethods]
impl PyTilemap {
    #[new]
    #[pyo3(signature = (
        width=16, height=9, tileset="", columns=1, rows=1, tile_size=1.0,
        autotiles=Vec::new(), color=(1.0, 1.0, 1.0, 1.0), pixelated=true, layer=0, collide=true
    ))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    #[allow(clippy::needless_pass_by_value)] // pyo3 extracts owned values
    fn new(
        width: u32,
        height: u32,
        tileset: &str,
        columns: u32,
        rows: u32,
        tile_size: f32,
        autotiles: Vec<Bound<'_, PyAny>>,
        color: Rgba,
        pixelated: bool,
        layer: i32,
        collide: bool,
    ) -> PyResult<Self> {
        let mut inner = Tilemap::new(grid("width", width)?, grid("height", height)?);
        tileset.clone_into(&mut inner.tileset);
        inner.columns = grid("columns", columns)?;
        inner.rows = grid("rows", rows)?;
        inner.tile_size = positive("tile_size", tile_size)?;
        inner.autotiles = self::autotiles(&autotiles)?;
        inner.color = [color.0, color.1, color.2, color.3];
        inner.pixelated = pixelated;
        inner.layer = layer;
        inner.collide = collide;
        Ok(Self { inner })
    }

    /// A map drawn as text, top row first; each character is looked up in
    /// ``legend`` (anything missing is empty). Other keywords as for
    /// ``Tilemap(...)``.
    #[staticmethod]
    #[pyo3(signature = (
        rows_text, legend, tileset="", columns=1, rows=1, tile_size=1.0,
        autotiles=Vec::new(), color=(1.0, 1.0, 1.0, 1.0), pixelated=true, layer=0, collide=true
    ))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    #[allow(clippy::needless_pass_by_value)] // pyo3 extracts owned values
    fn from_rows(
        rows_text: Vec<String>,
        legend: std::collections::HashMap<char, i32>,
        tileset: &str,
        columns: u32,
        rows: u32,
        tile_size: f32,
        autotiles: Vec<Bound<'_, PyAny>>,
        color: Rgba,
        pixelated: bool,
        layer: i32,
        collide: bool,
    ) -> PyResult<Self> {
        let text: Vec<&str> = rows_text.iter().map(String::as_str).collect();
        let legend: Vec<(char, i32)> = legend.into_iter().collect();
        let mut inner = Tilemap::from_rows(&text, &legend);
        tileset.clone_into(&mut inner.tileset);
        inner.columns = grid("columns", columns)?;
        inner.rows = grid("rows", rows)?;
        inner.tile_size = positive("tile_size", tile_size)?;
        inner.autotiles = self::autotiles(&autotiles)?;
        inner.color = [color.0, color.1, color.2, color.3];
        inner.pixelated = pixelated;
        inner.layer = layer;
        inner.collide = collide;
        Ok(Self { inner })
    }

    /// The cell at ``(x, y)`` (0 outside the map).
    fn get(&self, x: i64, y: i64) -> i32 {
        self.inner.get(x, y)
    }

    /// The tileset tile cell ``(x, y)`` shows, auto-tiles worked out
    /// (None when empty).
    fn tile_at(&self, x: i64, y: i64) -> Option<u32> {
        self.inner.tile_at(x, y)
    }

    /// A copy with cell ``(x, y)`` set to ``value``.
    fn set(&self, x: i64, y: i64, value: i32) -> Self {
        self.edited(|m| {
            m.set(x, y, value);
        })
    }

    /// A copy with every cell from ``start`` to ``end`` (inclusive) set.
    fn fill(&self, start: (i64, i64), end: (i64, i64), value: i32) -> Self {
        self.edited(|m| {
            m.fill(start, end, value);
        })
    }

    /// A copy resized, keeping what's painted at the bottom-left.
    fn resize(&self, width: u32, height: u32) -> PyResult<Self> {
        let (w, h) = (grid("width", width)?, grid("height", height)?);
        Ok(self.edited(|m| m.resize(w, h)))
    }

    /// The cell holding the map-local point ``(x, y)``.
    fn cell_at(&self, x: f32, y: f32) -> (i64, i64) {
        self.inner.cell_at(x, y)
    }

    /// Cells across.
    #[getter]
    fn width(&self) -> u32 {
        self.inner.width
    }

    /// Cells up.
    #[getter]
    fn height(&self) -> u32 {
        self.inner.height
    }

    /// The tileset image.
    #[getter]
    fn tileset(&self) -> String {
        self.inner.tileset.clone()
    }

    /// The tileset's tiles across.
    #[getter]
    fn columns(&self) -> u32 {
        self.inner.columns
    }

    /// The tileset's tiles down.
    #[getter]
    fn rows(&self) -> u32 {
        self.inner.rows
    }

    /// A cell's side in world units.
    #[getter]
    fn tile_size(&self) -> f32 {
        self.inner.tile_size
    }

    /// Solid for 2D physics.
    #[getter]
    fn collide(&self) -> bool {
        self.inner.collide
    }

    /// Drawing order among sprites.
    #[getter]
    fn layer(&self) -> i32 {
        self.inner.layer
    }

    /// The auto-tiles.
    #[getter]
    fn autotiles(&self) -> Vec<PyAutoTile> {
        self.inner
            .autotiles
            .iter()
            .map(|a| PyAutoTile { inner: a.clone() })
            .collect()
    }

    /// How many cells aren't empty.
    #[getter]
    fn filled(&self) -> usize {
        self.inner.cells.iter().filter(|c| **c != 0).count()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let m = &self.inner;
        format!(
            "Tilemap({}x{}, tileset={:?}, columns={}, rows={}, tile_size={}, filled={})",
            m.width,
            m.height,
            m.tileset,
            m.columns,
            m.rows,
            m.tile_size,
            self.filled()
        )
    }
}

/// Makes the entity's ``Camera2D`` follow the entity named ``target``:
/// it eases after it (``smoothing`` seconds of lag, 0 to stick), lets it
/// move about a ``dead_zone`` (half sizes) without moving, looks at it
/// plus ``offset``, and keeps the view inside ``bounds`` (``(min_x, min_y,
/// max_x, max_y)``, assuming a 16:9 window) when given.
///
/// ```python
/// at.Entity("Camera").add(at.Transform()).add(at.Camera2D(half_height=6)).add(
///     at.CameraFollow2D("Player", smoothing=0.2, bounds=(0, 0, 120, 30)))
/// ```
#[pyclass(name = "CameraFollow2D", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyCameraFollow2D {
    pub(crate) inner: CameraFollow2D,
}

#[pymethods]
impl PyCameraFollow2D {
    #[new]
    #[pyo3(signature = (target="Player", offset=(0.0, 1.0), smoothing=0.15, dead_zone=(0.5, 0.5), bounds=None))]
    fn new(
        target: &str,
        offset: (f32, f32),
        smoothing: f32,
        dead_zone: (f32, f32),
        bounds: Option<(f32, f32, f32, f32)>,
    ) -> PyResult<Self> {
        if !(smoothing.is_finite() && smoothing >= 0.0) {
            return Err(PyValueError::new_err("smoothing must be 0 or more seconds"));
        }
        let b = bounds.unwrap_or_default();
        Ok(Self {
            inner: CameraFollow2D {
                target: target.to_owned(),
                offset: [offset.0, offset.1],
                smoothing,
                dead_zone: [dead_zone.0.max(0.0), dead_zone.1.max(0.0)],
                bounds: [b.0, b.1, b.2, b.3],
                settled: false,
            },
        })
    }

    /// The followed entity's name.
    #[getter]
    fn target(&self) -> String {
        self.inner.target.clone()
    }

    /// Where the camera looks relative to the target.
    #[getter]
    fn offset(&self) -> (f32, f32) {
        (self.inner.offset[0], self.inner.offset[1])
    }

    /// Seconds of lag.
    #[getter]
    fn smoothing(&self) -> f32 {
        self.inner.smoothing
    }

    /// Half the size of the box the target moves in freely.
    #[getter]
    fn dead_zone(&self) -> (f32, f32) {
        (self.inner.dead_zone[0], self.inner.dead_zone[1])
    }

    /// ``(min_x, min_y, max_x, max_y)`` the view stays inside, or None.
    #[getter]
    fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let b = self.inner.bounds;
        self.inner.bounded().then_some((b[0], b[1], b[2], b[3]))
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "CameraFollow2D({:?}, smoothing={})",
            self.inner.target, self.inner.smoothing
        )
    }
}

/// A sprite's state as the dict behaviour scripts read (see
/// `EntityView.sprite_state`).
pub(crate) fn state_dict<'py>(
    py: Python<'py>,
    s: &Sprite2D,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let d = pyo3::types::PyDict::new(py);
    d.set_item("clip", (!s.playing.is_empty()).then(|| s.playing.clone()))?;
    d.set_item("frame", s.frame)?;
    d.set_item("done", s.finished)?;
    d.set_item(
        "clips",
        s.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
    )?;
    d.set_item("fx", s.flip_x)?;
    d.set_item("fy", s.flip_y)?;
    d.set_item("layer", s.layer)?;
    d.set_item("color", s.color.to_vec())?;
    d.set_item("image", s.image.clone())?;
    d.set_item("size", s.size.to_vec())?;
    Ok(d)
}

/// A sprite edit from `sprite_edit(**changes)`.
pub(crate) fn edit_from(
    changes: Option<&Bound<'_, pyo3::types::PyDict>>,
) -> PyResult<automata_scene::SpriteEdit> {
    let mut edit = automata_scene::SpriteEdit::default();
    let Some(changes) = changes else {
        return Ok(edit);
    };
    for (key, value) in changes.iter() {
        match key.extract::<String>()?.as_str() {
            "play" => edit.play = Some(value.extract()?),
            "stop" => edit.stop = value.extract()?,
            "frame" => edit.frame = Some(value.extract()?),
            "flip_x" => edit.flip_x = Some(value.extract()?),
            "flip_y" => edit.flip_y = Some(value.extract()?),
            "color" => {
                let c: Vec<f32> = value.extract()?;
                edit.color = Some([
                    c.first().copied().unwrap_or(1.0),
                    c.get(1).copied().unwrap_or(1.0),
                    c.get(2).copied().unwrap_or(1.0),
                    c.get(3).copied().unwrap_or(1.0),
                ]);
            }
            "layer" => edit.layer = Some(value.extract()?),
            "image" => edit.image = Some(value.extract()?),
            "size" => {
                let (w, h): (f32, f32) = value.extract()?;
                edit.size = Some([positive("size", w)?, positive("size", h)?]);
            }
            other => {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "sprite_edit() got an unexpected keyword {other:?}"
                )))
            }
        }
    }
    Ok(edit)
}
