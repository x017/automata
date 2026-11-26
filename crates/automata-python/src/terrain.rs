//! `at.Terrain` and `at.TerrainLayer`: a sculpted, painted heightmap ground.

use automata_scene::terrain::layer;
use automata_scene::{Sculpt, Terrain, TerrainLayer, TERRAIN_LAYERS};
use glam::Vec2;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

type Rgb = (f32, f32, f32);

/// One of a terrain's four ground materials: a colour, optionally an image
/// tiled across the ground every ``tile`` metres.
///
/// ```python
/// sand = at.TerrainLayer(color=(0.8, 0.7, 0.5), tile=3)
/// rock = at.TerrainLayer(texture="textures/rock.png", tile=6)
/// ```
#[pyclass(name = "TerrainLayer", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyTerrainLayer {
    inner: TerrainLayer,
}

#[pymethods]
impl PyTerrainLayer {
    /// A layer; ``color`` multiplies ``texture`` (an image path relative to
    /// the asset root, or none for a plain colour with a little grain).
    #[new]
    #[pyo3(signature = (color=(1.0, 1.0, 1.0), texture="", tile=4.0))]
    fn new(color: Rgb, texture: &str, tile: f32) -> Self {
        Self {
            inner: TerrainLayer {
                color: [color.0, color.1, color.2],
                texture: texture.to_owned(),
                tile: tile.max(0.05),
            },
        }
    }

    /// Its colour (linear RGB).
    #[getter]
    fn color(&self) -> Rgb {
        let c = self.inner.color;
        (c[0], c[1], c[2])
    }

    /// The tiled image (empty for none).
    #[getter]
    fn texture(&self) -> String {
        self.inner.texture.clone()
    }

    /// How wide one repeat is (metres).
    #[getter]
    fn tile(&self) -> f32 {
        self.inner.tile
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let c = self.inner.color;
        format!(
            "TerrainLayer(color=({}, {}, {}), texture={:?}, tile={})",
            c[0], c[1], c[2], self.inner.texture, self.inner.tile
        )
    }
}

/// Which layer `which` names: an index 0..3 or ``"grass"``, ``"rock"``,
/// ``"sand"``, ``"dirt"`` (the default layers' order).
fn layer_index(which: &Bound<'_, PyAny>) -> PyResult<usize> {
    if let Ok(i) = which.extract::<usize>() {
        if i < TERRAIN_LAYERS {
            return Ok(i);
        }
    } else if let Ok(name) = which.extract::<String>() {
        match name.to_ascii_lowercase().as_str() {
            "grass" => return Ok(layer::GRASS),
            "rock" => return Ok(layer::ROCK),
            "sand" => return Ok(layer::SAND),
            "dirt" => return Ok(layer::DIRT),
            _ => {}
        }
    }
    Err(PyValueError::new_err(
        "layer must be 0..3 or one of 'grass', 'rock', 'sand', 'dirt'",
    ))
}

fn positive(name: &str, value: f32) -> PyResult<f32> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(PyValueError::new_err(format!(
            "{name} must be a positive number"
        )))
    }
}

/// A heightmap ground: hills, valleys and cliffs over a square ``size``
/// metres a side, centred on the entity, painted with four blended
/// ``layers`` (grass, rock, sand and dirt to start with).
///
/// It's solid when ``collide`` is on (bodies, characters and ray casts
/// meet it), and ``Grass`` and scattered models standing over it follow
/// its surface. Shape it with the same brushes as Oberon's Inspector —
/// each returns a changed copy:
///
/// ```python
/// island = at.Terrain(size=96, resolution=129)
/// island = island.sculpt("raise", (0, 0), radius=30, amount=12)
/// island = island.sculpt("noise", (0, 0), radius=40, amount=2)
/// island = island.auto_paint(water_level=0.0)   # sand by the sea, rock on cliffs
/// scene.add(at.Entity("island").add(at.Transform(position=(0, -2, 0))).add(island))
/// ```
#[pyclass(name = "Terrain", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyTerrain {
    inner: Terrain,
}

