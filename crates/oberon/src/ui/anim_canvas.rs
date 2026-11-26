//! The graph editor's node canvas: one layer's states as boxes, its
//! transitions as arrows, Any State on the left. Drag a state to move it,
//! drag the background (or the middle button) to pan, wheel to zoom,
//! right-click for commands, and double-click the background to add a
//! state. During Play the state the entity is in glows, with its progress
//! along the bottom, and the transition it's fading through lights up.

use automata_assets::anim_graph::{AnimGraph, Condition, GraphLayer, ANY_STATE};

use super::anim_editor::{any_position, AnimAction, AnimDrag, AnimNode, AnimSel, NODE_H, NODE_W};
use super::menus::{MenuItem, UiAction};
use super::{color, layout, CursorHint, Rect, Ui};
use crate::canvas::Canvas;

/// A transition as drawn: its index, its ends (screen) and whether it
/// loops back to its own state.
#[derive(Debug, Clone, Copy)]
pub(super) struct Arrow {
    index: usize,
    a: (f32, f32),
    b: (f32, f32),
    looped: bool,
}

/// How far `p` is from the segment `a`–`b`.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Where a line from `r`'s centre heading along `dir` (unit) leaves it.
fn edge_point(r: Rect, dir: (f32, f32), offset: (f32, f32)) -> (f32, f32) {
    let (cx, cy) = (r.x + r.w / 2.0 + offset.0, r.y + r.h / 2.0 + offset.1);
    let tx = if dir.0.abs() > 1e-4 {
        (r.w / 2.0) / dir.0.abs()
    } else {
        f32::INFINITY
    };
    let ty = if dir.1.abs() > 1e-4 {
        (r.h / 2.0) / dir.1.abs()
    } else {
        f32::INFINITY
    };
    let t = tx.min(ty);
    (cx + dir.0 * t, cy + dir.1 * t)
}

/// The two points of a loop above `r` (a transition to its own state).
fn loop_ends(r: Rect) -> ((f32, f32), (f32, f32)) {
    ((r.x + r.w * 0.62, r.y), (r.x + r.w * 0.82, r.y))
}

impl Ui {
    fn anim_zoom(&self) -> f32 {
        if self.animed.zoom > 0.0 {
            self.animed.zoom
        } else {
            1.0
        }
    }

    /// A world point (graph units) on the canvas `c`.
    pub(super) fn anim_to_screen(&self, c: Rect, p: [f32; 2]) -> (f32, f32) {
        let z = self.anim_zoom();
        (
            c.x + c.w / 2.0 + (p[0] + self.animed.pan[0]) * z,
            c.y + c.h / 2.0 + (p[1] + self.animed.pan[1]) * z,
        )
    }

    /// A canvas point in world units.
    pub(super) fn anim_to_world(&self, c: Rect, x: f32, y: f32) -> [f32; 2] {
        let z = self.anim_zoom();
        [
            (x - c.x - c.w / 2.0) / z - self.animed.pan[0],
            (y - c.y - c.h / 2.0) / z - self.animed.pan[1],
        ]
    }

    /// The node canvas for the shown layer of `g`.
    pub(super) fn anim_canvas(&mut self, canvas: &mut Canvas, c: Rect, g: &AnimGraph) {
        self.animed.canvas_rect = Some(c);
        if std::mem::take(&mut self.animed.refit) {
            self.anim_frame_all();
        }
        canvas.fill_rect(c.x, c.y, c.w, c.h, color::viewport_bg());
        let prev = canvas.set_clip(Some((c.x, c.y, c.w, c.h)));
        self.anim_grid(canvas, c);
        let Some(l) = g.layers.get(self.animed.layer) else {
            canvas.restore_clip(prev);
            return;
        };
        let nodes = self.anim_node_rects(c, l);
        let arrows = anim_arrows(l, &nodes);
        self.anim_draw_arrows(canvas, l, &arrows);
        self.anim_draw_nodes(canvas, l, &nodes);
        self.anim_draw_labels(canvas, l, &arrows);
        if let Some(r) = self
            .animed
            .linking
            .and_then(|n| nodes.iter().find(|(m, _)| *m == n))
            .map(|(_, r)| *r)
        {
            let from = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            canvas.line(from.0, from.1, self.raw.x, self.raw.y, color::accent(), 1.5);
            canvas.fill_circle(self.raw.x, self.raw.y, 3.0, color::accent());
        }
        if l.states.is_empty() {
            let hint = "Right-click to add a state";
            let tw = canvas.measure_text(layout::FONT_SIZE, hint);
            canvas.text(
                c.x + (c.w - tw) / 2.0,
                c.y + c.h / 2.0,
                layout::FONT_SIZE,
                hint,
                color::text_faint(),
            );
        }
        canvas.restore_clip(prev);
        self.animed.nodes.clone_from(&nodes);
        self.anim_canvas_drag(c);
        self.anim_canvas_input(c, l, &nodes, &arrows);
    }

