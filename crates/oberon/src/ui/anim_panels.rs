//! The graph editor's tab: a toolbar of layers, the node canvas
//! (`anim_canvas`) between a panel of parameters and layer settings on
//! the left and the selection's settings on the right (with the 1D and 2D
//! blend space editors), and a status line with the graph's problems.

use std::path::Path;

use automata_assets::anim_graph::{AnimGraph, GraphLayer, Motion, ParamKind, State, Transition};

use super::anim_editor::{
    cond_op, cond_value, ops_for, scene_path, AnimAction, AnimDrag, AnimNode, AnimProp, AnimSel,
    COND_OPS, MOTION_KINDS,
};
use super::menus::{MenuItem, UiAction};
use super::ui_editor::{num, LABEL_W, ROW_H, STATUS_H, TOOLBAR_H};
use super::{color, layout, ButtonStyle, CursorHint, EditTarget, Rect, Ui};
use crate::canvas::Canvas;

/// The left panel's width.
const LEFT_W: f32 = 250.0;
/// The right panel's width.
const RIGHT_W: f32 = 300.0;

fn item(label: impl Into<String>, a: AnimAction) -> MenuItem {
    MenuItem::ui(label, "", UiAction::Anim(a), true)
}

/// A parameter's value as the panel shows it.
fn param_text(kind: ParamKind) -> String {
    match kind {
        ParamKind::Float(v) => num(v),
        ParamKind::Int(v) => v.to_string(),
        ParamKind::Bool(v) => if v { "true" } else { "false" }.into(),
        ParamKind::Trigger => String::new(),
    }
}

