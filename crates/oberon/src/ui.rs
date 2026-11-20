//! The Oberon editor UI: an immediate-mode interface rendered into a
//! [`Canvas`] each frame.
//!
//! Layout is fixed (no docking): a toolbar on top, hierarchy left, viewport
//! center, inspector right, console bottom, status bar at the foot. Widgets
//! are drawn and hit-tested against the current pointer state; interactions
//! return intents that the app applies to the document.

use crate::canvas::Canvas;
use crate::doc::Document;
use crate::theme::{color, layout};

/// The pointer's current state for a frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pointer {
    /// X position in pixels.
    pub x: f32,
    /// Y position in pixels.
    pub y: f32,
    /// Whether the primary button is held.
    pub down: bool,
    /// Whether the primary button was pressed this frame.
    pub pressed: bool,
    /// Scroll wheel delta this frame (positive = up/zoom-in).
    pub scroll: f32,
}

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle from origin and size.
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether `(px, py)` is inside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// An action the UI requests the app to perform.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Select an entity.
    Select(u64),
    /// Add a new entity.
    AddEntity,
    /// Delete the selected entity.
    DeleteSelected,
    /// Add a cube mesh to the selected entity.
    AddCube,
    /// Add a light to the selected entity.
    AddLight,
    /// Add a 3D camera to the selected entity.
    AddCamera,
    /// Remove the mesh from the selected entity.
    RemoveMesh,
    /// Remove the light from the selected entity.
    RemoveLight,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Save the document.
    Save,
    /// New empty document.
    New,
    /// Orbit the viewport camera.
    Orbit(f32, f32),
    /// Zoom the viewport camera.
    Zoom(f32),
}

/// The viewport camera (orbit state), shared with the app.
#[derive(Debug, Clone, Copy)]
pub struct ViewCamera {
    /// Orbit yaw in radians.
    pub yaw: f32,
    /// Orbit pitch in radians.
    pub pitch: f32,
    /// Distance from the target.
    pub distance: f32,
}

impl Default for ViewCamera {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.5,
            distance: 9.0,
        }
    }
}

/// The editor UI. Holds transient interaction state (hover, active widget).
pub struct Ui {
    /// The viewport camera.
    pub camera: ViewCamera,
    drag: Option<(f32, f32)>,
    /// The viewport frame to display this frame (RGBA + size), if rendered.
    pub viewport_frame: Option<(Vec<u8>, u32, u32)>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    /// Creates the UI.
    pub fn new() -> Self {
        Self {
            camera: ViewCamera::default(),
            drag: None,
            viewport_frame: None,
        }
    }

    /// Draws the whole editor for this frame, applying interactions.
    ///
    /// `viewport` is the live scene frame to composite (RGBA + size), if the
    /// renderer produced one this frame.
    pub fn frame(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        pointer: Pointer,
        viewport: Option<(&[u8], u32, u32)>,
    ) -> Vec<Intent> {
        let mut intents = Vec::new();
        let (w, h) = canvas.size();
        let (wf, hf) = (w as f32, h as f32);

        canvas.clear(color::background());

        self.toolbar(canvas, doc, pointer, wf, &mut intents);
        let body_y = layout::TOOLBAR_H;
        let body_h = hf - layout::TOOLBAR_H - layout::STATUS_H;
        self.hierarchy(
            canvas,
            doc,
            pointer,
            Rect::new(0.0, body_y, layout::HIERARCHY_W, body_h),
            &mut intents,
        );
        let ins_x = wf - layout::INSPECTOR_W;
        self.inspector(
            canvas,
            doc,
            pointer,
            Rect::new(ins_x, body_y, layout::INSPECTOR_W, body_h),
            &mut intents,
        );
        let view_x = layout::HIERARCHY_W;
        let view_w = ins_x - view_x;
        let view_h = body_h - layout::CONSOLE_H;
        self.viewport(
            canvas,
            pointer,
            Rect::new(view_x, body_y, view_w, view_h),
            viewport,
            &mut intents,
        );
        self.console(
            canvas,
            Rect::new(view_x, body_y + view_h, view_w, layout::CONSOLE_H),
        );
        self.status_bar(canvas, doc, wf, hf);

        intents
    }

