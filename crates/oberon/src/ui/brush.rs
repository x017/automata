//! The brush: paints grass (or a scatter's copies) onto whatever surface
//! is under the cursor in the scene view, or erases it; or sculpts and
//! paints a terrain.
//!
//! The Inspector's Grass, Scatter and Terrain sections switch it on. While
//! it's on and the selected entity has what its tool works on, a left drag
//! in the scene view lays dabs ([`Intent::Brush`]) every so often along
//! the stroke; the app finds the surface under the cursor each frame
//! ([`Ui::set_brush_hit`]), since only it has the scene's meshes. A whole
//! stroke is one undo step.

use glam::Vec3;

use super::{ButtonStyle, Intent, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::{BrushTool, Document, EntityEntry};
use crate::theme::{color, layout};
use crate::view::Projector;

/// Brush sizes offered for foliage (radius, metres).
pub const SIZES: [f32; 4] = [0.5, 1.5, 3.0, 6.0];
/// Brush sizes offered for terrains (radius, metres).
pub const TERRAIN_SIZES: [f32; 4] = [2.0, 5.0, 10.0, 20.0];
/// Brush strengths offered.
pub const STRENGTHS: [f32; 3] = [0.25, 0.5, 1.0];

/// The brush's settings and where it's pointing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    /// What a stroke does (`None`: the brush is off).
    pub tool: Option<BrushTool>,
    /// The brush's radius (metres).
    pub radius: f32,
    /// How full it paints (or how much it erases), 0..1.
    pub strength: f32,
    /// The surface point and normal under the cursor (world space), set by
    /// the app each frame while the brush is on.
    pub hit: Option<(Vec3, Vec3)>,
    /// Where the stroke last laid a dab.
    last: Option<Vec3>,
    /// Where the stroke began (what Flatten levels to).
    anchor: Option<Vec3>,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            tool: None,
            radius: SIZES[1],
            strength: 1.0,
            hit: None,
            last: None,
            anchor: None,
        }
    }
}

/// Whether `e` has something for `tool` to work on.
fn paintable(e: &EntityEntry, tool: BrushTool) -> bool {
    if tool.is_terrain() {
        e.data.terrain.is_some()
    } else {
        e.data.grass.is_some() || e.data.scatter.is_some()
    }
}

impl Ui {
    /// The entity the brush works on, when it's on, the scene is being
    /// edited and the selection has what its tool needs (Grass or a
    /// Scatter for foliage, a Terrain for terrain tools).
    #[must_use]
    pub fn brush_target(&self, doc: &Document) -> Option<u64> {
        let tool = self.brush.tool?;
        if self.play != super::PlayState::Editing {
            return None;
        }
        let e = &doc.entities[doc.selected_index()?];
        paintable(e, tool).then_some(e.id)
    }

