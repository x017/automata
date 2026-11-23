//! `at.Grass` and `at.Scatter`: fields of grass and scattered models.

use automata_scene::{Dab, Grass, Scatter};
use glam::Vec3;
use pyo3::prelude::*;

type Rgb = (f32, f32, f32);
type Point = (f32, f32, f32);

fn rgb(c: Rgb) -> [f32; 3] {
    [c.0, c.1, c.2]
}

fn tuple(c: [f32; 3]) -> Rgb {
    (c[0], c[1], c[2])
}

/// A brush dab at `position` (the entity's space): paints for positive
/// `strength`, erases for negative.
fn dab(position: Point, radius: f32, normal: Point, strength: f32) -> PyResult<Dab> {
    if !(radius.is_finite() && radius > 0.0) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "radius must be a positive number",
        ));
    }
    let normal = Vec3::new(normal.0, normal.1, normal.2)
        .try_normalize()
        .unwrap_or(Vec3::Y);
    Ok(Dab {
        position: [position.0, position.1, position.2],
        normal: normal.to_array(),
        radius,
        strength: strength.clamp(-1.0, 1.0),
    })
}

/// A field of grass: blades that sway in the ``Environment``'s wind and
/// bend away from characters and moving bodies walking through them.
///
/// Grow it over a rectangle centred on the entity (``area``, metres along
/// its X and Z), paint it on with ``paint`` (a soft brush disc, in the
/// entity's space, lying on a surface facing ``normal``), and cut it out
/// again with ``erase``. Past ``draw_distance`` metres from the camera
/// it's gone, thinning out over the way there.
///
/// ```python
/// meadow = at.Grass(area=(30, 30), height=0.7, tip_color=(0.6, 0.7, 0.2))
/// meadow = meadow.erase((0, 0, 0), radius=3)       # a clearing
/// scene.add(at.Entity("meadow").add(at.Transform()).add(meadow))
/// scene.add(at.Entity("sky").add(at.Environment(sky=True, wind=0.4)))
/// ```
#[pyclass(name = "Grass", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyGrass {
    inner: Grass,
}

#[pymethods]
impl PyGrass {
    /// Creates a field; unspecified settings use the defaults.
    #[new]
    #[pyo3(signature = (
        area=None,
        density=None,
        height=None,
        height_variation=None,
        width=None,
        base_color=None,
        tip_color=None,
        color_variation=None,
        draw_distance=None,
        seed=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        area: Option<(f32, f32)>,
        density: Option<f32>,
        height: Option<f32>,
        height_variation: Option<f32>,
        width: Option<f32>,
        base_color: Option<Rgb>,
        tip_color: Option<Rgb>,
        color_variation: Option<f32>,
        draw_distance: Option<f32>,
        seed: Option<u32>,
    ) -> Self {
        let d = Grass::default();
        Self {
            inner: Grass {
                area: area.map_or(d.area, |(w, l)| [w.max(0.0), l.max(0.0)]),
                density: density.map_or(d.density, |v| v.max(0.0)),
                height: height.map_or(d.height, |v| v.max(0.01)),
                height_variation: height_variation
                    .map_or(d.height_variation, |v| v.clamp(0.0, 1.0)),
                width: width.map_or(d.width, |v| v.max(0.002)),
                base_color: base_color.map_or(d.base_color, rgb),
                tip_color: tip_color.map_or(d.tip_color, rgb),
                color_variation: color_variation.map_or(d.color_variation, |v| v.clamp(0.0, 1.0)),
                draw_distance: draw_distance.map_or(d.draw_distance, |v| v.max(0.0)),
                seed: seed.unwrap_or(d.seed),
                ..d
            },
        }
    }

    /// A copy with grass painted in a soft disc of ``radius`` metres at
    /// ``position`` (the entity's space) on a surface facing ``normal``.
    /// ``strength`` (0..1) is how full it gets.
    #[pyo3(signature = (position, radius=2.0, normal=(0.0, 1.0, 0.0), strength=1.0))]
    fn paint(&self, position: Point, radius: f32, normal: Point, strength: f32) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        inner
            .strokes
            .push(dab(position, radius, normal, strength.clamp(0.0, 1.0))?);
        Ok(Self { inner })
    }

    /// A copy with the grass in a soft disc of ``radius`` metres at
    /// ``position`` cut away (``strength`` 0..1 of it).
    #[pyo3(signature = (position, radius=2.0, strength=1.0))]
    fn erase(&self, position: Point, radius: f32, strength: f32) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        inner.strokes.push(dab(
            position,
            radius,
            (0.0, 1.0, 0.0),
            -strength.clamp(0.0, 1.0),
        )?);
        Ok(Self { inner })
    }

    /// The filled rectangle (metres along X and Z; zero for none).
    #[getter]
    fn area(&self) -> (f32, f32) {
        (self.inner.area[0], self.inner.area[1])
    }

    /// Blades per square metre where fully painted.
    #[getter]
    fn density(&self) -> f32 {
        self.inner.density
    }

    /// The tallest blades (metres).
    #[getter]
    fn height(&self) -> f32 {
        self.inner.height
    }

    /// 0..1: how much shorter some blades are.
    #[getter]
    fn height_variation(&self) -> f32 {
        self.inner.height_variation
    }

    /// Blade width at the root (metres).
    #[getter]
    fn width(&self) -> f32 {
        self.inner.width
    }

    /// Colour at the root (linear RGB).
    #[getter]
    fn base_color(&self) -> Rgb {
        tuple(self.inner.base_color)
    }

    /// Colour at the tip (linear RGB).
    #[getter]
    fn tip_color(&self) -> Rgb {
        tuple(self.inner.tip_color)
    }

    /// 0..1: how much blades' colours differ.
    #[getter]
    fn color_variation(&self) -> f32 {
        self.inner.color_variation
    }

    /// Metres from the camera it's drawn to (0 = any distance).
    #[getter]
    fn draw_distance(&self) -> f32 {
        self.inner.draw_distance
    }

    /// Picks which random field grows.
    #[getter]
    fn seed(&self) -> u32 {
        self.inner.seed
    }

    /// How many brush strokes (paint and erase dabs) it has.
    #[getter]
    fn strokes(&self) -> usize {
        self.inner.strokes.len()
    }

    /// How many blades it grows on an entity at the origin.
    fn blade_count(&self) -> usize {
        self.inner.blade_count(glam::Mat4::IDENTITY)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Grass(area=({}, {}), density={}, height={}, strokes={})",
            self.inner.area[0],
            self.inner.area[1],
            self.inner.density,
            self.inner.height,
            self.inner.strokes.len()
        )
    }
}

