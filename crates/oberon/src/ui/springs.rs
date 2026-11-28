//! The Springs part of a model's Import settings: spring chains (hair,
//! skirts, tails that swing on their own) and the body colliders that keep
//! them out of the body. Like the rest of the import settings they are
//! saved next to the model file (`<model>.import.ron`), so every entity
//! showing the model gets them.
//!
//! Chain roots and collider bones are picked from a searchable list of the
//! model's bones in the palette (likely chain roots first).

use std::path::{Path, PathBuf};

use automata_assets::import::{read_settings, ImportSettings};
use automata_assets::springs::{suggested_roots, SpringChain, SpringCollider};
use automata_assets::Rig;

use super::palette::{PaletteAction, PaletteItem};
use super::{ButtonStyle, Icon, Intent, Rect, Ui};
use crate::canvas::Canvas;
use crate::theme::{color, layout};

/// Which number of a model's import settings a text edit changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportField {
    /// The uniform scale.
    Scale,
    /// Rotation about X, Y or Z (0..=2).
    Rotation(u8),
    /// A setting of the spring chain at this index.
    Chain(u8, ChainField),
    /// A setting of the collider at this index.
    Collider(u8, ColliderField),
}

/// A spring chain's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainField {
    /// How strongly it springs back (0..=1).
    Stiffness,
    /// How quickly swinging dies down (0..=1).
    Drag,
    /// How much gravity pulls it.
    Gravity,
    /// Its thickness for colliding.
    Radius,
    /// Steady wind along X, Y or Z (0..=2).
    Wind(u8),
}

/// A collider's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColliderField {
    /// The sphere's (or capsule's) radius.
    Radius,
    /// The capsule's length along the bone (0 for a sphere).
    Length,
    /// Where it sits on the bone, X, Y or Z (0..=2).
    Offset(u8),
}

impl ImportField {
    /// Sets this field of `settings` to `value`, within sensible limits.
    /// Returns false when the value can't be used (a zero scale, a chain
    /// that no longer exists).
    pub fn apply(self, settings: &mut ImportSettings, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self {
            Self::Scale if value.abs() > 1e-6 => settings.scale = value,
            Self::Scale => return false,
            Self::Rotation(i) => settings.rotation[usize::from(i).min(2)] = value,
            Self::Chain(i, field) => {
                let Some(chain) = settings.springs.chains.get_mut(usize::from(i)) else {
                    return false;
                };
                match field {
                    ChainField::Stiffness => chain.stiffness = value.clamp(0.0, 1.0),
                    ChainField::Drag => chain.drag = value.clamp(0.0, 1.0),
                    ChainField::Gravity => chain.gravity = value.clamp(-10.0, 10.0),
                    ChainField::Radius => chain.radius = value.max(0.0),
                    ChainField::Wind(axis) => chain.wind[usize::from(axis).min(2)] = value,
                }
            }
            Self::Collider(i, field) => {
                let Some(collider) = settings.springs.colliders.get_mut(usize::from(i)) else {
                    return false;
                };
                match field {
                    ColliderField::Radius => collider.radius = value.max(0.0),
                    ColliderField::Length => collider.length = value,
                    ColliderField::Offset(axis) => {
                        collider.offset[usize::from(axis).min(2)] = value;
                    }
                }
            }
        }
        true
    }
}

/// Which list a bone picked in the palette joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpringPick {
    /// A new spring chain rooted at the bone.
    Chain,
    /// A new collider on the bone.
    Collider,
}

/// Body bones colliders usually go on, by name.
const BODY_WORDS: [&str; 10] = [
    "head", "neck", "chest", "spine", "hip", "pelvis", "shoulder", "arm", "thigh", "leg",
];

/// Whether node `i` of `rig` is a bone (skinning joint).
fn is_bone(rig: &Rig, i: usize) -> bool {
    rig.joints.iter().any(|(node, _)| *node == i)
}

