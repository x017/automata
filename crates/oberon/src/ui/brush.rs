//! The foliage brush: paints grass (or a scatter's copies) onto whatever
//! surface is under the cursor in the scene view, or erases it.
//!
//! The Inspector's Grass and Scatter sections switch it on. While it's on
//! and the selected entity has one of them, a left drag in the scene view
//! lays dabs ([`Intent::Brush`]) every so often along the stroke; the app
//! finds the surface under the cursor each frame ([`Ui::set_brush_hit`]),
//! since only it has the scene's meshes. A whole stroke is one undo step.

use glam::Vec3;

use super::{ButtonStyle, Intent, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::{Document, EntityEntry};
use crate::theme::{color, layout};
use crate::view::Projector;

/// Brush sizes offered (radius, metres).
pub const SIZES: [f32; 4] = [0.5, 1.5, 3.0, 6.0];
/// Brush strengths offered.
pub const STRENGTHS: [f32; 3] = [0.25, 0.5, 1.0];

/// The foliage brush's settings and where it's pointing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    /// `Some(false)` paints, `Some(true)` erases, `None` is off.
    pub tool: Option<bool>,
    /// The brush's radius (metres).
    pub radius: f32,
    /// How full it paints (or how much it erases), 0..1.
    pub strength: f32,
    /// The surface point and normal under the cursor (world space), set by
    /// the app each frame while the brush is on.
    pub hit: Option<(Vec3, Vec3)>,
    /// Where the stroke last laid a dab.
    last: Option<Vec3>,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            tool: None,
            radius: SIZES[1],
            strength: 1.0,
            hit: None,
            last: None,
        }
    }
}

/// Whether `e` has something to paint.
fn paintable(e: &EntityEntry) -> bool {
    e.data.grass.is_some() || e.data.scatter.is_some()
}

impl Ui {
    /// The entity the brush paints on, when it's on, the scene is being
    /// edited and the selection has a Grass or Scatter.
    #[must_use]
    pub fn brush_target(&self, doc: &Document) -> Option<u64> {
        self.brush.tool?;
        if self.play != super::PlayState::Editing {
            return None;
        }
        let e = &doc.entities[doc.selected_index()?];
        paintable(e).then_some(e.id)
    }

    /// Sets the surface under the cursor (`None` when there's none).
    pub fn set_brush_hit(&mut self, hit: Option<(Vec3, Vec3)>) {
        self.brush.hit = hit;
    }

    /// The Inspector rows switching the brush on and setting it up.
    pub(super) fn brush_rows(&mut self, canvas: &mut Canvas, x: f32, y: f32, w: f32) -> f32 {
        let mut y = y;
        let r = Rect::new(x, y, w, 26.0);
        if self.row_visible(r) {
            self.row_label(canvas, x, y + 3.0, "Brush");
            let bx = x + layout::LABEL_W;
            let bw = (w - layout::LABEL_W - 6.0) / 2.0;
            for (i, (label, erase, hint)) in [
                ("Paint", false, "Drag in the scene view to paint it on"),
                ("Erase", true, "Drag in the scene view to cut it away"),
            ]
            .into_iter()
            .enumerate()
            {
                #[allow(clippy::cast_precision_loss)]
                let b = Rect::new(bx + i as f32 * (bw + 6.0), y, bw, 26.0);
                let on = self.brush.tool == Some(erase);
                let style = self.editable_style(if on {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Normal
                });
                if self.button(canvas, b, label, style) {
                    self.brush.tool = if on { None } else { Some(erase) };
                }
                self.tip(b, hint);
            }
        }
        y += 32.0;
        y = self.brush_choice(canvas, x, y, w, "Size", &SIZES, true);
        y = self.brush_choice(canvas, x, y, w, "Strength", &STRENGTHS, false);
        y
    }