#[pymethods]
impl PyTerrain {
    /// A flat terrain ``size`` metres a side with ``resolution`` height
    /// samples a side (3..1025: 129 gives a sample every half metre on a
    /// 64 m terrain).
    #[new]
    #[pyo3(signature = (size=64.0, resolution=129, layers=None, roughness=0.9, collide=true))]
    fn new(
        size: f32,
        resolution: u32,
        layers: Option<Vec<Bound<'_, PyAny>>>,
        roughness: f32,
        collide: bool,
    ) -> PyResult<Self> {
        let mut inner = Terrain::new(positive("size", size)?, resolution);
        if let Some(layers) = layers {
            if layers.len() != TERRAIN_LAYERS {
                return Err(PyValueError::new_err("a terrain has exactly 4 layers"));
            }
            for (slot, l) in inner.layers.iter_mut().zip(layers) {
                *slot = l.cast::<PyTerrainLayer>()?.get().inner.clone();
            }
        }
        inner.roughness = roughness.clamp(0.0, 1.0);
        inner.collide = collide;
        Ok(Self { inner })
    }

    /// A copy with every height set by ``height(x, z)`` (metres, in the
    /// entity's space; ``x`` and ``z`` run from ``-size/2`` to ``size/2``).
    fn shape(&self, height: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        let mut error = None;
        inner.set_heights_with(|x, z| {
            if error.is_some() {
                return 0.0;
            }
            match height.call1((x, z)).and_then(|v| v.extract::<f32>()) {
                Ok(h) if h.is_finite() => h,
                Ok(_) => {
                    error = Some(PyValueError::new_err("heights must be finite numbers"));
                    0.0
                }
                Err(e) => {
                    error = Some(e);
                    0.0
                }
            }
        });
        error.map_or(Ok(Self { inner }), Err)
    }

    /// A copy with one brush dab at ``center`` (``(x, z)`` in the entity's
    /// space), fading out to ``radius`` metres. Tools: ``"raise"`` and
    /// ``"lower"`` (by ``amount`` metres), ``"smooth"`` (``amount`` 0..1),
    /// ``"flatten"`` (``amount`` 0..1 of the way to ``height``, by default
    /// the ground's height at ``center``) and ``"noise"`` (bumps up to
    /// ``amount`` metres).
    #[pyo3(signature = (tool, center, radius=4.0, amount=1.0, height=None))]
    fn sculpt(
        &self,
        tool: &str,
        center: (f32, f32),
        radius: f32,
        amount: f32,
        height: Option<f32>,
    ) -> PyResult<Self> {
        let tool = match tool.to_ascii_lowercase().as_str() {
            "raise" => Sculpt::Raise,
            "lower" => Sculpt::Lower,
            "smooth" => Sculpt::Smooth,
            "flatten" => {
                Sculpt::Flatten(height.unwrap_or_else(|| self.inner.height_at(center.0, center.1)))
            }
            "noise" => Sculpt::Noise,
            _ => {
                return Err(PyValueError::new_err(
                    "tool must be 'raise', 'lower', 'smooth', 'flatten' or 'noise'",
                ))
            }
        };
        let mut inner = self.inner.clone();
        inner.sculpt(
            tool,
            Vec2::new(center.0, center.1),
            positive("radius", radius)?,
            amount,
        );
        Ok(Self { inner })
    }

    /// A copy with ``layer`` (0..3 or ``"grass"``, ``"rock"``, ``"sand"``,
    /// ``"dirt"``) painted in a soft disc at ``center`` (``strength``
    /// 0..1 of the way to all that layer).
    #[pyo3(signature = (layer, center, radius=4.0, strength=1.0))]
    fn paint(
        &self,
        layer: &Bound<'_, PyAny>,
        center: (f32, f32),
        radius: f32,
        strength: f32,
    ) -> PyResult<Self> {
        let index = layer_index(layer)?;
        let mut inner = self.inner.clone();
        inner.paint(
            index,
            Vec2::new(center.0, center.1),
            positive("radius", radius)?,
            strength.clamp(0.0, 1.0),
        );
        Ok(Self { inner })
    }