/// The palette entries for picking a bone of `rig`: the likely ones first
/// (chain roots, or body bones for colliders), then every other bone with
/// its parent as the hint. Bones already in use are left out.
#[must_use]
pub fn bone_items(model: &Path, rig: &Rig, pick: SpringPick) -> Vec<PaletteItem> {
    let used: Vec<&str> = match pick {
        SpringPick::Chain => rig.springs.chains.iter().map(|c| c.root.as_str()).collect(),
        SpringPick::Collider => rig
            .springs
            .colliders
            .iter()
            .map(|c| c.bone.as_str())
            .collect(),
    };
    let likely: Vec<String> = match pick {
        SpringPick::Chain => suggested_roots(rig),
        SpringPick::Collider => rig
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                let name = n.name.to_lowercase();
                is_bone(rig, *i) && BODY_WORDS.iter().any(|w| name.contains(w))
            })
            .map(|(_, n)| n.name.clone())
            .collect(),
    };
    let item = |i: usize, group: &'static str| {
        let node = &rig.nodes[i];
        PaletteItem {
            label: node.name.clone(),
            group,
            hint: node
                .parent
                .map(|p| format!("in {}", rig.nodes[p].name))
                .unwrap_or_default(),
            action: PaletteAction::SpringBone {
                model: model.to_path_buf(),
                bone: node.name.clone(),
                pick,
            },
        }
    };
    let first = match pick {
        SpringPick::Chain => "Suggested",
        SpringPick::Collider => "Body",
    };
    let mut items: Vec<PaletteItem> = likely
        .iter()
        .filter(|name| !used.contains(&name.as_str()))
        .filter_map(|name| rig.node(name))
        .map(|i| item(i, first))
        .collect();
    items.extend(
        (0..rig.nodes.len())
            .filter(|&i| is_bone(rig, i))
            .filter(|&i| {
                let name = rig.nodes[i].name.as_str();
                !used.contains(&name) && !likely.iter().any(|l| l == name)
            })
            .map(|i| item(i, "Bone")),
    );
    items
}

/// Adds the picked bone to the model's springs (a chain rooted at it, or a
/// collider on it).
#[must_use]
pub fn add_bone(model: PathBuf, bone: String, pick: SpringPick) -> Intent {
    let mut settings = read_settings(&model);
    match pick {
        SpringPick::Chain => settings.springs.chains.push(SpringChain::new(bone)),
        SpringPick::Collider => settings.springs.colliders.push(SpringCollider::new(bone)),
    }
    Intent::SetImport { model, settings }
}