    /// One row of preset buttons for the brush's size or strength.
    #[allow(clippy::too_many_arguments)]
    fn brush_choice(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        values: &[f32],
        size: bool,
    ) -> f32 {
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let bx = x + layout::LABEL_W;
            #[allow(clippy::cast_precision_loss)]
            let n = values.len() as f32;
            let bw = (w - layout::LABEL_W - 4.0 * (n - 1.0)) / n;
            for (i, &v) in values.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let b = Rect::new(bx + i as f32 * (bw + 4.0), y, bw, layout::FIELD_H);
                let current = if size {
                    self.brush.radius
                } else {
                    self.brush.strength
                };
                let text = if size {
                    format!("{v} m")
                } else {
                    format!("{}%", (v * 100.0).round())
                };
                let style = if (current - v).abs() < 1e-4 {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Normal
                };
                if self.button(canvas, b, &text, style) {
                    if size {
                        self.brush.radius = v;
                    } else {
                        self.brush.strength = v;
                    }
                }
            }
        }
        y + layout::ROW_STEP
    }

    /// A line under a Grass or Scatter section: what it's made of and
    /// what it needs.
    pub(super) fn foliage_note(
        &self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        grass: bool,
    ) -> f32 {
        let strokes = if grass {
            e.data.grass.as_ref().map_or(0, |g| g.strokes.len())
        } else {
            e.data.scatter.as_ref().map_or(0, |s| s.strokes.len())
        };
        let text = if !grass && e.data.mesh_visual.is_none() {
            String::from("Add a Mesh Renderer: its model gets scattered")
        } else if grass {
            format!("{strokes} strokes · sways in the Environment's Wind")
        } else {
            format!("{strokes} strokes · copies aren't solid")
        };
        self.gfx_note(canvas, x, y, w, &text)
    }

    /// Starts a stroke on `id`.
    pub(super) fn brush_start(&mut self, id: u64) {
        self.brush.last = None;
        self.brush_stroke(id);
    }

    /// Lays a dab under the cursor if the stroke has moved far enough
    /// since the last one.
    pub(super) fn brush_stroke(&mut self, id: u64) {
        let (Some((point, normal)), Some(erase)) = (self.brush.hit, self.brush.tool) else {
            return;
        };
        let spacing = self.brush.radius * 0.3;
        if self.brush.last.is_some_and(|l| l.distance(point) < spacing) {
            return;
        }
        self.brush.last = Some(point);
        let strength = if erase {
            -self.brush.strength
        } else {
            self.brush.strength
        };
        self.out.push(Intent::Brush {
            id,
            point: point.to_array(),
            normal: normal.to_array(),
            radius: self.brush.radius,
            strength,
        });
    }

    /// Ends a stroke.
    pub(super) fn brush_end(&mut self) {
        self.brush.last = None;
        self.out.push(Intent::EndGesture);
    }

    /// The brush's outline on the surface under the cursor.
    pub(super) fn draw_brush(&self, canvas: &mut Canvas, proj: &Projector) {
        let (Some((point, normal)), Some(erase)) = (self.brush.hit, self.brush.tool) else {
            return;
        };
        let n = normal.try_normalize().unwrap_or(Vec3::Y);
        let u = n.any_orthonormal_vector();
        let v = n.cross(u);
        let ring = |radius: f32| -> Vec<(f32, f32)> {
            (0..=48)
                .filter_map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let t = i as f32 / 48.0 * std::f32::consts::TAU;
                    // Lifted a little so it isn't hidden in the surface.
                    proj.project(point + n * 0.03 + (u * t.cos() + v * t.sin()) * radius)
                })
                .collect()
        };
        let c = if erase {
            color::danger()
        } else {
            color::accent()
        };
        let outer = ring(self.brush.radius);
        if outer.len() > 2 {
            canvas.polyline(&outer, c, 2.0);
        }
        // The soft edge: full strength inside half the radius.
        let inner = ring(self.brush.radius * 0.5);
        if inner.len() > 2 {
            canvas.polyline(&inner, c, 1.0);
        }
        if let Some((x, y)) = proj.project(point) {
            canvas.fill_circle(x, y, 2.5, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_lay_spaced_dabs_and_erase_with_negative_strength() {
        let mut ui = Ui::new();
        let mut doc = Document::new();
        let id = doc.create(crate::doc::CreateKind::Grass);
        doc.selected = Some(id);
        assert_eq!(ui.brush_target(&doc), None, "off until switched on");
        ui.brush.tool = Some(false);
        assert_eq!(ui.brush_target(&doc), Some(id));
        ui.set_brush_hit(Some((Vec3::ZERO, Vec3::Y)));
        ui.brush_start(id);
        // Too close to the last dab: nothing.
        ui.set_brush_hit(Some((Vec3::new(0.1, 0.0, 0.0), Vec3::Y)));
        ui.brush_stroke(id);
        ui.set_brush_hit(Some((Vec3::new(1.0, 0.0, 0.0), Vec3::Y)));
        ui.brush_stroke(id);
        ui.brush_end();
        let dabs: Vec<_> = ui
            .out
            .iter()
            .filter_map(|i| match i {
                Intent::Brush { strength, .. } => Some(*strength),
                _ => None,
            })
            .collect();
        assert_eq!(dabs, [1.0, 1.0]);
        assert_eq!(ui.out.last(), Some(&Intent::EndGesture));
        ui.out.clear();
        ui.brush.tool = Some(true);
        ui.brush.strength = 0.5;
        ui.brush_start(id);
        assert!(
            matches!(ui.out[0], Intent::Brush { strength, .. } if (strength + 0.5).abs() < 1e-6)
        );
        // Nothing paintable selected: no target.
        let plain = doc.create(crate::doc::CreateKind::Cube);
        doc.selected = Some(plain);
        assert_eq!(ui.brush_target(&doc), None);
    }
}