    fn anim_grid(&self, canvas: &mut Canvas, c: Rect) {
        let z = self.anim_zoom();
        let step = 40.0 * z;
        if step < 8.0 {
            return;
        }
        let (ox, oy) = self.anim_to_screen(c, [0.0, 0.0]);
        let mut x = c.x + (ox - c.x).rem_euclid(step);
        while x < c.x + c.w {
            canvas.fill_rect(x, c.y, 1.0, c.h, color::raised());
            x += step;
        }
        let mut y = c.y + (oy - c.y).rem_euclid(step);
        while y < c.y + c.h {
            canvas.fill_rect(c.x, y, c.w, 1.0, color::raised());
            y += step;
        }
    }

    /// Every node's rectangle on screen (a state being dragged where it's
    /// dragged to).
    fn anim_node_rects(&self, c: Rect, l: &GraphLayer) -> Vec<(AnimNode, Rect)> {
        let z = self.anim_zoom();
        let rect = |p: [f32; 2]| {
            let (x, y) = self.anim_to_screen(c, p);
            Rect::new(x, y, NODE_W * z, NODE_H * z)
        };
        let mut out = vec![(AnimNode::Any, rect(any_position(l)))];
        for (i, s) in l.states.iter().enumerate() {
            let at = match self.animed.drag {
                Some(AnimDrag::Move { state, at, .. }) if state == i => at,
                _ => s.position,
            };
            out.push((AnimNode::State(i), rect(at)));
        }
        out
    }

    /// The live playback of the shown layer, if any.
    fn anim_live_layer(&self, l: &GraphLayer) -> Option<&automata_scene::GraphLayerState> {
        self.animed
            .live
            .as_ref()?
            .state
            .layers
            .iter()
            .find(|s| s.name == l.name)
    }