impl Ui {
    /// The graph editor in the center area.
    pub(super) fn anim_doc_view(&mut self, canvas: &mut Canvas, rect: Rect, idx: usize) {
        self.code.check_disk();
        self.drain_code_events();
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::background());
        if self.p.pressed {
            self.code.focused = rect.contains(self.p.x, self.p.y);
        }
        if idx >= self.code.tabs.len() {
            return;
        }
        self.refresh_anim(idx);
        let path = self.code.tabs[idx].path.clone();
        let toolbar = Rect::new(rect.x, rect.y, rect.w, TOOLBAR_H);
        let status = Rect::new(rect.x, rect.y + rect.h - STATUS_H, rect.w, STATUS_H);
        let body = Rect::new(
            rect.x,
            toolbar.y + toolbar.h,
            rect.w,
            (status.y - toolbar.y - toolbar.h).max(0.0),
        );
        let parsed = self.animed.parsed.as_ref().map(|(_, g)| g.clone());
        match parsed {
            Some(Ok(graph)) => {
                // Narrow: no left panel; the right one shows the parameters
                // and the layer while nothing is selected.
                let side = body.w >= LEFT_W + RIGHT_W + 500.0;
                let (lw, rw) = if side {
                    (LEFT_W, RIGHT_W)
                } else {
                    (0.0, RIGHT_W.min(body.w * 0.45))
                };
                let left = Rect::new(body.x, body.y, lw, body.h);
                let right = Rect::new(body.x + body.w - rw, body.y, rw, body.h);
                let center = Rect::new(body.x + lw, body.y, (body.w - lw - rw).max(0.0), body.h);
                self.anim_canvas(canvas, center, &graph);
                if side {
                    self.anim_left_panel(canvas, left, &graph);
                }
                self.anim_right_panel(canvas, right, &graph, !side);
                self.anim_toolbar(canvas, toolbar, &graph);
                self.anim_status(canvas, status, &path, idx, &graph);
            }
            Some(Err(e)) => {
                self.anim_broken(canvas, body, &e);
                canvas.fill_rect(status.x, status.y, status.w, status.h, color::surface());
            }
            None => {}
        }
    }

    fn anim_toolbar(&mut self, canvas: &mut Canvas, r: Rect, g: &AnimGraph) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, color::border());
        let y = r.y + 5.0;
        let h = r.h - 10.0;
        let mut x = r.x + 8.0;
        canvas.text(
            x,
            r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
            layout::SMALL_SIZE,
            "Layers",
            color::text_dim(),
        );
        x += 48.0;
        for (i, l) in g.layers.iter().enumerate() {
            let w = (canvas.measure_text(layout::FONT_SIZE, &l.name) + 22.0).clamp(54.0, 160.0);
            let style = if i == self.animed.layer {
                ButtonStyle::Active
            } else {
                ButtonStyle::Ghost
            };
            if self.button(canvas, Rect::new(x, y, w, h), &l.name, style) {
                self.anim_action(AnimAction::Layer(i));
            }
            x += w + 4.0;
        }
        let add = Rect::new(x, y, 26.0, h);
        if self.button(canvas, add, "+", ButtonStyle::Ghost) {
            self.anim_action(AnimAction::AddLayer);
        }
        self.tip(
            add,
            "Add a layer: a state machine over the ones below, for the bones in its mask",
        );
        x += 34.0;
        let text_b = Rect::new(r.x + r.w - 104.0, y, 96.0, h);
        if self.button(canvas, text_b, "Edit as Text", ButtonStyle::Ghost) {
            self.set_anim_source(true);
        }
        let frame = Rect::new(text_b.x - 88.0, y, 82.0, h);
        if self.button(canvas, frame, "Frame All", ButtonStyle::Ghost) {
            self.anim_action(AnimAction::FrameAll);
        }
        let (hint, c) = if self.animed.linking.is_some() {
            (
                "Click the state the transition goes to · Esc cancels",
                color::accent(),
            )
        } else {
            (
                "Right-click for commands · drag to move or pan · wheel zooms",
                color::text_faint(),
            )
        };
        let hw = canvas.measure_text(layout::SMALL_SIZE, hint);
        if frame.x - hw - 16.0 > x {
            canvas.text(
                frame.x - hw - 12.0,
                r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
                layout::SMALL_SIZE,
                hint,
                c,
            );
        }
    }

    fn anim_status(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        path: &Path,
        idx: usize,
        g: &AnimGraph,
    ) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y, r.w, 1.0, color::border());
        let clips = self.anim_clips();
        let bones = self.anim_bones();
        let (clips, bones): (Vec<&str>, Vec<&str>) = (
            clips.iter().map(String::as_str).collect(),
            bones.iter().map(String::as_str).collect(),
        );
        let model = self.animed.model.is_some();
        let problems = g.problems(
            model.then_some(clips.as_slice()),
            model.then_some(bones.as_slice()),
        );
        let mut parts = vec![format!("graph \u{201c}{}\u{201d}", scene_path(path))];
        if let Some(live) = &self.animed.live {
            parts.push(format!("live: {}", live.entity));
        }
        match &self.animed.model {
            Some((who, _)) => parts.push(format!("clips and bones from {who}")),
            None => parts.push("no animated model in the scene: type clip names".into()),
        }
        if self.code.tabs.get(idx).is_some_and(|t| t.buf.dirty()) {
            parts.push("unsaved: Play uses it as it is (Ctrl+S)".into());
        }
        let text = parts.join(" · ");
        let tw = canvas.measure_text(layout::SMALL_SIZE, &text);
        canvas.text_clipped(
            r.x + 10.0,
            r.y + 4.0,
            layout::SMALL_SIZE,
            &text,
            color::text_dim(),
            r.w * 0.6,
        );
        let (note, c) = match problems.as_slice() {
            [] => ("No problems".to_string(), color::success()),
            [one] => (one.clone(), color::warn()),
            [first, rest @ ..] => (format!("{first} (and {} more)", rest.len()), color::warn()),
        };
        let x = r.x + 20.0 + tw.min(r.w * 0.6);
        canvas.text_clipped(
            x,
            r.y + 4.0,
            layout::SMALL_SIZE,
            &note,
            c,
            (r.x + r.w - x - 10.0).max(0.0),
        );
        if problems.len() > 1 {
            self.tip(Rect::new(x, r.y, r.x + r.w - x, r.h), &problems.join("\n"));
        }
    }

    fn anim_broken(&mut self, canvas: &mut Canvas, r: Rect, error: &str) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::viewport_bg());
        let w = (r.w - 80.0).clamp(200.0, 640.0);
        let x = r.x + (r.w - w) / 2.0;
        let mut y = r.y + 60.0;
        canvas.text_in(
            crate::canvas::Face::Medium,
            x,
            y,
            16.0,
            "This animation graph doesn't read",
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
            self.set_anim_source(true);
        }
    }

    /// The bar above a graph shown as text.
    pub(super) fn anim_source_bar(&mut self, canvas: &mut Canvas, r: Rect) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        canvas.fill_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, color::border());
        canvas.text(
            r.x + 12.0,
            r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
            layout::SMALL_SIZE,
            "The animation graph as text",
            color::text_dim(),
        );
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 128.0, r.y + 4.0, 120.0, r.h - 8.0),
            "Back to the Graph",
            ButtonStyle::Ghost,
        ) {
            self.set_anim_source(false);
        }
    }

    // ----- shared rows ----------------------------------------------------

    fn anim_label(canvas: &mut Canvas, x: f32, y: f32, label: &str) {
        canvas.text_clipped(
            x,
            y + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            LABEL_W - 6.0,
        );
    }

    /// A labelled text field. Returns the next row's y.
    #[allow(clippy::too_many_arguments)]
    fn anim_field(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        label: &str,
        prop: AnimProp,
        value: &str,
        numeric: bool,
        placeholder: &str,
    ) -> f32 {
        Self::anim_label(canvas, x, y, label);
        let field = Rect::new(x + LABEL_W, y, w - LABEL_W, ROW_H - 4.0);
        self.field_box(
            canvas,
            field,
            EditTarget::AnimProp(prop),
            numeric,
            value,
            placeholder,
        );
        y + ROW_H
    }

    /// A "+" button opening the menu `items()`.
    fn anim_add_menu(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        items: impl FnOnce(&Self) -> Vec<MenuItem>,
    ) {
        if self.button(canvas, r, "+", ButtonStyle::Ghost) {
            if let Some(intent) = self.commit_edit() {
                self.out.push(intent);
            }
            let list = items(self);
            self.open_menu(r.x, r.y + r.h + 2.0, list);
        }
    }

    /// A drop-down: shows `text`; a click opens `items()`.
    fn anim_pick(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        text: &str,
        bad: bool,
        items: impl FnOnce(&Self) -> Vec<MenuItem>,
    ) {
        if !self.row_visible(r) {
            return;
        }
        let hot = self.hot(r);
        canvas.fill_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            layout::RADIUS,
            if hot { color::hover() } else { color::raised() },
        );
        let (shown, c) = if text.is_empty() {
            ("None", color::text_faint())
        } else if bad {
            (text, color::danger())
        } else {
            (text, color::text())
        };
        canvas.text_clipped(
            r.x + 6.0,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            shown,
            c,
            r.w - 22.0,
        );
        let (cx, cy) = (r.x + r.w - 10.0, r.y + r.h / 2.0);
        canvas.fill_polygon(
            &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
            color::text_dim(),
        );
        if hot {
            self.cursor = CursorHint::Pointer;
            if self.p.pressed {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
                let list = items(self);
                self.open_menu(r.x, r.y + r.h + 2.0, list);
            }
        }
    }

    /// Starts a scrolled, clipped panel; returns the first row's y and
    /// what to restore.
    fn anim_panel_begin(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        right: bool,
    ) -> (f32, Option<[i32; 4]>, (f32, f32)) {
        canvas.fill_rect(r.x, r.y, r.w, r.h, color::surface());
        let edge = if right { r.x } else { r.x + r.w - 1.0 };
        canvas.fill_rect(edge, r.y, 1.0, r.h, color::border());
        let scroll = if right {
            &mut self.animed.right_scroll
        } else {
            &mut self.animed.left_scroll
        };
        if r.contains(self.p.x, self.p.y) && self.p.scroll != 0.0 {
            *scroll = (*scroll - self.p.scroll * ROW_H * 3.0).max(0.0);
        }
        let top = r.y + 10.0 - *scroll;
        let prev = canvas.set_clip(Some((r.x, r.y, r.w, r.h)));
        let clip = self.clip;
        self.clip = (r.y.max(clip.0), (r.y + r.h).min(clip.1));
        (top, prev, clip)
    }

    fn anim_panel_end(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        right: bool,
        (top, y): (f32, f32),
        (prev, clip): (Option<[i32; 4]>, (f32, f32)),
    ) {
        let max = (y - top + 20.0 - r.h).max(0.0);
        let scroll = if right {
            &mut self.animed.right_scroll
        } else {
            &mut self.animed.left_scroll
        };
        *scroll = scroll.min(max);
        self.clip = clip;
        canvas.restore_clip(prev);
    }

    // ----- the left panel: parameters and the layer -----------------------

    fn anim_left_panel(&mut self, canvas: &mut Canvas, r: Rect, g: &AnimGraph) {
        let (top, prev, clip) = self.anim_panel_begin(canvas, r, false);
        let (x, w) = (r.x + 12.0, r.w - 24.0);
        let y = self.anim_params_and_layer(canvas, (x, top, w), g);
        self.anim_panel_end(canvas, r, false, (top, y), (prev, clip));
    }

    /// The Parameters and Layer sections. Returns the next row's y.
    fn anim_params_and_layer(
        &mut self,
        canvas: &mut Canvas,
        (x, top, w): (f32, f32, f32),
        g: &AnimGraph,
    ) -> f32 {
        let mut y = self.section_head(canvas, x, top, "Parameters");
        let add = Rect::new(x + w - 24.0, y - 24.0, 24.0, 20.0);
        self.anim_add_menu(canvas, add, |_| {
            ["Float", "Int", "Bool", "Trigger"]
                .iter()
                .zip(0u8..)
                .map(|(k, i)| item(format!("Add {k}"), AnimAction::AddParam(i)))
                .collect()
        });
        self.tip(
            add,
            "Add a parameter: a value scripts set with entity.animation.set or trigger",
        );
        for (i, p) in g.parameters.iter().enumerate() {
            y = self.anim_param_row(canvas, (x, y, w), i, p);
        }
        if g.parameters.is_empty() {
            canvas.text_clipped(
                x,
                y,
                layout::SMALL_SIZE,
                "None yet: + adds one",
                color::text_faint(),
                w,
            );
            y += 20.0;
        }
        self.anim_layer_rows(canvas, (x, y + 8.0, w), g)
    }

    fn anim_param_row(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        i: usize,
        p: &automata_assets::anim_graph::Parameter,
    ) -> f32 {
        let name_w = w * 0.46;
        let field = Rect::new(x, y, name_w, ROW_H - 4.0);
        self.field_box(
            canvas,
            field,
            EditTarget::AnimProp(AnimProp::ParamName(i)),
            false,
            &p.name,
            "name",
        );
        let kx = x + name_w + 6.0;
        canvas.text(
            kx,
            y + 6.0,
            layout::SMALL_SIZE,
            p.kind.label(),
            color::text_faint(),
        );
        let vx = kx + 44.0;
        let vw = (x + w - 26.0 - vx).max(20.0);
        let live = self
            .animed
            .live
            .as_ref()
            .and_then(|l| l.state.params.iter().find(|q| q.name == p.name))
            .map(|q| q.value);
        if let Some(v) = live {
            // During Play: the value the script set, read-only.
            let shown = match p.kind {
                ParamKind::Float(_) => num(v),
                ParamKind::Int(_) => num(v.round()),
                _ => if v >= 0.5 { "on" } else { "off" }.into(),
            };
            canvas.text_clipped(
                vx + 4.0,
                y + 4.0,
                layout::FONT_SIZE,
                &shown,
                color::success(),
                vw,
            );
        } else {
            match p.kind {
                ParamKind::Bool(on) => {
                    let b = Rect::new(vx, y + 3.0, 16.0, 16.0);
                    canvas.fill_rounded(
                        b.x,
                        b.y,
                        b.w,
                        b.h,
                        3.0,
                        if on { color::accent() } else { color::field() },
                    );
                    canvas.stroke_rounded(b.x, b.y, b.w, b.h, 3.0, color::border(), 1.0);
                    if self.hot(b) && self.p.pressed {
                        self.edit_graph(|g| {
                            if let Some(q) = g.parameters.get_mut(i) {
                                q.kind = ParamKind::Bool(!on);
                            }
                        });
                    }
                    self.tip(b, "Its value when the scene starts");
                }
                ParamKind::Trigger => {}
                kind => {
                    let f = Rect::new(vx, y, vw, ROW_H - 4.0);
                    self.field_box(
                        canvas,
                        f,
                        EditTarget::AnimProp(AnimProp::ParamValue(i)),
                        true,
                        &param_text(kind),
                        "",
                    );
                    self.tip(f, "Its value when the scene starts");
                }
            }
        }
        let del = Rect::new(x + w - 22.0, y, 22.0, ROW_H - 4.0);
        if self.button(canvas, del, "×", ButtonStyle::Ghost) {
            self.anim_action(AnimAction::DeleteParam(i));
        }
        self.tip(del, "Remove the parameter and the conditions testing it");
        y + ROW_H
    }

    fn anim_layer_rows(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        g: &AnimGraph,
    ) -> f32 {
        let Some(l) = g.layers.get(self.animed.layer) else {
            return y;
        };
        let first = self.animed.layer == 0;
        let mut y = self.section_head(canvas, x, y, "Layer");
        y = self.anim_field(
            canvas,
            (x, y, w),
            "Name",
            AnimProp::LayerName,
            &l.name,
            false,
            "",
        );
        if first {
            Self::anim_label(canvas, x, y, "Weight");
            canvas.text(
                x + LABEL_W,
                y + 5.0,
                layout::SMALL_SIZE,
                "1 (the first layer shows fully)",
                color::text_faint(),
            );
            y += ROW_H;
            Self::anim_label(canvas, x, y, "Mask");
            canvas.text(
                x + LABEL_W,
                y + 5.0,
                layout::SMALL_SIZE,
                "the whole model",
                color::text_faint(),
            );
            y += ROW_H;
        } else {
            y = self.anim_field(
                canvas,
                (x, y, w),
                "Weight",
                AnimProp::LayerWeight,
                &num(l.weight),
                true,
                "",
            );
            y = self.anim_mask_rows(canvas, (x, y, w), l);
        }
        y += 6.0;
        if g.layers.len() > 1
            && self.button(
                canvas,
                Rect::new(x, y, 110.0, 24.0),
                "Delete Layer",
                ButtonStyle::Danger,
            )
        {
            self.anim_action(AnimAction::DeleteLayer);
        }
        if g.layers.len() > 1 {
            y += 30.0;
        }
        y
    }

    fn anim_mask_rows(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        l: &GraphLayer,
    ) -> f32 {
        Self::anim_label(canvas, x, y, "Mask");
        let bones = self.anim_bones();
        let has_bones = !bones.is_empty();
        let fw = w - LABEL_W - if has_bones { 28.0 } else { 0.0 };
        let field = Rect::new(x + LABEL_W, y, fw, ROW_H - 4.0);
        self.field_box(
            canvas,
            field,
            EditTarget::AnimProp(AnimProp::LayerMask),
            false,
            &l.mask.join(", "),
            "the whole model",
        );
        self.tip(
            field,
            "Bones this layer animates, each with everything below it (comma-separated)",
        );
        if has_bones {
            let pick = Rect::new(field.x + fw + 4.0, y, 24.0, ROW_H - 4.0);
            let mask = l.mask.clone();
            self.anim_add_menu(canvas, pick, move |_| {
                bones
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| !mask.contains(b))
                    .map(|(i, b)| item(b.clone(), AnimAction::AddMaskBone(i)))
                    .collect()
            });
            self.tip(pick, "Add a bone of the model to the mask");
        }
        y + ROW_H
    }

    // ----- the right panel: the selection ---------------------------------

    fn anim_right_panel(&mut self, canvas: &mut Canvas, r: Rect, g: &AnimGraph, narrow: bool) {
        let (top, prev, clip) = self.anim_panel_begin(canvas, r, true);
        let (x, w) = (r.x + 12.0, r.w - 24.0);
        let layer = self.animed.layer;
        let y = match (self.animed.sel, g.layers.get(layer)) {
            (AnimSel::State(i), Some(l)) if i < l.states.len() => {
                self.anim_state_panel(canvas, (x, top, w), g, l, i)
            }
            (AnimSel::Transition(t), Some(l)) if t < l.transitions.len() => {
                self.anim_transition_panel(canvas, (x, top, w), g, l, t)
            }
            (AnimSel::Any, _) => {
                let y = self.section_head(canvas, x, top, "Any State");
                let y = self.anim_note(canvas, (x, y, w), "Transitions from Any State can fire whatever state the layer is in: a jump, a hit, a death.");
                if self.button(
                    canvas,
                    Rect::new(x, y + 4.0, 140.0, 24.0),
                    "Make Transition",
                    ButtonStyle::Normal,
                ) {
                    self.anim_action(AnimAction::Link(AnimNode::Any));
                }
                y + 34.0
            }
            _ if narrow => {
                let y = self.anim_params_and_layer(canvas, (x, top, w), g);
                self.anim_help_panel(canvas, (x, y + 8.0, w))
            }
            _ => self.anim_help_panel(canvas, (x, top, w)),
        };
        self.anim_panel_end(canvas, r, true, (top, y), (prev, clip));
    }

    /// Wrapped small text. Returns the next row's y.
    fn anim_note(&self, canvas: &mut Canvas, (x, y, w): (f32, f32, f32), text: &str) -> f32 {
        let mut y = y;
        for line in super::wrap_words(canvas, text, w) {
            canvas.text(x, y, layout::SMALL_SIZE, &line, color::text_dim());
            y += 17.0;
        }
        y + 4.0
    }

    fn anim_help_panel(&mut self, canvas: &mut Canvas, (x, y, w): (f32, f32, f32)) -> f32 {
        let mut y = self.section_head(canvas, x, y, "Animation graph");
        for text in [
            "States play clips. Transitions pick the next state when their conditions hold.",
            "Right-click the canvas to add a state, and a state to make a transition from it. Double-click the background to add one too.",
            "Scripts never name clips: they set the parameters on the left.",
        ] {
            y = self.anim_note(canvas, (x, y, w), text);
        }
        y += 4.0;
        for line in [
            "anim = entity.animation",
            "anim.set(\"speed\", 3.0)",
            "anim.trigger(\"jump\")",
        ] {
            canvas.text_in(
                crate::canvas::Face::Mono,
                x,
                y,
                layout::SMALL_SIZE,
                line,
                color::text(),
            );
            y += 18.0;
        }
        if let Some(live) = &self.animed.live {
            y += 10.0;
            let text = format!(
                "Live: {} is in {}",
                live.entity,
                live.state.layers.first().map_or("", |l| l.state.as_str())
            );
            canvas.text_clipped(x, y, layout::FONT_SIZE, &text, color::success(), w);
            y += 22.0;
        }
        y
    }

    #[allow(clippy::too_many_lines)]
    fn anim_state_panel(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        g: &AnimGraph,
        l: &GraphLayer,
        i: usize,
    ) -> f32 {
        let s = &l.states[i];
        let mut y = self.section_head(canvas, x, y, "State");
        y = self.anim_field(
            canvas,
            (x, y, w),
            "Name",
            AnimProp::StateName,
            &s.name,
            false,
            "",
        );
        if l.entry_index() == Some(i) {
            Self::anim_label(canvas, x, y, "Entry");
            canvas.text(
                x + LABEL_W,
                y + 5.0,
                layout::SMALL_SIZE,
                "the layer starts here",
                color::warn(),
            );
        } else if self.button(
            canvas,
            Rect::new(x + LABEL_W, y, 120.0, ROW_H - 4.0),
            "Set as Entry",
            ButtonStyle::Normal,
        ) {
            self.anim_action(AnimAction::SetEntry(i));
        }
        y += ROW_H;
        // The motion kind, as a row of buttons.
        Self::anim_label(canvas, x, y, "Motion");
        let kind = super::anim_editor::motion_kind(&s.motion);
        let bw = (w - LABEL_W - 6.0) / 4.0;
        for (k, label) in (0u8..).zip(["None", "Clip", "1D", "2D"]) {
            let b = Rect::new(x + LABEL_W + f32::from(k) * (bw + 2.0), y, bw, ROW_H - 4.0);
            let style = if k == kind {
                ButtonStyle::Active
            } else {
                ButtonStyle::Ghost
            };
            if self.button(canvas, b, label, style) {
                self.anim_action(AnimAction::MotionKind(k));
            }
            self.tip(b, MOTION_KINDS[usize::from(k)]);
        }
        y += ROW_H;
        y = match &s.motion {
            Motion::Empty => self.anim_note(
                canvas,
                (x, y, w),
                if self.animed.layer == 0 {
                    "Plays nothing: the model holds its rest pose."
                } else {
                    "Plays nothing: this layer lets the ones below show through."
                },
            ),
            Motion::Clip(c) => self.anim_clip_row(canvas, (x, y, w), "Clip", c, None),
            Motion::Blend1D { param, points } => {
                let y = self.anim_axis_row(canvas, (x, y, w), "Parameter", param, 0, g);
                let y = self.anim_blend1d(canvas, (x, y, w), l, i, param, points);
                self.anim_points(canvas, (x, y, w), s)
            }
            Motion::Blend2D {
                x: px,
                y: py,
                points,
            } => {
                let y = self.anim_axis_row(canvas, (x, y, w), "X", px, 0, g);
                let y = self.anim_axis_row(canvas, (x, y, w), "Y", py, 1, g);
                let y = self.anim_blend2d(canvas, (x, y, w), l, i, (px, py), points);
                self.anim_points(canvas, (x, y, w), s)
            }
        };
        y = self.anim_playback_rows(canvas, (x, y + 4.0, w), g, s);
        y = self.anim_outgoing(canvas, (x, y, w), l, i);
        let b = Rect::new(x, y + 6.0, 130.0, 24.0);
        if self.button(canvas, b, "Make Transition", ButtonStyle::Normal) {
            self.anim_action(AnimAction::Link(AnimNode::State(i)));
        }
        if self.button(
            canvas,
            Rect::new(x + 136.0, y + 6.0, 110.0, 24.0),
            "Delete State",
            ButtonStyle::Danger,
        ) {
            self.anim_action(AnimAction::DeleteState(i));
        }
        y + 40.0
    }

    /// A clip: a text field, plus a menu of the model's clips.
    fn anim_clip_row(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        label: &str,
        clip: &str,
        point: Option<usize>,
    ) -> f32 {
        Self::anim_label(canvas, x, y, label);
        let clips = self.anim_clips();
        let bad = !clips.is_empty() && !clips.iter().any(|c| c == clip);
        let r = Rect::new(x + LABEL_W, y, w - LABEL_W, ROW_H - 4.0);
        if clips.is_empty() {
            let prop = point.map_or(AnimProp::Clip, AnimProp::PointClip);
            self.field_box(
                canvas,
                r,
                EditTarget::AnimProp(prop),
                false,
                clip,
                "clip name",
            );
        } else {
            self.anim_pick(canvas, r, clip, bad, move |_| {
                clips
                    .iter()
                    .enumerate()
                    .map(|(c, name)| {
                        let a = point.map_or(AnimAction::Clip(c), |p| AnimAction::PointClip(p, c));
                        item(name.clone(), a)
                    })
                    .collect()
            });
        }
        y + ROW_H
    }

    /// A blend axis: a menu of the number parameters.
    fn anim_axis_row(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        label: &str,
        param: &str,
        axis: u8,
        g: &AnimGraph,
    ) -> f32 {
        Self::anim_label(canvas, x, y, label);
        let bad = !g
            .parameter(param)
            .is_some_and(|p| matches!(p.kind, ParamKind::Float(_) | ParamKind::Int(_)));
        let numbers: Vec<(usize, String)> = g
            .parameters
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.kind, ParamKind::Float(_) | ParamKind::Int(_)))
            .map(|(i, p)| (i, p.name.clone()))
            .collect();
        let r = Rect::new(x + LABEL_W, y, w - LABEL_W, ROW_H - 4.0);
        self.anim_pick(canvas, r, param, bad, move |_| {
            if numbers.is_empty() {
                return vec![item("Add a Float parameter first", AnimAction::AddParam(0))];
            }
            numbers
                .iter()
                .map(|(i, name)| item(name.clone(), AnimAction::BlendParam(axis, *i)))
                .collect()
        });
        y + ROW_H
    }

    /// A blend space's points: clip, position, ×; then + Point.
    fn anim_points(&mut self, canvas: &mut Canvas, (x, y, w): (f32, f32, f32), s: &State) -> f32 {
        let mut y = y;
        let rows: Vec<(String, Vec<f32>)> = match &s.motion {
            Motion::Blend1D { points, .. } => points
                .iter()
                .map(|p| (p.clip.clone(), vec![p.at]))
                .collect(),
            Motion::Blend2D { points, .. } => points
                .iter()
                .map(|p| (p.clip.clone(), p.at.to_vec()))
                .collect(),
            _ => Vec::new(),
        };
        for (p, (clip, at)) in rows.iter().enumerate() {
            let label = format!("Point {}", p + 1);
            y = self.anim_clip_row(canvas, (x, y, w - 26.0), &label, clip, Some(p));
            let del = Rect::new(x + w - 22.0, y - ROW_H, 22.0, ROW_H - 4.0);
            if self.button(canvas, del, "×", ButtonStyle::Ghost) {
                self.anim_action(AnimAction::RemovePoint(p));
            }
            let fw = (w - LABEL_W - 26.0 - 6.0 * (at.len().saturating_sub(1) as f32))
                / at.len().max(1) as f32;
            Self::anim_label(
                canvas,
                x,
                y,
                if at.len() == 1 { "  at" } else { "  at (x, y)" },
            );
            for (axis, v) in (0u8..).zip(at) {
                let f = Rect::new(
                    x + LABEL_W + f32::from(axis) * (fw + 6.0),
                    y,
                    fw,
                    ROW_H - 4.0,
                );
                self.field_box(
                    canvas,
                    f,
                    EditTarget::AnimProp(AnimProp::PointAt(p, axis)),
                    true,
                    &num(*v),
                    "",
                );
            }
            y += ROW_H + 2.0;
        }
        if self.button(
            canvas,
            Rect::new(x + LABEL_W, y, 90.0, ROW_H - 4.0),
            "+ Point",
            ButtonStyle::Normal,
        ) {
            self.anim_action(AnimAction::AddPoint);
        }
        y + ROW_H + 4.0
    }

    fn anim_playback_rows(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        g: &AnimGraph,
        s: &State,
    ) -> f32 {
        let mut y = self.anim_field(
            canvas,
            (x, y, w),
            "Speed",
            AnimProp::StateSpeed,
            &num(s.speed),
            true,
            "",
        );
        Self::anim_label(canvas, x, y, "  × parameter");
        let floats: Vec<(usize, String)> = g
            .parameters
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.kind, ParamKind::Float(_)))
            .map(|(i, p)| (i, p.name.clone()))
            .collect();
        let r = Rect::new(x + LABEL_W, y, w - LABEL_W, ROW_H - 4.0);
        self.anim_pick(canvas, r, &s.speed_param, false, move |_| {
            std::iter::once(item("None", AnimAction::SpeedParam(None)))
                .chain(
                    floats
                        .iter()
                        .map(|(i, n)| item(n.clone(), AnimAction::SpeedParam(Some(*i)))),
                )
                .collect()
        });
        self.tip(
            r,
            "A float parameter the speed is multiplied by (say, how fast the character moves)",
        );
        y += ROW_H;
        let (ny, clicked) = self.prop_check(canvas, x, y, w, "Loop", s.looping);
        if clicked {
            let (layer, sel) = (self.animed.layer, self.animed.sel);
            if let AnimSel::State(i) = sel {
                self.edit_state(layer, i, |s| s.looping = !s.looping);
            }
        }
        ny
    }

    /// The transitions leaving state `i` (click one to select it).
    fn anim_outgoing(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        l: &GraphLayer,
        i: usize,
    ) -> f32 {
        let name = &l.states[i].name;
        let out: Vec<(usize, &Transition)> = l
            .transitions
            .iter()
            .enumerate()
            .filter(|(_, t)| &t.from == name)
            .collect();
        if out.is_empty() {
            return y;
        }
        let mut y = self.section_head(canvas, x, y, "Transitions");
        for (t, tr) in out {
            let r = Rect::new(x - 4.0, y - 2.0, w + 8.0, 22.0);
            let hot = self.hot(r);
            if hot {
                canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::hover());
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    self.animed.sel = AnimSel::Transition(t);
                }
            }
            let when: Vec<String> = tr
                .when
                .iter()
                .map(automata_assets::anim_graph::Condition::describe)
                .collect();
            let text = if when.is_empty() {
                format!("→ {}  when it ends", tr.to)
            } else {
                format!("→ {}  {}", tr.to, when.join(" & "))
            };
            canvas.text_clipped(x, y + 2.0, layout::FONT_SIZE, &text, color::text(), w);
            y += 22.0;
        }
        y
    }

    #[allow(clippy::too_many_lines)]
    fn anim_transition_panel(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        g: &AnimGraph,
        l: &GraphLayer,
        t: usize,
    ) -> f32 {
        let tr = &l.transitions[t];
        let mut y = self.section_head(canvas, x, y, "Transition");
        let from = if tr.from == automata_assets::anim_graph::ANY_STATE {
            "Any State"
        } else {
            tr.from.as_str()
        };
        canvas.text_in(
            crate::canvas::Face::Medium,
            x,
            y,
            layout::FONT_SIZE,
            &format!("{from}  →  {}", tr.to),
            color::text(),
        );
        y += 26.0;
        y = self.anim_field(
            canvas,
            (x, y, w),
            "Fade",
            AnimProp::Fade,
            &num(tr.fade),
            true,
            "",
        );
        let exit = tr.exit_time.map(num).unwrap_or_default();
        y = self.anim_field(
            canvas,
            (x, y, w),
            "Exit time",
            AnimProp::ExitTime,
            &exit,
            true,
            "none",
        );
        y = self.anim_note(
            canvas,
            (x, y, w),
            "Fade: seconds to blend. Exit time: how far through its motion the state must be first (1 = once through; empty: go at once).",
        );
        y = self.section_head(canvas, x, y, "Conditions");
        for (c, cond) in tr.when.iter().enumerate() {
            y = self.anim_condition_row(canvas, (x, y, w), g, c, cond);
        }
        if tr.when.is_empty() {
            let text = match tr.effective_exit_time() {
                Some(e) => format!(
                    "None: it fires when the state has played {} times through.",
                    num(e)
                ),
                None => "None".into(),
            };
            y = self.anim_note(canvas, (x, y, w), &text);
        }
        let style = if g.parameters.is_empty() {
            ButtonStyle::Disabled
        } else {
            ButtonStyle::Normal
        };
        let add = Rect::new(x, y + 2.0, 120.0, 24.0);
        if self.button(canvas, add, "+ Condition", style) {
            self.anim_action(AnimAction::AddCondition);
        }
        if g.parameters.is_empty() {
            self.tip(add, "Add a parameter first");
        }
        y += 34.0;
        if self.button(
            canvas,
            Rect::new(x, y, 140.0, 24.0),
            "Delete Transition",
            ButtonStyle::Danger,
        ) {
            self.anim_action(AnimAction::DeleteTransition(t));
        }
        y + 34.0
    }

    fn anim_condition_row(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        g: &AnimGraph,
        c: usize,
        cond: &automata_assets::anim_graph::Condition,
    ) -> f32 {
        let param = g.parameter(cond.param()).cloned();
        let pw = w * 0.38;
        let names: Vec<String> = g.parameters.iter().map(|p| p.name.clone()).collect();
        self.anim_pick(
            canvas,
            Rect::new(x, y, pw, ROW_H - 4.0),
            cond.param(),
            param.is_none(),
            move |_| {
                names
                    .iter()
                    .enumerate()
                    .map(|(p, n)| item(n.clone(), AnimAction::CondParam(c, p)))
                    .collect()
            },
        );
        let ops: Vec<u8> = param
            .as_ref()
            .map_or_else(|| vec![cond_op(cond)], |p| ops_for(p.kind).to_vec());
        let ow = 64.0;
        let op_r = Rect::new(x + pw + 4.0, y, ow, ROW_H - 4.0);
        self.anim_pick(
            canvas,
            op_r,
            COND_OPS[usize::from(cond_op(cond))],
            false,
            move |_| {
                ops.iter()
                    .map(|&op| item(COND_OPS[usize::from(op)], AnimAction::CondOp(c, op)))
                    .collect()
            },
        );
        if let Some(v) = cond_value(cond) {
            let vx = op_r.x + ow + 4.0;
            let f = Rect::new(vx, y, (x + w - 26.0 - vx).max(20.0), ROW_H - 4.0);
            self.field_box(
                canvas,
                f,
                EditTarget::AnimProp(AnimProp::CondValue(c)),
                true,
                &num(v),
                "",
            );
        }
        let del = Rect::new(x + w - 22.0, y, 22.0, ROW_H - 4.0);
        if self.button(canvas, del, "×", ButtonStyle::Ghost) {
            self.anim_action(AnimAction::RemoveCondition(c));
        }
        y + ROW_H
    }

    // ----- blend space editors ----------------------------------------------

    /// The live value of parameter `name`, during Play.
    fn anim_live_param(&self, name: &str) -> Option<f32> {
        self.animed
            .live
            .as_ref()?
            .state
            .params
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.value)
    }

    /// Follows a blend point drag: `to_value` maps the pointer to the
    /// point's position; on release it's written to the file.
    fn anim_point_drag(
        &mut self,
        layer: usize,
        state: usize,
        to_value: impl Fn(f32, f32) -> [f32; 2],
    ) -> Option<(usize, [f32; 2])> {
        let Some(AnimDrag::Point { index, .. }) = self.animed.drag else {
            return None;
        };
        let at = to_value(self.raw.x, self.raw.y);
        if self.raw.down {
            self.animed.drag = Some(AnimDrag::Point { index, at });
            self.cursor = CursorHint::Grab;
            return Some((index, at));
        }
        self.animed.drag = None;
        let snap = |v: f32| (v * 20.0).round() / 20.0;
        self.edit_state(layer, state, |s| {
            super::anim_editor::set_point(&mut s.motion, index, 0, snap(at[0]));
            super::anim_editor::set_point(&mut s.motion, index, 1, snap(at[1]));
        });
        None
    }

    /// The 1D blend space: the parameter's line with a tick per clip
    /// (drag to move one) and, during Play, the live value.
    #[allow(clippy::too_many_arguments)]
    fn anim_blend1d(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        l: &GraphLayer,
        i: usize,
        param: &str,
        points: &[automata_assets::anim_graph::BlendPoint1D],
    ) -> f32 {
        let r = Rect::new(x, y + 2.0, w, 64.0);
        canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, color::viewport_bg());
        let lo = points.iter().map(|p| p.at).fold(0.0f32, f32::min);
        let hi = points.iter().map(|p| p.at).fold(lo + 1.0, f32::max);
        let (left, span) = (r.x + 16.0, r.w - 32.0);
        let to_x = |v: f32| left + (v - lo) / (hi - lo) * span;
        let layer = self.animed.layer;
        let dragged =
            self.anim_point_drag(layer, i, |px, _| [lo + (px - left) / span * (hi - lo), 0.0]);
        let line_y = r.y + 34.0;
        canvas.fill_rect(left, line_y, span, 1.5, color::text_faint());
        for (p, pt) in points.iter().enumerate() {
            let at = dragged
                .filter(|(d, _)| *d == p)
                .map_or(pt.at, |(_, a)| a[0]);
            let px = to_x(at);
            let knob = Rect::new(px - 6.0, line_y - 8.0, 12.0, 16.0);
            let hot = self.hot(knob);
            canvas.fill_rounded(
                knob.x,
                knob.y,
                knob.w,
                knob.h,
                3.0,
                if hot {
                    color::accent()
                } else {
                    color::accent_soft()
                },
            );
            let label = if pt.clip.is_empty() {
                "?"
            } else {
                pt.clip.as_str()
            };
            let above = p % 2 == 0;
            let ty = if above { r.y + 4.0 } else { line_y + 12.0 };
            let lw = canvas.measure_text(layout::SMALL_SIZE, label).min(80.0);
            let lx = (px - lw / 2.0).clamp(r.x + 4.0, r.x + r.w - lw - 4.0);
            canvas.text_clipped(lx, ty, layout::SMALL_SIZE, label, color::text_dim(), lw);
            if hot {
                self.cursor = CursorHint::Grab;
                self.tip(knob, &format!("{label} at {param} = {}", num(at)));
                if self.p.pressed {
                    self.animed.drag = Some(AnimDrag::Point {
                        index: p,
                        at: [at, 0.0],
                    });
                }
            }
        }
        if let Some(v) = self
            .anim_live_param(param)
            .filter(|_| self.anim_state_live(l, i))
        {
            let px = to_x(v.clamp(lo, hi));
            canvas.fill_rect(px - 1.0, r.y + 4.0, 2.0, r.h - 8.0, color::success());
        }
        r.y + r.h + 8.0
    }

    /// Whether state `i` is the live state of its layer.
    fn anim_state_live(&self, l: &GraphLayer, i: usize) -> bool {
        self.animed.live.as_ref().is_some_and(|live| {
            live.state
                .layers
                .iter()
                .any(|s| s.name == l.name && s.state == l.states[i].name)
        })
    }

    /// The 2D blend space: a plane with a dot per clip (drag to move one)
    /// and, during Play, where the two parameters put the character.
    #[allow(clippy::too_many_arguments)]
    fn anim_blend2d(
        &mut self,
        canvas: &mut Canvas,
        (x, y, w): (f32, f32, f32),
        l: &GraphLayer,
        i: usize,
        (px, py): (&str, &str),
        points: &[automata_assets::anim_graph::BlendPoint2D],
    ) -> f32 {
        let side = w.min(240.0);
        let r = Rect::new(x + (w - side) / 2.0, y + 4.0, side, side);
        canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, color::viewport_bg());
        let mut lo = [-1.0f32, -1.0];
        let mut hi = [1.0f32, 1.0];
        for p in points {
            for k in 0..2 {
                lo[k] = lo[k].min(p.at[k]);
                hi[k] = hi[k].max(p.at[k]);
            }
        }
        let inner = Rect::new(r.x + 14.0, r.y + 14.0, r.w - 28.0, r.h - 28.0);
        let to_screen = |v: [f32; 2]| {
            (
                inner.x + (v[0] - lo[0]) / (hi[0] - lo[0]) * inner.w,
                inner.y + inner.h - (v[1] - lo[1]) / (hi[1] - lo[1]) * inner.h,
            )
        };
        let to_value = |sx: f32, sy: f32| {
            [
                lo[0] + (sx - inner.x) / inner.w * (hi[0] - lo[0]),
                lo[1] + (inner.y + inner.h - sy) / inner.h * (hi[1] - lo[1]),
            ]
        };
        let (ox, oy) = to_screen([0.0, 0.0]);
        canvas.fill_rect(inner.x, oy, inner.w, 1.0, color::grid());
        canvas.fill_rect(ox, inner.y, 1.0, inner.h, color::grid());
        let pw = canvas.measure_text(layout::SMALL_SIZE, px);
        canvas.text(
            r.x + r.w - 6.0 - pw,
            oy + 3.0,
            layout::SMALL_SIZE,
            px,
            color::text_faint(),
        );
        canvas.text(
            ox + 4.0,
            r.y + 2.0,
            layout::SMALL_SIZE,
            py,
            color::text_faint(),
        );
        let layer = self.animed.layer;
        let dragged = self.anim_point_drag(layer, i, to_value);
        for (p, pt) in points.iter().enumerate() {
            let at = dragged.filter(|(d, _)| *d == p).map_or(pt.at, |(_, a)| a);
            let (sx, sy) = to_screen(at);
            let knob = Rect::new(sx - 6.0, sy - 6.0, 12.0, 12.0);
            let hot = self.hot(knob);
            canvas.fill_circle(sx, sy, if hot { 6.0 } else { 5.0 }, color::accent_soft());
            let label = if pt.clip.is_empty() {
                "?"
            } else {
                pt.clip.as_str()
            };
            canvas.text_clipped(
                sx + 8.0,
                sy - 7.0,
                layout::SMALL_SIZE,
                label,
                color::text_dim(),
                70.0,
            );
            if hot {
                self.cursor = CursorHint::Grab;
                self.tip(
                    knob,
                    &format!("{label} at ({}, {})", num(at[0]), num(at[1])),
                );
                if self.p.pressed {
                    self.animed.drag = Some(AnimDrag::Point { index: p, at });
                }
            }
        }
        if self.anim_state_live(l, i) {
            if let (Some(vx), Some(vy)) = (self.anim_live_param(px), self.anim_live_param(py)) {
                let (sx, sy) = to_screen([vx.clamp(lo[0], hi[0]), vy.clamp(lo[1], hi[1])]);
                canvas.stroke_circle(sx, sy, 7.0, color::success(), 2.0);
                canvas.fill_circle(sx, sy, 2.0, color::success());
            }
        }
        r.y + r.h + 10.0
    }
}