    fn panel_title(&self, canvas: &mut Canvas, rect: Rect, title: &str) {
        canvas.fill_rect(rect.x, rect.y, rect.w, layout::TITLEBAR_H, color::surface());
        canvas.fill_rect(rect.x, rect.y, rect.w, 1.0, color::border());
        canvas.text(
            rect.x + layout::PAD,
            rect.y + (layout::TITLEBAR_H - layout::TITLE_SIZE) / 2.0,
            layout::TITLE_SIZE,
            title,
            color::text_dim(),
        );
    }

    fn button(
        &mut self,
        canvas: &mut Canvas,
        rect: Rect,
        label: &str,
        pointer: Pointer,
        accent: bool,
    ) -> bool {
        let hovered = rect.contains(pointer.x, pointer.y);
        let bg = if hovered {
            color::hover()
        } else {
            color::raised()
        };
        canvas.fill_rounded(rect.x, rect.y, rect.w, rect.h, layout::RADIUS, bg);
        canvas.stroke_rounded(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            layout::RADIUS,
            color::border(),
            1.0,
        );
        let text_color = if accent {
            color::accent_soft()
        } else {
            color::text()
        };
        let tw = canvas.measure_text(layout::FONT_SIZE, label);
        canvas.text(
            rect.x + (rect.w - tw) / 2.0,
            rect.y + (rect.h - layout::FONT_SIZE) / 2.0,
            layout::FONT_SIZE,
            label,
            text_color,
        );
        hovered && pointer.pressed
    }

    fn toolbar(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        pointer: Pointer,
        width: f32,
        intents: &mut Vec<Intent>,
    ) {
        let rect = Rect::new(0.0, 0.0, width, layout::TOOLBAR_H);
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, color::border());

        canvas.text(layout::PAD, 13.0, 15.0, "OBERON", color::accent_soft());

        let mut x = layout::PAD + 90.0;
        let btn = Rect::new(0.0, 6.0, 64.0, layout::TOOLBAR_H - 12.0);
        for (label, intent) in [
            ("New", Intent::New),
            ("Save", Intent::Save),
            ("Undo", Intent::Undo),
            ("Redo", Intent::Redo),
        ] {
            let r = Rect::new(x, btn.y, btn.w, btn.h);
            if self.button(canvas, r, label, pointer, false) {
                intents.push(intent);
            }
            x += btn.w + 6.0;
        }

        // Play controls.
        x += 12.0;
        let play = Rect::new(x, btn.y, 70.0, btn.h);
        if self.button(canvas, play, "Play", pointer, true) {
            // Play is handled by the app (runs the sim); surfaced via intent-less log.
        }