    fn anim_draw_arrows(&self, canvas: &mut Canvas, l: &GraphLayer, arrows: &[Arrow]) {
        let live = self.anim_live_layer(l);
        let z = self.anim_zoom();
        for arrow in arrows {
            let t = &l.transitions[arrow.index];
            let selected = self.animed.sel == AnimSel::Transition(arrow.index);
            let firing = live.is_some_and(|s| {
                s.fade < 1.0
                    && s.state == t.to
                    && (t.from == ANY_STATE || s.from.as_deref() == Some(t.from.as_str()))
            });
            let (c, width) = if firing {
                (color::success(), 2.5)
            } else if selected {
                (color::accent(), 2.5)
            } else {
                (color::text_dim(), 1.5)
            };
            let (a, b) = (arrow.a, arrow.b);
            let (dir, mid) = if arrow.looped {
                let lift = 22.0 * z;
                canvas.polyline(&[a, (a.0, a.1 - lift), (b.0, b.1 - lift), b], c, width);
                ((0.0, 1.0), (b.0, b.1 - 4.0 * z))
            } else {
                canvas.line(a.0, a.1, b.0, b.1, c, width);
                let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1e-3);
                (
                    ((b.0 - a.0) / len, (b.1 - a.1) / len),
                    (f32::midpoint(a.0, b.0), f32::midpoint(a.1, b.1)),
                )
            };
            let size = 7.0;
            let (px, py) = (-dir.1, dir.0);
            let tip = (mid.0 + dir.0 * size, mid.1 + dir.1 * size);
            canvas.fill_polygon(
                &[
                    tip,
                    (
                        mid.0 - dir.0 * size + px * size,
                        mid.1 - dir.1 * size + py * size,
                    ),
                    (
                        mid.0 - dir.0 * size - px * size,
                        mid.1 - dir.1 * size - py * size,
                    ),
                ],
                c,
            );
        }
    }

    /// The transitions' conditions, beside their arrows (over the nodes,
    /// on a backing so they stay readable).
    fn anim_draw_labels(&self, canvas: &mut Canvas, l: &GraphLayer, arrows: &[Arrow]) {
        if self.anim_zoom() < 0.7 {
            return;
        }
        for arrow in arrows.iter().filter(|a| !a.looped) {
            let t = &l.transitions[arrow.index];
            let mut label: Vec<String> = t.when.iter().map(Condition::describe).collect();
            if let Some(e) = t.exit_time {
                label.push(format!("at {}", super::ui_editor::num(e)));
            }
            if label.is_empty() {
                continue;
            }
            let selected = self.animed.sel == AnimSel::Transition(arrow.index);
            let text = label.join(" & ");
            let tw = canvas.measure_text(layout::SMALL_SIZE, &text).min(160.0);
            let (a, b) = (arrow.a, arrow.b);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1e-3);
            // To the arrow's right (its own side: A→B and B→A part ways).
            let (px, py) = (-(b.1 - a.1) / len, (b.0 - a.0) / len);
            let (mx, my) = (f32::midpoint(a.0, b.0), f32::midpoint(a.1, b.1));
            let reach = 9.0 + (px * (tw / 2.0 + 4.0)).abs().max((py * 9.0).abs());
            let (cx, cy) = (mx + px * reach, my + py * reach);
            let (x, y) = (cx - tw / 2.0, cy - 8.0);
            canvas.fill_rounded(x - 4.0, y, tw + 8.0, 16.0, 3.0, color::viewport_bg());
            canvas.text_clipped(
                x,
                y + 2.0,
                layout::SMALL_SIZE,
                &text,
                if selected {
                    color::text()
                } else {
                    color::text_dim()
                },
                tw,
            );
        }
    }

    #[allow(clippy::too_many_lines)]
    fn anim_draw_nodes(&self, canvas: &mut Canvas, l: &GraphLayer, nodes: &[(AnimNode, Rect)]) {
        let live = self.anim_live_layer(l);
        let z = self.anim_zoom();
        let title_size = (layout::FONT_SIZE * z).clamp(8.0, 16.0);
        let sub_size = (layout::SMALL_SIZE * z).clamp(7.0, 14.0);
        let entry = l.entry_index();
        for &(node, r) in nodes {
            let hot = self.hot(r) && self.animed.drag.is_none();
            let selected = match node {
                AnimNode::Any => self.animed.sel == AnimSel::Any,
                AnimNode::State(i) => self.animed.sel == AnimSel::State(i),
            };
            let (title, sub, is_live, fading) = match node {
                AnimNode::Any => (
                    "Any State".to_string(),
                    "from every state".to_string(),
                    false,
                    false,
                ),
                AnimNode::State(i) => {
                    let s = &l.states[i];
                    let is_live = live.is_some_and(|ls| ls.state == s.name);
                    let fading = live.is_some_and(|ls| ls.from.as_deref() == Some(s.name.as_str()));
                    (s.name.clone(), s.motion.summary(), is_live, fading)
                }
            };
            let radius = 6.0 * z.max(0.5);
            canvas.fill_rounded(
                r.x,
                r.y,
                r.w,
                r.h,
                radius,
                if hot { color::hover() } else { color::raised() },
            );
            if node == AnimNode::Any {
                canvas.fill_rect(r.x, r.y + 4.0, 3.0, r.h - 8.0, color::axis_z());
            } else if entry
                == Some(match node {
                    AnimNode::State(i) => i,
                    AnimNode::Any => usize::MAX,
                })
            {
                canvas.fill_rect(r.x, r.y + 4.0, 3.0, r.h - 8.0, color::warn());
                // A short arrow in from the left: where the layer starts.
                let (ex, ey) = (r.x - 18.0 * z, r.y + r.h / 2.0);
                canvas.line(ex, ey, r.x - 3.0, ey, color::warn(), 1.5);
                canvas.fill_polygon(
                    &[
                        (r.x - 2.0, ey),
                        (r.x - 8.0, ey - 4.0),
                        (r.x - 8.0, ey + 4.0),
                    ],
                    color::warn(),
                );
            }
            let (border, width) = if is_live {
                (color::success(), 2.5)
            } else if selected {
                (color::accent(), 2.0)
            } else if fading {
                (color::success(), 1.2)
            } else {
                (color::border(), 1.0)
            };
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, radius, border, width);
            let pad = 10.0 * z.max(0.6);
            let shown = clip_to(canvas, &title, title_size, r.w - 2.0 * pad);
            canvas.text_in(
                crate::canvas::Face::Medium,
                r.x + pad,
                r.y + r.h * 0.18,
                title_size,
                &shown,
                color::text(),
            );
            canvas.text_clipped(
                r.x + pad,
                r.y + r.h * 0.56,
                sub_size,
                &sub,
                color::text_dim(),
                r.w - 2.0 * pad,
            );
            if let (true, Some(ls)) = (is_live, live) {
                let bar = (r.w - 2.0 * radius) * ls.time.clamp(0.0, 1.0);
                canvas.fill_rect(r.x + radius, r.y + r.h - 4.0, bar, 2.5, color::success());
            }
        }
    }

    /// Follows a drag in progress: a state moving (written on release) or
    /// the view panning.
    fn anim_canvas_drag(&mut self, c: Rect) {
        match self.animed.drag {
            Some(AnimDrag::Move {
                state,
                grab,
                at: last,
                moved,
            }) => {
                let w = self.anim_to_world(c, self.raw.x, self.raw.y);
                let at = [w[0] - grab[0], w[1] - grab[1]];
                if self.raw.down {
                    if moved || (at[0] - last[0]).abs() + (at[1] - last[1]).abs() > 3.0 {
                        self.animed.drag = Some(AnimDrag::Move {
                            state,
                            grab,
                            at,
                            moved: true,
                        });
                    }
                    self.cursor = CursorHint::Grab;
                    return;
                }
                self.animed.drag = None;
                if moved {
                    let snap = |v: f32| (v / 10.0).round() * 10.0;
                    let layer = self.animed.layer;
                    self.edit_state(layer, state, |s| s.position = [snap(at[0]), snap(at[1])]);
                }
            }
            Some(AnimDrag::Pan { last }) => {
                if self.raw.down || self.raw.middle_down {
                    let z = self.anim_zoom();
                    self.animed.pan[0] += (self.raw.x - last.0) / z;
                    self.animed.pan[1] += (self.raw.y - last.1) / z;
                    self.animed.drag = Some(AnimDrag::Pan {
                        last: (self.raw.x, self.raw.y),
                    });
                    self.cursor = CursorHint::Grab;
                } else {
                    self.animed.drag = None;
                }
            }
            _ => {}
        }
    }

    /// Clicks, wheel and right-clicks on the canvas.
    fn anim_canvas_input(
        &mut self,
        c: Rect,
        l: &GraphLayer,
        nodes: &[(AnimNode, Rect)],
        arrows: &[Arrow],
    ) {
        if !self.hot(c) || self.animed.drag.is_some() {
            return;
        }
        let (px, py) = (self.p.x, self.p.y);
        if self.p.scroll != 0.0 {
            let before = self.anim_to_world(c, px, py);
            let z = (self.anim_zoom() * 1.15f32.powf(self.p.scroll)).clamp(0.25, 2.5);
            self.animed.zoom = z;
            let after = self.anim_to_world(c, px, py);
            self.animed.pan[0] += after[0] - before[0];
            self.animed.pan[1] += after[1] - before[1];
        }
        let hit_node = nodes
            .iter()
            .rev()
            .find(|(_, r)| r.contains(px, py))
            .map(|(n, _)| *n);
        let hit_arrow = if hit_node.is_none() {
            arrows
                .iter()
                .find(|a| {
                    if a.looped {
                        let lift = 22.0 * self.anim_zoom();
                        Rect::new(
                            a.a.0 - 4.0,
                            a.a.1 - lift - 4.0,
                            a.b.0 - a.a.0 + 8.0,
                            lift + 8.0,
                        )
                        .contains(px, py)
                    } else {
                        segment_distance((px, py), a.a, a.b) < 6.0
                    }
                })
                .map(|a| a.index)
        } else {
            None
        };
        if hit_node.is_some() || hit_arrow.is_some() || self.animed.linking.is_some() {
            self.cursor = CursorHint::Pointer;
        }
        if self.p.pressed {
            self.anim_canvas_press(c, l, hit_node, hit_arrow);
        } else if self.p.middle_pressed {
            self.animed.drag = Some(AnimDrag::Pan { last: (px, py) });
        } else if self.p.right_pressed {
            self.animed.linking = None;
            self.anim_canvas_menu(c, l, hit_node, hit_arrow);
        }
    }

    fn anim_canvas_press(
        &mut self,
        c: Rect,
        l: &GraphLayer,
        hit_node: Option<AnimNode>,
        hit_arrow: Option<usize>,
    ) {
        if let Some(intent) = self.commit_edit() {
            self.out.push(intent);
        }
        let (px, py) = (self.p.x, self.p.y);
        if self.animed.linking.is_some() {
            match hit_node {
                Some(n) => self.anim_finish_link(n),
                None => self.animed.linking = None,
            }
            return;
        }
        match (hit_node, hit_arrow) {
            (Some(AnimNode::State(i)), _) => {
                self.animed.sel = AnimSel::State(i);
                let at = l.states[i].position;
                let w = self.anim_to_world(c, px, py);
                self.animed.drag = Some(AnimDrag::Move {
                    state: i,
                    grab: [w[0] - at[0], w[1] - at[1]],
                    at,
                    moved: false,
                });
            }
            (Some(AnimNode::Any), _) => self.animed.sel = AnimSel::Any,
            (None, Some(t)) => self.animed.sel = AnimSel::Transition(t),
            (None, None) if self.p.double_click => {
                let w = self.anim_to_world(c, px, py);
                self.anim_action(AnimAction::AddState {
                    x: w[0] - NODE_W / 2.0,
                    y: w[1] - NODE_H / 2.0,
                    kind: 1,
                });
            }
            (None, None) => {
                self.animed.sel = AnimSel::None;
                self.animed.drag = Some(AnimDrag::Pan { last: (px, py) });
            }
        }
    }

    fn anim_canvas_menu(
        &mut self,
        c: Rect,
        l: &GraphLayer,
        hit_node: Option<AnimNode>,
        hit_arrow: Option<usize>,
    ) {
        let (px, py) = (self.p.x, self.p.y);
        let item =
            |label: &str, a: AnimAction, on: bool| MenuItem::ui(label, "", UiAction::Anim(a), on);
        let items = match (hit_node, hit_arrow) {
            (Some(AnimNode::State(i)), _) => {
                self.animed.sel = AnimSel::State(i);
                vec![
                    item(
                        "Make Transition",
                        AnimAction::Link(AnimNode::State(i)),
                        true,
                    ),
                    item(
                        "Set as Entry State",
                        AnimAction::SetEntry(i),
                        l.entry_index() != Some(i),
                    ),
                    MenuItem::separator(),
                    item("Delete State", AnimAction::DeleteState(i), true),
                ]
            }
            (Some(AnimNode::Any), _) => {
                self.animed.sel = AnimSel::Any;
                vec![item(
                    "Make Transition",
                    AnimAction::Link(AnimNode::Any),
                    true,
                )]
            }
            (None, Some(t)) => {
                self.animed.sel = AnimSel::Transition(t);
                vec![item(
                    "Delete Transition",
                    AnimAction::DeleteTransition(t),
                    true,
                )]
            }
            (None, None) => {
                let w = self.anim_to_world(c, px, py);
                let (x, y) = (w[0] - NODE_W / 2.0, w[1] - NODE_H / 2.0);
                vec![
                    item("Add State", AnimAction::AddState { x, y, kind: 1 }, true),
                    item(
                        "Add 1D Blend State",
                        AnimAction::AddState { x, y, kind: 2 },
                        true,
                    ),
                    item(
                        "Add 2D Blend State",
                        AnimAction::AddState { x, y, kind: 3 },
                        true,
                    ),
                    item(
                        "Add Empty State",
                        AnimAction::AddState { x, y, kind: 0 },
                        true,
                    ),
                    MenuItem::separator(),
                    item("Frame All", AnimAction::FrameAll, true),
                ]
            }
        };
        self.open_menu(px, py, items);
    }
}

