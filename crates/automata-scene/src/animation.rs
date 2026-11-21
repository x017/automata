//! Clip playback for animated models.
//!
//! An [`Animator`] plays one of its model's clips; [`animate`] advances
//! every animator each frame and hands the resulting joint palette to the
//! entity's [`MeshRenderer`], whose skinned parts then draw posed (shadows
//! included).

use std::sync::Arc;

use automata_assets::{Model, Pose, Rig};
use glam::Mat4;

use crate::render::MeshRenderer;
use crate::scene::SceneWorld;

/// Plays an animated model's clips.
#[derive(Debug, Clone)]
pub struct Animator {
    rig: Arc<Rig>,
    clip: Option<usize>,
    /// Seconds into the current clip.
    pub time: f32,
    /// Playback rate (1 is normal speed, negative plays backwards).
    pub speed: f32,
    /// Whether the clip starts over when it ends.
    pub looping: bool,
    /// Whether time advances.
    pub playing: bool,
    pose: Pose,
}

impl Animator {
    /// An animator for `rig`, showing the rest pose.
    #[must_use]
    pub fn new(rig: Arc<Rig>) -> Self {
        let pose = rig.rest_pose();
        Self {
            rig,
            clip: None,
            time: 0.0,
            speed: 1.0,
            looping: true,
            playing: false,
            pose,
        }
    }

    /// An animator for `model`, or `None` when it has no rig.
    #[must_use]
    pub fn for_model(model: &Model) -> Option<Self> {
        model.rig.clone().map(Self::new)
    }

    /// The rig being animated.
    #[must_use]
    pub fn rig(&self) -> &Arc<Rig> {
        &self.rig
    }

    /// Starts `clip` from the beginning. Returns `false` (and changes
    /// nothing) when the model has no clip by that name.
    pub fn play(&mut self, clip: &str) -> bool {
        let Some(index) = self.rig.clips.iter().position(|c| c.name == clip) else {
            return false;
        };
        self.clip = Some(index);
        self.time = 0.0;
        self.playing = true;
        true
    }

    /// Stops playback and returns to the rest pose.
    pub fn stop(&mut self) {
        self.clip = None;
        self.time = 0.0;
        self.playing = false;
    }

    /// The name of the current clip.
    #[must_use]
    pub fn clip(&self) -> Option<&str> {
        self.clip
            .and_then(|i| self.rig.clips.get(i))
            .map(|c| c.name.as_str())
    }

    /// Advances time by `dt` seconds (scaled by [`Self::speed`]) and
    /// samples the pose. A clip that doesn't loop holds its last frame and
    /// stops playing.
    pub fn advance(&mut self, dt: f32) {
        let Some(clip) = self.clip.and_then(|i| self.rig.clips.get(i)) else {
            self.pose = self.rig.rest_pose();
            return;
        };
        if self.playing {
            self.time += dt * self.speed;
            let duration = clip.duration;
            if self.looping && duration > 0.0 {
                self.time = self.time.rem_euclid(duration);
            } else if self.time >= duration || self.time <= 0.0 && self.speed < 0.0 {
                self.time = self.time.clamp(0.0, duration);
                self.playing = false;
            }
        }
        self.pose = self.rig.rest_pose();
        clip.sample_into(self.time, &mut self.pose);
    }

    /// The current pose, one local transform per rig node.
    #[must_use]
    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// The joint matrices for the current pose.
    #[must_use]
    pub fn palette(&self) -> Arc<[Mat4]> {
        let mut out = Vec::with_capacity(self.rig.joints.len());
        self.rig.palette(&self.pose, &mut out);
        out.into()
    }
}

/// Advances every [`Animator`] by `dt` seconds and poses its entity's
/// [`MeshRenderer`]. Run once per frame after the game update.
pub fn animate(world: &mut SceneWorld, dt: f32) {
    for (animator, renderer) in world.query_mut::<(&mut Animator, Option<&mut MeshRenderer>)>() {
        animator.advance(dt);
        if let Some(renderer) = renderer {
            renderer.joints = Some(animator.palette());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_assets::anim::{Channel, Clip, Interpolation, Property, RigNode, Trs};
    use glam::{Quat, Vec3};

    fn rig() -> Arc<Rig> {
        Arc::new(Rig {
            nodes: vec![RigNode {
                name: "spin".into(),
                parent: None,
                rest: Trs::IDENTITY,
            }],
            joints: vec![(0, Mat4::IDENTITY)],
            import: Mat4::IDENTITY,
            clips: vec![Clip {
                name: "slide".into(),
                duration: 2.0,
                channels: vec![Channel {
                    node: 0,
                    property: Property::Translation,
                    interpolation: Interpolation::Linear,
                    times: vec![0.0, 2.0],
                    values: vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0],
                }],
            }],
        })
    }

    fn x(animator: &Animator) -> f32 {
        animator.palette()[0].w_axis.x
    }

    #[test]
    fn plays_loops_and_holds() {
        let mut a = Animator::new(rig());
        assert!(!a.play("missing"));
        assert!(a.play("slide"));
        a.advance(0.5);
        assert!((x(&a) - 0.5).abs() < 1e-5);
        a.advance(2.0); // 2.5 wraps to 0.5
        assert!((a.time - 0.5).abs() < 1e-5);

        a.looping = false;
        a.advance(5.0);
        assert!(!a.playing);
        assert!((x(&a) - 2.0).abs() < 1e-5, "holds the last frame");

        a.stop();
        a.advance(0.1);
        assert!(x(&a).abs() < 1e-6, "rest pose");
        assert_eq!(a.clip(), None);
        assert_eq!(a.pose().0[0].rotation, Quat::IDENTITY);
        assert_eq!(a.pose().0[0].translation, Vec3::ZERO);
    }

    #[test]
    fn animate_advances_animators_without_renderers() {
        let mut scene = crate::Scene::new();
        let mut a = Animator::new(rig());
        a.play("slide");
        let e = scene.spawn((a,));
        animate(scene.world_mut(), 1.0);
        let a = scene.world().get::<&Animator>(e).unwrap();
        assert!((a.time - 1.0).abs() < 1e-6);
    }
}
