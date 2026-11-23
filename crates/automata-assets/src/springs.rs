//! Spring bones: chains of bones (hair, skirts, tails, antennae) that swing
//! and settle on their own as the model moves, on top of its animation.
//!
//! They are set up per model file, in its import settings
//! (`<model>.import.ron`), so every entity showing the model gets them:
//!
//! ```ron
//! (
//!     springs: (
//!         chains: [
//!             (root: "Hair1", stiffness: 0.35, drag: 0.15, gravity: 0.6),
//!             (root: "Skirt"),
//!         ],
//!         colliders: [
//!             (bone: "Head", radius: 0.13),
//!             (bone: "ThighL", radius: 0.08, length: -0.4),
//!         ],
//!     ),
//! )
//! ```
//!
//! A chain is its root bone and everything below it. Each bone with one
//! child swings so it points at a simulated point (Verlet integration) that
//! is pulled back towards where the animation puts it; the last bone of a
//! branch swings too. A bone with several children (a skirt's waistband)
//! stays put and each child branch swings on its own. Colliders are spheres
//! or capsules on body bones that keep the chains out of the body.
//!
//! Sizes are in metres on the imported model (after import scale), and
//! grow with the entity's own scale. The simulation itself lives in
//! `automata_scene` (it needs the entity's movement).

use serde::{Deserialize, Serialize};

/// One chain of swinging bones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpringChain {
    /// The chain's first bone. It and every bone below it swing.
    pub root: String,
    /// How strongly the bones spring back to the animated pose: 0 is limp,
    /// 1 is stiff.
    pub stiffness: f32,
    /// How quickly swinging dies down, like air resistance: 0 swings the
    /// longest, 1 barely swings. (Stiff chains also damp themselves a little.)
    pub drag: f32,
    /// How much gravity pulls the chain down (1 is full gravity).
    pub gravity: f32,
    /// A steady wind on the chain, in m/s² (world X, Y, Z). It gusts
    /// around this.
    pub wind: [f32; 3],
    /// The bones' thickness for colliding with the body, in metres.
    pub radius: f32,
}

impl Default for SpringChain {
    fn default() -> Self {
        Self {
            root: String::new(),
            stiffness: 0.5,
            drag: 0.3,
            gravity: 0.5,
            wind: [0.0; 3],
            radius: 0.02,
        }
    }
}

impl SpringChain {
    /// A chain from `root` with the default feel.
    #[must_use]
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            ..Self::default()
        }
    }
}

/// A sphere or capsule on a body bone that chains can't pass through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpringCollider {
    /// The bone it moves with.
    pub bone: String,
    /// Its centre, from the bone's origin along the bone's own axes, in
    /// metres.
    pub offset: [f32; 3],
    /// Its radius in metres.
    pub radius: f32,
    /// 0 for a sphere. Otherwise a capsule this long along the bone's Y
    /// axis from `offset` (negative runs the other way).
    pub length: f32,
}

impl Default for SpringCollider {
    fn default() -> Self {
        Self {
            bone: String::new(),
            offset: [0.0; 3],
            radius: 0.1,
            length: 0.0,
        }
    }
}

impl SpringCollider {
    /// A sphere collider on `bone` with the default size.
    #[must_use]
    pub fn new(bone: impl Into<String>) -> Self {
        Self {
            bone: bone.into(),
            ..Self::default()
        }
    }
}

/// A model's spring bones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Springs {
    /// The swinging chains.
    pub chains: Vec<SpringChain>,
    /// What they collide with.
    pub colliders: Vec<SpringCollider>,
}

impl Springs {
    /// Whether there's nothing to simulate.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chains.is_empty()
    }

    /// Whether there are no chains and no colliders (nothing to save).
    #[must_use]
    pub fn is_unset(&self) -> bool {
        self.chains.is_empty() && self.colliders.is_empty()
    }
}

/// Bones that make good chain roots: the top bone of each branch that no
/// clip animates (hair, skirts and tails usually have no keys of their
/// own). Bones with several children are listed as they are, since every
/// child branch swings.
#[must_use]
pub fn suggested_roots(rig: &crate::anim::Rig) -> Vec<String> {
    let n = rig.nodes.len();
    let mut keyed = vec![false; n];
    for clip in &rig.clips {
        for channel in &clip.channels {
            if let Some(k) = keyed.get_mut(channel.node) {
                *k = true;
            }
        }
    }
    // Whether anything in each node's subtree is keyed (children come
    // after their parents, so walk backwards).
    let mut subtree = keyed.clone();
    for i in (0..n).rev() {
        if let Some(p) = rig.nodes[i].parent {
            if subtree[i] && p < n {
                subtree[p] = true;
            }
        }
    }
    let has_children = |i: usize| rig.nodes.iter().any(|m| m.parent == Some(i));
    let jointed: Vec<bool> = (0..n)
        .map(|i| rig.joints.iter().any(|&(node, _)| node == i))
        .collect();
    (0..n)
        .filter(|&i| {
            let parent_keyed = rig.nodes[i].parent.is_some_and(|p| subtree[p]);
            !subtree[i] && parent_keyed && jointed[i] && has_children(i)
        })
        .map(|i| rig.nodes[i].name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Channel, Clip, Interpolation, Property, Rig, RigNode, Trs};
    use glam::Mat4;

    fn node(name: &str, parent: Option<usize>) -> RigNode {
        RigNode {
            name: name.into(),
            parent,
            rest: Trs::IDENTITY,
        }
    }

    #[test]
    fn unkeyed_branches_are_suggested_as_roots() {
        // Hips (keyed) → Head (keyed) → Hair1 → Hair2; Hips → Skirt → Panel.
        let nodes = vec![
            node("Hips", None),
            node("Head", Some(0)),
            node("Hair1", Some(1)),
            node("Hair2", Some(2)),
            node("Skirt", Some(0)),
            node("Panel", Some(4)),
            node("Tip", Some(5)),
        ];
        let key = |node| Channel {
            node,
            property: Property::Rotation,
            interpolation: Interpolation::Linear,
            times: vec![0.0],
            values: vec![0.0, 0.0, 0.0, 1.0],
        };
        let rig = Rig {
            joints: (0..nodes.len()).map(|i| (i, Mat4::IDENTITY)).collect(),
            nodes,
            import: Mat4::IDENTITY,
            clips: vec![Clip {
                name: "Run".into(),
                duration: 1.0,
                channels: vec![key(0), key(1)],
            }],
            springs: Springs::default(),
        };
        assert_eq!(suggested_roots(&rig), ["Hair1", "Skirt"]);
    }

    #[test]
    fn partial_settings_fill_in_defaults() {
        let s: Springs = ron::from_str("(chains: [(root: \"Tail\", drag: 0.5)])").unwrap();
        assert_eq!(s.chains[0].root, "Tail");
        assert!((s.chains[0].drag - 0.5).abs() < 1e-6);
        assert!((s.chains[0].stiffness - 0.5).abs() < 1e-6);
        assert!(s.colliders.is_empty() && !s.is_empty());
        assert!(Springs::default().is_unset());
    }
}