/// Every transition of `l` as an arrow between the drawn nodes. Arrows
/// sit a little right of the line between centres, so A→B and B→A show
/// as two.
fn anim_arrows(l: &GraphLayer, nodes: &[(AnimNode, Rect)]) -> Vec<Arrow> {
    let rect_of = |name: &str| {
        let node = if name == ANY_STATE {
            Some(AnimNode::Any)
        } else {
            l.state_index(name).map(AnimNode::State)
        }?;
        nodes.iter().find(|(n, _)| *n == node).map(|(_, r)| *r)
    };
    let mut out = Vec::new();
    for (index, t) in l.transitions.iter().enumerate() {
        let (Some(ra), Some(rb)) = (rect_of(&t.from), rect_of(&t.to)) else {
            continue;
        };
        if t.from == t.to {
            let (a, b) = loop_ends(ra);
            out.push(Arrow {
                index,
                a,
                b,
                looped: true,
            });
            continue;
        }
        let (ca, cb) = (
            (ra.x + ra.w / 2.0, ra.y + ra.h / 2.0),
            (rb.x + rb.w / 2.0, rb.y + rb.h / 2.0),
        );
        let len = ((cb.0 - ca.0).powi(2) + (cb.1 - ca.1).powi(2))
            .sqrt()
            .max(1e-3);
        let dir = ((cb.0 - ca.0) / len, (cb.1 - ca.1) / len);
        let off = (-dir.1 * 6.0, dir.0 * 6.0);
        out.push(Arrow {
            index,
            a: edge_point(ra, dir, off),
            b: edge_point(rb, (-dir.0, -dir.1), off),
            looped: false,
        });
    }
    out
}

/// `text` cut with an ellipsis to fit `w` at `size`.
fn clip_to(canvas: &mut Canvas, text: &str, size: f32, w: f32) -> String {
    if canvas.measure_text(size, text) <= w {
        return text.to_string();
    }
    let mut out: String = text.to_string();
    while !out.is_empty() && canvas.measure_text(size, &format!("{out}…")) > w {
        out.pop();
    }
    format!("{out}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_measure_and_edges_clip() {
        assert!((segment_distance((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)) - 3.0).abs() < 1e-5);
        assert!((segment_distance((-4.0, 3.0), (0.0, 0.0), (10.0, 0.0)) - 5.0).abs() < 1e-5);
        let r = Rect::new(0.0, 0.0, 100.0, 40.0);
        let p = edge_point(r, (1.0, 0.0), (0.0, 0.0));
        assert!(
            (p.0 - 100.0).abs() < 1e-4 && (p.1 - 20.0).abs() < 1e-4,
            "{p:?}"
        );
        let p = edge_point(r, (0.0, -1.0), (0.0, 0.0));
        assert!((p.1).abs() < 1e-4, "{p:?}");
    }
}