    /// Picks `tool` (or switches the brush off if it's already picked),
    /// moving the size onto the presets that suit it.
    fn pick_tool(&mut self, tool: BrushTool) {
        if self.brush.tool == Some(tool) {
            self.brush.tool = None;
            return;
        }
        let was_terrain = self.brush.tool.is_some_and(BrushTool::is_terrain);
        let sizes: &[f32] = if tool.is_terrain() {
            &TERRAIN_SIZES
        } else {
            &SIZES
        };
        let switched = self.brush.tool.is_none() || was_terrain != tool.is_terrain();
        if switched && !sizes.iter().any(|s| (s - self.brush.radius).abs() < 1e-4) {
            self.brush.radius = sizes[1];
        }
        self.brush.tool = Some(tool);
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
            for (i, (tool, hint)) in [
                (BrushTool::Paint, "Drag in the scene view to paint it on"),
                (BrushTool::Erase, "Drag in the scene view to cut it away"),
            ]
            .into_iter()
            .enumerate()
            {
                #[allow(clippy::cast_precision_loss)]
                let b = Rect::new(bx + i as f32 * (bw + 6.0), y, bw, 26.0);
                self.tool_button(canvas, b, tool, hint);
            }
        }
        y += 32.0;
        y = self.brush_choice(canvas, x, y, w, "Size", &SIZES, true);
        y = self.brush_choice(canvas, x, y, w, "Strength", &STRENGTHS, false);
        y
    }

    /// One brush tool's button: lit while picked, a click picks it (or
    /// switches the brush off).
    fn tool_button(&mut self, canvas: &mut Canvas, b: Rect, tool: BrushTool, hint: &str) {
        let on = self.brush.tool == Some(tool);
        let style = self.editable_style(if on {
            ButtonStyle::Active
        } else {
            ButtonStyle::Normal
        });
        if self.button(canvas, b, tool.label(), style) {
            self.pick_tool(tool);
        }
        self.tip(b, hint);
    }

    /// A row of brush tool buttons under a label.
    fn tool_row(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        label: &str,
        tools: &[(BrushTool, &str)],
    ) -> f32 {
        let r = Rect::new(x, y, w, 26.0);
        if self.row_visible(r) {
            self.row_label(canvas, x, y + 3.0, label);
            let bx = x + layout::LABEL_W;
            #[allow(clippy::cast_precision_loss)]
            let n = tools.len() as f32;
            let bw = (w - layout::LABEL_W - 4.0 * (n - 1.0)) / n;
            for (i, &(tool, hint)) in tools.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let b = Rect::new(bx + i as f32 * (bw + 4.0), y, bw, 26.0);
                self.tool_button(canvas, b, tool, hint);
            }
        }
        y + 30.0
    }

    /// The Inspector rows for sculpting and painting a terrain.
    pub(super) fn terrain_brush_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let mut y = self.tool_row(
            canvas,
            (x, y, w),
            "Sculpt",
            &[
                (BrushTool::Raise, "Drag to raise the ground: hills"),
                (BrushTool::Lower, "Drag to lower the ground: valleys"),
                (BrushTool::Smooth, "Drag to even out bumps"),
            ],
        );
        y = self.tool_row(
            canvas,
            (x, y, w),
            "",
            &[
                (
                    BrushTool::Flatten,
                    "Drag to level the ground to where the stroke began",
                ),
                (BrushTool::Noise, "Drag to roughen the ground"),
            ],
        );
        y = self.tool_row(
            canvas,
            (x, y, w),
            "Paint",
            &[
                (BrushTool::Layer(0), "Drag to paint layer 1 (grass)"),
                (BrushTool::Layer(1), "Drag to paint layer 2 (rock)"),
                (BrushTool::Layer(2), "Drag to paint layer 3 (sand)"),
                (BrushTool::Layer(3), "Drag to paint layer 4 (dirt)"),
            ],
        );
        y = self.brush_choice(canvas, x, y, w, "Size", &TERRAIN_SIZES, true);
        self.brush_choice(canvas, x, y, w, "Strength", &STRENGTHS, false)
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

    /// The Terrain section: brushes, shape settings, layers, and the Auto
    /// Paint and heightmap Import buttons.
    pub(super) fn terrain_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        use super::picker::ColorTarget;
        use crate::fields::{Choice, Field, Flag};
        let mut y = self.terrain_brush_rows(canvas, x, y, w);
        y = self.choice_row(canvas, x, y, w, e, "Resolution", Choice::TerrainResolution);
        y = self.float_row(canvas, x, y, w, e, "Size", Field::TerrainSize);
        y = self.flag_row(canvas, x, y, w, e, "Solid", Flag::TerrainCollide);
        y = self.float_row(canvas, x, y, w, e, "Roughness", Field::TerrainRoughness);
        for layer in 0..4u8 {
            let name = BrushTool::Layer(layer).label();
            y = self.color_row(canvas, x, y, w, e, name, ColorTarget::TerrainLayer(layer));
            y = self.float_row(
                canvas,
                x,
                y,
                w,
                e,
                &format!("{name} Tile"),
                Field::TerrainTile(layer),
            );
        }
        y += 4.0;
        let half = (w - 6.0) * 0.5;
        let auto = Rect::new(x, y, half, 26.0);
        let import = Rect::new(x + half + 6.0, y, half, 26.0);
        if self.row_visible(auto) {
            let style = self.editable_style(ButtonStyle::Normal);
            self.tip(
                auto,
                "Paint by shape: sand along the Water, rock on cliffs, dirt on steep \
                 slopes, grass elsewhere",
            );
            if self.button(canvas, auto, "Auto Paint", style) && style != ButtonStyle::Disabled {
                self.out.push(Intent::AutoPaintTerrain(e.id));
            }
            self.tip(
                import,
                "Replace the heights with a greyscale image: black is the bottom, white a \
                 quarter of the size up",
            );
            if self.button(canvas, import, "Import Heightmap", style)
                && style != ButtonStyle::Disabled
            {
                self.open_file_dialog(super::menus::DialogKind::ImportHeightmap(e.id), None);
            }
        }
        y += 32.0;
        if let Some(t) = &e.data.terrain {
            let (lo, hi) = t.height_range();
            let r = t.resolution();
            let text = format!(
                "{r} × {r} heights, {:.2} m apart · {lo:.1} to {hi:.1} m",
                t.spacing()
            );
            y = self.gfx_note(canvas, x, y, w, &text);
        }
        y + 6.0
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
        self.brush.anchor = self.brush.hit.map(|(p, _)| p);
        self.brush_stroke(id);
    }

    /// Lays a dab under the cursor if the stroke has moved far enough
    /// since the last one.
    pub(super) fn brush_stroke(&mut self, id: u64) {
        let (Some((point, normal)), Some(tool)) = (self.brush.hit, self.brush.tool) else {
            return;
        };
        let spacing = self.brush.radius * if tool.is_terrain() { 0.2 } else { 0.3 };
        if self.brush.last.is_some_and(|l| l.distance(point) < spacing) {
            return;
        }
        self.brush.last = Some(point);
        self.out.push(Intent::Brush {
            id,
            tool,
            point: point.to_array(),
            normal: normal.to_array(),
            radius: self.brush.radius,
            strength: self.brush.strength,
            anchor: self.brush.anchor.unwrap_or(point).to_array(),
        });
    }

    /// Ends a stroke.
    pub(super) fn brush_end(&mut self) {
        self.brush.last = None;
        self.brush.anchor = None;
        self.out.push(Intent::EndGesture);
    }

    /// The brush's outline on the surface under the cursor.
    pub(super) fn draw_brush(&self, canvas: &mut Canvas, proj: &Projector) {
        let (Some((point, normal)), Some(tool)) = (self.brush.hit, self.brush.tool) else {
            return;
        };
        let erase = matches!(tool, BrushTool::Erase | BrushTool::Lower);
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
        ui.brush.tool = Some(BrushTool::Paint);
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
        ui.brush.tool = Some(BrushTool::Erase);
        ui.brush.strength = 0.5;
        ui.brush_start(id);
        assert!(matches!(
            ui.out[0],
            Intent::Brush { tool: BrushTool::Erase, strength, .. } if (strength - 0.5).abs() < 1e-6
        ));
        // Nothing paintable selected: no target.
        let plain = doc.create(crate::doc::CreateKind::Cube);
        doc.selected = Some(plain);
        assert_eq!(ui.brush_target(&doc), None);
    }

    #[test]
    fn terrain_tools_need_a_terrain_and_flatten_remembers_where_it_began() {
        let mut ui = Ui::new();
        let mut doc = Document::new();
        let grass = doc.create(crate::doc::CreateKind::Grass);
        let ground = doc.create(crate::doc::CreateKind::Terrain);
        doc.selected = Some(grass);
        ui.pick_tool(BrushTool::Raise);
        assert!(
            (ui.brush.radius - TERRAIN_SIZES[1]).abs() < 1e-6,
            "terrain-sized"
        );
        assert_eq!(ui.brush_target(&doc), None, "grass has no terrain");
        doc.selected = Some(ground);
        assert_eq!(ui.brush_target(&doc), Some(ground));
        ui.pick_tool(BrushTool::Paint);
        assert_eq!(ui.brush_target(&doc), None, "the terrain has no grass");
        assert!((ui.brush.radius - SIZES[1]).abs() < 1e-6);
        ui.pick_tool(BrushTool::Paint);
        assert_eq!(ui.brush.tool, None, "picking it again switches it off");

        ui.pick_tool(BrushTool::Flatten);
        ui.set_brush_hit(Some((Vec3::new(0.0, 2.0, 0.0), Vec3::Y)));
        ui.brush_start(ground);
        ui.set_brush_hit(Some((Vec3::new(4.0, 5.0, 0.0), Vec3::Y)));
        ui.brush_stroke(ground);
        let anchors: Vec<[f32; 3]> = ui
            .out
            .iter()
            .filter_map(|i| match i {
                Intent::Brush { anchor, .. } => Some(*anchor),
                _ => None,
            })
            .collect();
        assert_eq!(anchors, [[0.0, 2.0, 0.0], [0.0, 2.0, 0.0]]);
    }
}