    /// A copy painted by shape: sand up to ``beach`` metres above
    /// ``water_level``, rock on slopes steeper than ``cliff`` degrees, dirt
    /// on the slopes leading up to them, grass elsewhere (heights in the
    /// entity's space).
    #[pyo3(signature = (water_level=0.0, beach=1.5, cliff=38.0))]
    fn auto_paint(&self, water_level: f32, beach: f32, cliff: f32) -> Self {
        let mut inner = self.inner.clone();
        inner.auto_paint(water_level, beach.max(0.0), cliff.clamp(1.0, 89.0));
        Self { inner }
    }

    /// A copy with heights from a greyscale image (8- or 16-bit PNG and
    /// the like, relative to the asset root): black is ``low`` metres,
    /// white ``high``. The image is resampled to the terrain's resolution.
    #[pyo3(signature = (path, low=0.0, high=10.0))]
    fn load_heightmap(&self, path: &str, low: f32, high: f32) -> PyResult<Self> {
        let bytes = std::fs::read(automata_assets::resolve(path))
            .map_err(|e| PyValueError::new_err(format!("{path}: {e}")))?;
        let mut inner = self.inner.clone();
        inner
            .load_heightmap(&bytes, low, high)
            .map_err(|e| PyValueError::new_err(format!("{path}: {e}")))?;
        Ok(Self { inner })
    }

    /// A copy resampled to ``resolution`` samples a side, keeping its shape.
    fn with_resolution(&self, resolution: u32) -> Self {
        let mut inner = self.inner.clone();
        inner.set_resolution(resolution);
        Self { inner }
    }

    /// The ground's height at ``(x, z)`` (entity space; edges clamp).
    fn height_at(&self, x: f32, z: f32) -> f32 {
        self.inner.height_at(x, z)
    }

    /// The ground's up-facing normal at ``(x, z)``.
    fn normal_at(&self, x: f32, z: f32) -> (f32, f32, f32) {
        self.inner.normal_at(x, z).into()
    }

    /// How much of each layer shows at ``(x, z)``: four weights summing to 1.
    fn weights_at(&self, x: f32, z: f32) -> (f32, f32, f32, f32) {
        self.inner.weights_at(x, z).into()
    }

    /// The side of the square (metres).
    #[getter]
    fn size(&self) -> f32 {
        self.inner.size
    }

    /// Height samples a side.
    #[getter]
    fn resolution(&self) -> u32 {
        self.inner.resolution()
    }

    /// Metres between neighbouring samples.
    #[getter]
    fn spacing(&self) -> f32 {
        self.inner.spacing()
    }

    /// Every height, row by row (rows along Z, from ``-size/2``).
    #[getter]
    fn heights(&self) -> Vec<f32> {
        self.inner.heights().to_vec()
    }

    /// The lowest and highest heights.
    #[getter]
    fn height_range(&self) -> (f32, f32) {
        self.inner.height_range()
    }

    /// The four layers.
    #[getter]
    fn layers(&self) -> Vec<PyTerrainLayer> {
        self.inner
            .layers
            .iter()
            .map(|l| PyTerrainLayer { inner: l.clone() })
            .collect()
    }

    /// Lighting roughness (0 shiny .. 1 matte).
    #[getter]
    fn roughness(&self) -> f32 {
        self.inner.roughness
    }

    /// Whether it's solid.
    #[getter]
    fn collide(&self) -> bool {
        self.inner.collide
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let (lo, hi) = self.inner.height_range();
        format!(
            "Terrain(size={}, resolution={}, heights={lo:.2}..{hi:.2})",
            self.inner.size,
            self.inner.resolution()
        )
    }
}

impl PyTerrain {
    /// The engine component value.
    pub fn inner(&self) -> Terrain {
        self.inner.clone()
    }

    /// Wraps an engine terrain for Python.
    pub fn from_inner(inner: Terrain) -> Self {
        Self { inner }
    }
}