impl Ui {
    /// The Springs rows under a rigged model's import settings: one block
    /// per chain and per collider, with buttons to add more.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) fn springs_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        mut y: f32,
        w: f32,
        id: u64,
        file: &Path,
        rig: &Rig,
        editing: bool,
    ) -> f32 {
        let settings = read_settings(file);
        let fx = x + layout::LABEL_W;
        let fw = w - layout::LABEL_W;
        let normal = if editing {
            ButtonStyle::Normal
        } else {
            ButtonStyle::Disabled
        };
        let ghost = if editing {
            ButtonStyle::Ghost
        } else {
            ButtonStyle::Disabled
        };

        // "Springs", and the add buttons.
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            canvas.text(x, y + 3.0, layout::SMALL_SIZE, "Springs", color::text_dim());
            let bw = ((fw - 4.0) / 2.0).max(40.0);
            let chain = Rect::new(fx, y, bw, layout::FIELD_H);
            let collider = Rect::new(fx + bw + 4.0, y, bw, layout::FIELD_H);
            self.tip(
                chain,
                "Make a bone and everything below it swing (hair, skirt, tail)",
            );
            if self.button(canvas, chain, "+ Chain", normal) && editing {
                self.open_bone_picker(file, rig, SpringPick::Chain);
            }
            self.tip(
                collider,
                "A sphere or capsule on a body bone that chains can't pass",
            );
            if self.button(canvas, collider, "+ Collider", normal) && editing {
                self.open_bone_picker(file, rig, SpringPick::Collider);
            }
        }
        y += layout::ROW_STEP;
        if settings.springs.is_empty() {
            if self.row_visible(Rect::new(x, y, w, 16.0)) {
                canvas.text_clipped(
                    fx,
                    y,
                    layout::SMALL_SIZE,
                    "Bones that swing on their own",
                    color::text_faint(),
                    fw,
                );
            }
            return y + 20.0;
        }

        let mut remove: Option<SpringPick> = None;
        let mut index = 0;
        // A heading row: kind, bone name (red if the model lacks it), ×.
        let heading = |ui: &mut Self, canvas: &mut Canvas, y: f32, kind: &str, bone: &str| {
            if !ui.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
                return false;
            }
            ui.row_label(canvas, x, y, kind);
            let known = rig.node(bone).is_some();
            let tint = if known {
                color::text()
            } else {
                color::danger()
            };
            canvas.text_clipped(fx, y + 3.0, layout::FONT_SIZE, bone, tint, fw - 30.0);
            if !known {
                ui.tip(
                    Rect::new(fx, y, fw - 30.0, layout::FIELD_H),
                    "The model has no bone by this name",
                );
            }
            let close = Rect::new(x + w - 22.0, y + 1.0, 22.0, 18.0);
            ui.tip(close, "Remove");
            ui.icon_button(canvas, close, Icon::Close, ghost) && editing
        };

        for (i, chain) in settings.springs.chains.iter().enumerate() {
            let Ok(n) = u8::try_from(i) else { break };
            if heading(self, canvas, y, "Chain", &chain.root) {
                remove = Some(SpringPick::Chain);
                index = i;
            }
            y += layout::ROW_STEP;
            let rows = [
                ("Stiffness", ChainField::Stiffness, chain.stiffness),
                ("Drag", ChainField::Drag, chain.drag),
                ("Gravity", ChainField::Gravity, chain.gravity),
                ("Thickness", ChainField::Radius, chain.radius),
            ];
            for (label, field, value) in rows {
                if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
                    self.row_label(canvas, x + 10.0, y, label);
                    let r = Rect::new(fx, y, fw, layout::FIELD_H);
                    let which = ImportField::Chain(n, field);
                    self.import_field(canvas, r, id, which, value, file, &settings, editing);
                }
                y += layout::ROW_STEP;
            }
            y = self.triple_row(canvas, x, y, w, id, file, &settings, editing, "Wind", |a| {
                (
                    ImportField::Chain(n, ChainField::Wind(a)),
                    chain.wind[usize::from(a)],
                )
            });
        }
        for (i, collider) in settings.springs.colliders.iter().enumerate() {
            let Ok(n) = u8::try_from(i) else { break };
            if heading(self, canvas, y, "Collider", &collider.bone) {
                remove = Some(SpringPick::Collider);
                index = i;
            }
            y += layout::ROW_STEP;
            let rows = [
                ("Radius", ColliderField::Radius, collider.radius),
                ("Length", ColliderField::Length, collider.length),
            ];
            for (label, field, value) in rows {
                if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
                    self.row_label(canvas, x + 10.0, y, label);
                    let r = Rect::new(fx, y, fw, layout::FIELD_H);
                    if field == ColliderField::Length {
                        self.tip(
                            r,
                            "0 is a sphere; otherwise a capsule this long along the bone",
                        );
                    }
                    let which = ImportField::Collider(n, field);
                    self.import_field(canvas, r, id, which, value, file, &settings, editing);
                }
                y += layout::ROW_STEP;
            }
            y = self.triple_row(
                canvas,
                x,
                y,
                w,
                id,
                file,
                &settings,
                editing,
                "Offset",
                |a| {
                    (
                        ImportField::Collider(n, ColliderField::Offset(a)),
                        collider.offset[usize::from(a)],
                    )
                },
            );
        }

        if let Some(pick) = remove {
            let mut s = settings.clone();
            if pick == SpringPick::Chain {
                s.springs.chains.remove(index);
            } else {
                s.springs.colliders.remove(index);
            }
            self.out.push(Intent::SetImport {
                model: file.to_path_buf(),
                settings: s,
            });
        }
        y + 4.0
    }

    /// A labelled row of three import numbers (X Y Z).
    #[allow(clippy::too_many_arguments)]
    fn triple_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        id: u64,
        file: &Path,
        settings: &ImportSettings,
        editing: bool,
        label: &str,
        field: impl Fn(u8) -> (ImportField, f32),
    ) -> f32 {
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x + 10.0, y, label);
            let fx = x + layout::LABEL_W;
            let cw = ((w - layout::LABEL_W - 8.0) / 3.0).max(24.0);
            for axis in 0..3u8 {
                let r = Rect::new(fx + (cw + 4.0) * f32::from(axis), y, cw, layout::FIELD_H);
                let (which, value) = field(axis);
                self.import_field(canvas, r, id, which, value, file, settings, editing);
            }
        }
        y + layout::ROW_STEP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_set_their_setting_within_limits() {
        let mut s = ImportSettings::default();
        s.springs.chains.push(SpringChain::new("Hair1"));
        s.springs.colliders.push(SpringCollider::new("Head"));
        assert!(ImportField::Chain(0, ChainField::Stiffness).apply(&mut s, 2.0));
        assert!(ImportField::Chain(0, ChainField::Wind(2)).apply(&mut s, -1.5));
        assert!(ImportField::Collider(0, ColliderField::Offset(1)).apply(&mut s, 0.09));
        assert!(ImportField::Collider(0, ColliderField::Radius).apply(&mut s, -1.0));
        assert_eq!(s.springs.chains[0].stiffness, 1.0);
        assert_eq!(s.springs.chains[0].wind, [0.0, 0.0, -1.5]);
        assert_eq!(s.springs.colliders[0].offset, [0.0, 0.09, 0.0]);
        assert_eq!(s.springs.colliders[0].radius, 0.0);
        // Gone chains, zero scales and NaN are refused.
        assert!(!ImportField::Chain(3, ChainField::Drag).apply(&mut s, 0.5));
        assert!(!ImportField::Scale.apply(&mut s, 0.0));
        assert!(!ImportField::Rotation(0).apply(&mut s, f32::NAN));
    }
}