impl PyGrass {
    /// The engine component value.
    pub fn inner(&self) -> Grass {
        self.inner.clone()
    }

    /// Wraps an engine grass field for Python.
    pub fn from_inner(inner: Grass) -> Self {
        Self { inner }
    }
}

/// Foliage scatter: draws the entity's model (its ``MeshRenderer``) at many
/// spots on the ground instead of once, for rocks, flowers and bushes.
///
/// ``density`` copies per square metre, over ``area`` and whatever's
/// painted (``paint``/``erase``, as on ``Grass``), each turned at random,
/// scaled between ``scale[0]`` and ``scale[1]`` and tilted ``align`` of
/// the way to the ground's slope. Copies aren't solid.
///
/// ```python
/// scene.add(at.Entity("flowers").add(at.Transform())
///           .add(at.MeshRenderer.model("models/flower.glb"))
///           .add(at.Scatter(area=(20, 20), density=0.5, scale=(0.8, 1.2))))
/// ```
#[pyclass(name = "Scatter", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyScatter {
    inner: Scatter,
}

#[pymethods]
impl PyScatter {
    /// Creates a scatter; unspecified settings use the defaults.
    #[new]
    #[pyo3(signature = (area=None, density=None, scale=None, align=None, seed=None))]
    fn new(
        area: Option<(f32, f32)>,
        density: Option<f32>,
        scale: Option<(f32, f32)>,
        align: Option<f32>,
        seed: Option<u32>,
    ) -> Self {
        let d = Scatter::default();
        Self {
            inner: Scatter {
                area: area.map_or(d.area, |(w, l)| [w.max(0.0), l.max(0.0)]),
                density: density.map_or(d.density, |v| v.max(0.0)),
                scale: scale.map_or(d.scale, |(lo, hi)| [lo.max(0.01), hi.max(lo.max(0.01))]),
                align: align.map_or(d.align, |v| v.clamp(0.0, 1.0)),
                seed: seed.unwrap_or(d.seed),
                ..d
            },
        }
    }

    /// A copy with copies painted in a soft disc (see ``Grass.paint``).
    #[pyo3(signature = (position, radius=2.0, normal=(0.0, 1.0, 0.0), strength=1.0))]
    fn paint(&self, position: Point, radius: f32, normal: Point, strength: f32) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        inner
            .strokes
            .push(dab(position, radius, normal, strength.clamp(0.0, 1.0))?);
        Ok(Self { inner })
    }

    /// A copy with the copies in a soft disc cut away (see
    /// ``Grass.erase``).
    #[pyo3(signature = (position, radius=2.0, strength=1.0))]
    fn erase(&self, position: Point, radius: f32, strength: f32) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        inner.strokes.push(dab(
            position,
            radius,
            (0.0, 1.0, 0.0),
            -strength.clamp(0.0, 1.0),
        )?);
        Ok(Self { inner })
    }

    /// The filled rectangle (metres along X and Z; zero for none).
    #[getter]
    fn area(&self) -> (f32, f32) {
        (self.inner.area[0], self.inner.area[1])
    }

    /// Copies per square metre where fully painted.
    #[getter]
    fn density(&self) -> f32 {
        self.inner.density
    }

    /// The smallest and largest scale.
    #[getter]
    fn scale(&self) -> (f32, f32) {
        (self.inner.scale[0], self.inner.scale[1])
    }

    /// 0 = upright, 1 = following the ground's slope.
    #[getter]
    fn align(&self) -> f32 {
        self.inner.align
    }

    /// Picks where the copies go.
    #[getter]
    fn seed(&self) -> u32 {
        self.inner.seed
    }

    /// How many brush strokes it has.
    #[getter]
    fn strokes(&self) -> usize {
        self.inner.strokes.len()
    }

    /// How many copies it places.
    fn count(&self) -> usize {
        self.inner.placements().len()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Scatter(area=({}, {}), density={}, strokes={})",
            self.inner.area[0],
            self.inner.area[1],
            self.inner.density,
            self.inner.strokes.len()
        )
    }
}

impl PyScatter {
    /// The engine component value.
    pub fn inner(&self) -> Scatter {
        self.inner.clone()
    }

    /// Wraps an engine scatter for Python.
    pub fn from_inner(inner: Scatter) -> Self {
        Self { inner }
    }
}