        // Document name + dirty marker on the right.
        let name = doc
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "untitled".to_string());
        let label = if doc.dirty() {
            format!("{name} ●")
        } else {
            name
        };
        let tw = canvas.measure_text(layout::FONT_SIZE, &label);
        canvas.text(
            width - tw - layout::PAD,
            13.0,
            layout::FONT_SIZE,
            &label,
            color::text_dim(),
        );
    }

    fn hierarchy(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        pointer: Pointer,
        rect: Rect,
        intents: &mut Vec<Intent>,
    ) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x + rect.w - 1.0, rect.y, 1.0, rect.h, color::border());
        self.panel_title(canvas, rect, "Hierarchy");

        let add = Rect::new(
            rect.x + layout::PAD,
            rect.y + layout::TITLEBAR_H + 6.0,
            rect.w - 2.0 * layout::PAD,
            24.0,
        );
        if self.button(canvas, add, "+ Add Entity", pointer, false) {
            intents.push(Intent::AddEntity);
        }

        let mut y = add.y + add.h + 6.0;
        let row_h = 24.0;
        for entity in &doc.entities {
            let row = Rect::new(rect.x + 4.0, y, rect.w - 8.0, row_h);
            let selected = doc.selected == Some(entity.id);
            let hovered = row.contains(pointer.x, pointer.y);
            if selected {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::select());
            } else if hovered {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::hover());
            }
            canvas.text(
                row.x + 10.0,
                row.y + 5.0,
                layout::FONT_SIZE,
                entity.name(),
                color::text(),
            );
            if hovered && pointer.pressed {
                intents.push(Intent::Select(entity.id));
            }
            y += row_h;
        }
    }

    fn inspector(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        pointer: Pointer,
        rect: Rect,
        intents: &mut Vec<Intent>,
    ) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y, 1.0, rect.h, color::border());
        self.panel_title(canvas, rect, "Inspector");

        let Some(idx) = doc.selected_index() else {
            canvas.text(
                rect.x + layout::PAD,
                rect.y + 40.0,
                layout::FONT_SIZE,
                "Nothing selected",
                color::text_dim(),
            );
            return;
        };
        let entity = &doc.entities[idx];
        let mut y = rect.y + layout::TITLEBAR_H + 8.0;
        let inner_x = rect.x + layout::PAD;
        let inner_w = rect.w - 2.0 * layout::PAD;

        // Name.
        canvas.text(inner_x, y, layout::FONT_SIZE, "Name", color::text_dim());
        canvas.text(
            inner_x + 70.0,
            y,
            layout::FONT_SIZE,
            entity.name(),
            color::text(),
        );
        y += 26.0;

        // Transform.
        if let Some(t) = &entity.data.transform {
            y = self.section(canvas, inner_x, y, inner_w, "Transform");
            y = self.vec3_row(canvas, inner_x, y, inner_w, "Position", t.position);
            y = self.vec3_row(canvas, inner_x, y, inner_w, "Rotation", t.rotation_euler);
            y = self.vec3_row(canvas, inner_x, y, inner_w, "Scale", t.scale);
        }

        // Mesh.
        if entity.data.mesh_visual.is_some() {
            y = self.section(canvas, inner_x, y, inner_w, "Mesh Renderer");
            let rm = Rect::new(inner_x, y, inner_w, 22.0);
            if self.button(canvas, rm, "Remove Mesh", pointer, false) {
                intents.push(Intent::RemoveMesh);
            }
            y += 28.0;
        }

        // Light.
        if let Some(light) = &entity.data.directional_light {
            y = self.section(canvas, inner_x, y, inner_w, "Directional Light");
            canvas.text(
                inner_x,
                y,
                layout::FONT_SIZE,
                &format!("Ambient {:.2}", light.ambient),
                color::text(),
            );
            y += 22.0;
            let rm = Rect::new(inner_x, y, inner_w, 22.0);
            if self.button(canvas, rm, "Remove Light", pointer, false) {
                intents.push(Intent::RemoveLight);
            }
            y += 28.0;
        }

        // Camera.
        if entity.data.camera_3d.is_some() {
            y = self.section(canvas, inner_x, y, inner_w, "Camera 3D");
        }

        // Add-component buttons.
        y += 6.0;
        canvas.text(
            inner_x,
            y,
            layout::FONT_SIZE,
            "Add component:",
            color::text_dim(),
        );
        y += 22.0;
        for (label, intent, present) in [
            ("+ Mesh", Intent::AddCube, entity.data.mesh_visual.is_some()),
            (
                "+ Light",
                Intent::AddLight,
                entity.data.directional_light.is_some(),
            ),
            (
                "+ Camera 3D",
                Intent::AddCamera,
                entity.data.camera_3d.is_some(),
            ),
        ] {
            if !present {
                let b = Rect::new(inner_x, y, inner_w, 22.0);
                if self.button(canvas, b, label, pointer, false) {
                    intents.push(intent);
                }
                y += 26.0;
            }
        }

        // Delete at the bottom.
        let del = Rect::new(inner_x, rect.y + rect.h - 34.0, inner_w, 24.0);
        let hovered = del.contains(pointer.x, pointer.y);
        canvas.fill_rounded(
            del.x,
            del.y,
            del.w,
            del.h,
            layout::RADIUS,
            if hovered {
                color::danger()
            } else {
                color::raised()
            },
        );
        let tw = canvas.measure_text(layout::FONT_SIZE, "Delete Entity");
        canvas.text(
            del.x + (del.w - tw) / 2.0,
            del.y + 5.0,
            layout::FONT_SIZE,
            "Delete Entity",
            color::text(),
        );
        if hovered && pointer.pressed {
            intents.push(Intent::DeleteSelected);
        }
    }

    fn section(&self, canvas: &mut Canvas, x: f32, y: f32, w: f32, title: &str) -> f32 {
        canvas.fill_rounded(x, y, w, 20.0, 3.0, color::raised());
        canvas.text(
            x + 6.0,
            y + 3.0,
            layout::TITLE_SIZE,
            title,
            color::accent_soft(),
        );
        y + 26.0
    }

    fn vec3_row(
        &self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        v: [f32; 3],
    ) -> f32 {
        canvas.text(x, y + 3.0, layout::FONT_SIZE, label, color::text_dim());
        let field_w = (w - 70.0 - 12.0) / 3.0;
        let mut fx = x + 70.0;
        for value in v {
            canvas.fill_rounded(fx, y, field_w, 20.0, 3.0, color::viewport_bg());
            canvas.stroke_rounded(fx, y, field_w, 20.0, 3.0, color::border(), 1.0);
            let text = format!("{value:.2}");
            canvas.text(fx + 5.0, y + 3.0, layout::FONT_SIZE, &text, color::text());
            fx += field_w + 6.0;
        }
        y + 26.0
    }

    fn viewport(
        &mut self,
        canvas: &mut Canvas,
        pointer: Pointer,
        rect: Rect,
        frame: Option<(&[u8], u32, u32)>,
        intents: &mut Vec<Intent>,
    ) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::viewport_bg());
        // Blit the live frame, fit into the panel preserving aspect ratio.
        if let Some((rgba, fw, fh)) = frame {
            let (fw, fh) = (fw.max(1) as f32, fh.max(1) as f32);
            let scale = (rect.w / fw).min(rect.h / fh);
            let (dw, dh) = (fw * scale, fh * scale);
            let (dx, dy) = (rect.x + (rect.w - dw) / 2.0, rect.y + (rect.h - dh) / 2.0);
            canvas.blit_rgba(dx, dy, fw as u32, fh as u32, rgba);
        }

        // Orbit on drag; zoom on scroll.
        if rect.contains(pointer.x, pointer.y) {
            if pointer.scroll != 0.0 {
                intents.push(Intent::Zoom(pointer.scroll));
            }
            if pointer.down {
                if let Some((px, py)) = self.drag {
                    intents.push(Intent::Orbit(pointer.x - px, pointer.y - py));
                }
                self.drag = Some((pointer.x, pointer.y));
            } else {
                self.drag = None;
            }
        } else if !pointer.down {
            self.drag = None;
        }

        canvas.stroke_rect(rect.x, rect.y, rect.w, rect.h, color::border(), 1.0);
    }

    fn console(&self, canvas: &mut Canvas, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y, rect.w, 1.0, color::border());
        self.panel_title(canvas, rect, "Console");
        canvas.text(
            rect.x + layout::PAD,
            rect.y + layout::TITLEBAR_H + 8.0,
            layout::FONT_SIZE,
            "Oberon ready.",
            color::text_dim(),
        );
    }

    fn status_bar(&self, canvas: &mut Canvas, doc: &Document, width: f32, height: f32) {
        let rect = Rect::new(0.0, height - layout::STATUS_H, width, layout::STATUS_H);
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y, rect.w, 1.0, color::border());
        let text = format!("{} entities  ·  editing", doc.entity_count());
        canvas.text(
            rect.x + layout::PAD,
            rect.y + 6.0,
            layout::FONT_SIZE,
            &text,
            color::text_dim(),
        );
    }
}
