//! Clip playback for animated models.
//!
//! An [`Animator`] plays one of its model's clips, crossfading from the
//! previous one when asked; [`animate`] advances every animator each frame
//! and hands the resulting joint palette to the entity's [`MeshRenderer`],
//! whose skinned parts then draw posed (shadows included).
//!
//! The [`Animation`] component is the saved, editable side: which clip a
//! model plays when the scene starts, how fast, and whether it loops.
//! [`animate`] gives each entity with an `Animation` and a rigged model
//! its animator the first time it runs, and scripts steer it with
//! [`AnimationCommand`]s ([`command_animation`]).
//!
//! Clips can carry events (named moments from the model's import
//! settings): the animator notes each one its playhead passes, and scripts
//! hear about them. With [`Animator::root_motion`] on, a clip's own travel
//! across the ground moves the entity instead of the pose: through its
//! character controller when it has one ([`CharacterMotion`]), otherwise by
//! moving its transform.
//!
//! An animator can also run an animation graph (`Animation::graph`, a
//! `.anim.ron` file): then the graph picks and blends the clips from the
//! parameters scripts set ([`AnimationCommand::Set`],
//! [`AnimationCommand::Trigger`]), and `play` jumps to a state (see
//! [`crate::anim_graph`]).

use std::sync::Arc;

use automata_assets::anim_graph::AnimGraph;
use automata_assets::{Clip, ClipMotion, Model, Pose, Rig};
use glam::{Mat4, Vec3};
use hecs::Entity;
use serde::{Deserialize, Serialize};

use crate::anim_graph::{GraphPlayer, GraphState, Sampler};
use crate::render::{MeshRenderer, MeshVisual};
use crate::scene::SceneWorld;
use crate::springs::SpringSim;
use crate::transform::{GlobalTransform, Transform};

/// How many fired events an animator remembers for scripts to catch up on.
const RECENT_EVENTS: usize = 32;

/// The event fired when a clip that doesn't loop reaches its end.
pub const END_EVENT: &str = "end";

/// What an animated model plays (also its ECS component and its record in
/// scene files). The model itself comes from the entity's mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    /// The clip shown when the scene starts (empty: the rest pose).
    pub clip: String,
    /// Playback rate (1 is normal speed, negative plays backwards).
    pub speed: f32,
    /// Whether the clip starts over when it ends.
    pub looping: bool,
    /// Whether the clip plays as soon as the scene runs. Off, the model
    /// holds the clip's first frame until a script plays something.
    pub play_on_start: bool,
    /// Root motion: the clip's own travel across the ground moves the
    /// entity (through its character controller, if it has one) instead
    /// of sliding the model away from it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub root_motion: bool,
    /// An animation graph (`.anim.ron`, a path like a model's) that picks
    /// the clips from parameters scripts set; empty plays `clip`. With a
    /// graph, `speed` scales the graph's own speeds.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub graph: String,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            clip: String::new(),
            speed: 1.0,
            looping: true,
            play_on_start: true,
            root_motion: false,
            graph: String::new(),
        }
    }
}

impl Animation {
    /// Plays `clip` on start, looping at normal speed.
    #[must_use]
    pub fn new(clip: impl Into<String>) -> Self {
        Self {
            clip: clip.into(),
            ..Self::default()
        }
    }

    /// The clip a new Animation on `rig` should show: the first one whose
    /// name contains "idle" (any case), else the first clip.
    #[must_use]
    pub fn default_clip(rig: &Rig) -> Option<&str> {
        rig.clips
            .iter()
            .find(|c| c.name.to_ascii_lowercase().contains("idle"))
            .or_else(|| rig.clips.first())
            .map(|c| c.name.as_str())
    }
}

/// How [`Animator::play_with`] starts a clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayOptions {
    /// Whether the clip starts over when it ends.
    pub looping: bool,
    /// A new playback rate, or `None` to keep the current one.
    pub speed: Option<f32>,
    /// Seconds to crossfade from the current pose (0 cuts straight over).
    pub fade: f32,
    /// Start over even when the clip is already playing (otherwise playing
    /// the current clip again just keeps it going).
    pub restart: bool,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            looping: true,
            speed: None,
            fade: 0.0,
            restart: false,
        }
    }
}

/// One clip's playhead.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layer {
    clip: usize,
    time: f32,
    speed: f32,
    looping: bool,
    playing: bool,
}

impl Layer {
    /// How far (in clip seconds, signed) a step of `dt` moves the playhead.
    fn advance(&self, dt: f32) -> f32 {
        if self.playing {
            dt * self.speed
        } else {
            0.0
        }
    }

    /// Moves the playhead `dt` seconds (scaled by speed). A clip that
    /// doesn't loop holds its last (or, backwards, first) frame and stops.
    fn step(&mut self, duration: f32, dt: f32) {
        if !self.playing {
            return;
        }
        self.time += dt * self.speed;
        if self.looping && duration > 0.0 {
            self.time = self.time.rem_euclid(duration);
        } else if self.time >= duration || self.time <= 0.0 && self.speed < 0.0 {
            self.time = self.time.clamp(0.0, duration);
            self.playing = false;
        }
    }
}

/// What a crossfade blends away from.
#[derive(Debug, Clone)]
enum FadeFrom {
    /// The previous clip, still playing underneath.
    Clip(Layer),
    /// A frozen pose (the rest pose, or a fade interrupted mid-way).
    Pose(Pose),
}

#[derive(Debug, Clone)]
struct Fade {
    from: FadeFrom,
    duration: f32,
    elapsed: f32,
}

/// An event an animator's playhead passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiredEvent {
    /// Counts up from 1 per animator, so a reader can tell which events
    /// it has already seen.
    pub seq: u64,
    /// The clip it's on.
    pub clip: String,
    /// The event's name ([`END_EVENT`] when a one-shot clip ends).
    pub name: String,
}

/// Component: root motion waiting for the entity's character controller.
/// The physics world adds it to every character and moves the character by
/// `pending` (colliding) on its next step; entities without one are moved
/// by setting their transform instead.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CharacterMotion {
    /// World-space movement not yet applied.
    pub pending: Vec3,
}

impl CharacterMotion {
    /// The pending movement, leaving none.
    pub fn take(&mut self) -> Vec3 {
        std::mem::take(&mut self.pending)
    }
}

/// The events of `clip` a playhead passes going from `from` by `advance`
/// seconds (negative backwards): `(how far along, name)`. `inclusive`
/// also counts an event exactly at `from` (a clip that just started).
pub(crate) fn crossed<'c>(
    clip: &'c Clip,
    from: f32,
    advance: f32,
    looping: bool,
    inclusive: bool,
    out: &mut Vec<(f32, &'c str)>,
) {
    let d = clip.duration;
    let to = from + advance;
    for e in &clip.events {
        if looping && d > 0.0 {
            // Every lap's copy of the event inside the swept span (at most
            // a few, should a huge step sweep many laps).
            if advance > 0.0 {
                let mut n = ((from - e.time) / d).ceil();
                if e.time + n * d <= from && !inclusive {
                    n += 1.0;
                }
                for _ in 0..4 {
                    let t = e.time + n * d;
                    if t > to {
                        break;
                    }
                    out.push((t - from, &e.name));
                    n += 1.0;
                }
            } else {
                let mut n = ((from - e.time) / d).floor();
                if e.time + n * d >= from && !inclusive {
                    n -= 1.0;
                }
                for _ in 0..4 {
                    let t = e.time + n * d;
                    if t < to {
                        break;
                    }
                    out.push((from - t, &e.name));
                    n -= 1.0;
                }
            }
        } else if advance > 0.0 {
            let after = e.time > from || inclusive && e.time >= from;
            if after && e.time <= to.min(d) {
                out.push((e.time - from, &e.name));
            }
        } else {
            let before = e.time < from || inclusive && e.time <= from;
            if before && e.time >= to.max(0.0) {
                out.push((from - e.time, &e.name));
            }
        }
    }
}

/// Plays an animated model's clips.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // independent switches, not states
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
    /// Root motion: the clip's travel across the ground is taken out of
    /// the pose and reported by [`Self::travel`] for [`animate`] to move
    /// the entity by.
    pub root_motion: bool,
    fade: Option<Fade>,
    pose: Pose,
    springs: Option<SpringSim>,
    /// Each clip's ground travel (root motion), by clip index.
    motions: Arc<[Option<ClipMotion>]>,
    /// The ground travel of the last advance, in model space.
    travel: Vec3,
    /// The latest events passed, oldest first.
    events: Vec<FiredEvent>,
    next_event: u64,
    /// The clip has just started: an event at its first moment fires.
    fresh: bool,
    /// The animation graph picking the clips, and its path.
    graph: Option<(String, GraphPlayer)>,
    /// [`automata_assets::anim_graph_generation`] when the graph was last
    /// loaded: when it moves, an edited graph is picked up.
    graph_gen: u64,
}

impl Animator {
    /// An animator for `rig`, showing the rest pose.
    #[must_use]
    pub fn new(rig: Arc<Rig>) -> Self {
        let pose = rig.rest_pose();
        let springs = SpringSim::new(&rig);
        let motions = rig.clips.iter().map(|c| ClipMotion::of(&rig, c)).collect();
        Self {
            rig,
            clip: None,
            time: 0.0,
            speed: 1.0,
            looping: true,
            playing: false,
            root_motion: false,
            fade: None,
            pose,
            springs,
            motions,
            travel: Vec3::ZERO,
            events: Vec::new(),
            next_event: 1,
            fresh: false,
            graph: None,
            graph_gen: 0,
        }
    }

    /// An animator for `model`, or `None` when it has no rig.
    #[must_use]
    pub fn for_model(model: &Model) -> Option<Self> {
        model.rig.clone().map(Self::new)
    }

    /// An animator for `rig` set up from an [`Animation`] component:
    /// playing its clip when `autoplay` and `play_on_start`, otherwise
    /// holding the clip's first frame.
    ///
    /// With a graph that loads, the graph runs instead (when `autoplay`
    /// and `play_on_start`; otherwise the model holds the entry state's
    /// first frame). One that doesn't load is logged and the clip plays.
    #[must_use]
    pub fn from_component(rig: Arc<Rig>, animation: &Animation, autoplay: bool) -> Self {
        let mut a = Self::new(rig);
        a.speed = animation.speed;
        a.looping = animation.looping;
        a.root_motion = animation.root_motion;
        if !animation.graph.is_empty() {
            match automata_assets::load_anim_graph(&animation.graph) {
                Ok(graph) => {
                    a.set_graph(&animation.graph, graph);
                    a.advance_clip(0.0);
                    a.playing = autoplay && animation.play_on_start;
                    return a;
                }
                Err(e) => tracing::warn!("{e}; playing the clip instead"),
            }
        }
        if !animation.clip.is_empty() {
            if autoplay && animation.play_on_start {
                a.play(&animation.clip);
            } else {
                a.pose_at(&animation.clip, 0.0);
            }
        }
        a
    }

    /// The rig being animated.
    #[must_use]
    pub fn rig(&self) -> &Arc<Rig> {
        &self.rig
    }

    /// The names of the model's clips.
    #[must_use]
    pub fn clip_names(&self) -> Vec<&str> {
        self.rig.clip_names()
    }

    /// Runs `graph` (loaded from `path`) from its entry states, instead of
    /// playing clips directly. What doesn't fit the model is logged.
    pub fn set_graph(&mut self, path: &str, graph: Arc<AnimGraph>) {
        let player = GraphPlayer::new(graph, &self.rig);
        for problem in player.problems() {
            tracing::warn!("animation graph {path}: {problem}");
        }
        self.fade = None;
        self.playing = true;
        self.graph = Some((path.to_string(), player));
        self.graph_gen = automata_assets::anim_graph_generation();
    }

    /// Picks up an edited graph (an editor's unsaved changes coming or
    /// going), carrying on where it was.
    fn reload_graph(&mut self) {
        let generation = automata_assets::anim_graph_generation();
        if generation == self.graph_gen {
            return;
        }
        self.graph_gen = generation;
        let Some((path, player)) = &mut self.graph else {
            return;
        };
        match automata_assets::load_anim_graph(path) {
            Ok(graph) if !Arc::ptr_eq(&graph, player.graph()) => {
                player.reload(graph, &self.rig);
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("{e}; keeping the graph as it was"),
        }
    }

    /// Stops running the graph (the pose stays until something plays).
    pub fn clear_graph(&mut self) {
        if self.graph.take().is_some() {
            self.clip = None;
            self.playing = false;
        }
    }

    /// The graph being run and its path.
    #[must_use]
    pub fn graph(&self) -> Option<(&str, &GraphPlayer)> {
        self.graph.as_ref().map(|(p, g)| (p.as_str(), g))
    }

    /// The graph being run, to set parameters on.
    pub fn graph_mut(&mut self) -> Option<&mut GraphPlayer> {
        self.graph.as_mut().map(|(_, g)| g)
    }

    fn find(&self, clip: &str) -> Option<usize> {
        self.rig.clips.iter().position(|c| c.name == clip)
    }

    fn current(&self) -> Option<&Clip> {
        self.clip.and_then(|i| self.rig.clips.get(i))
    }

    /// Starts `clip` from the beginning. Returns `false` (and changes
    /// nothing) when the model has no clip by that name.
    pub fn play(&mut self, clip: &str) -> bool {
        let looping = self.looping;
        self.play_with(
            clip,
            PlayOptions {
                looping,
                restart: true,
                ..PlayOptions::default()
            },
        )
    }

    /// Plays `clip` as `options` say, crossfading from the current pose
    /// over `options.fade` seconds. Playing the clip that's already playing
    /// keeps it going (taking the new loop and speed settings) unless
    /// `options.restart`. Returns `false` (and changes nothing) when the
    /// model has no clip by that name.
    ///
    /// With a graph, `clip` names a state of its first layer instead, and
    /// the graph goes straight there.
    pub fn play_with(&mut self, clip: &str, options: PlayOptions) -> bool {
        if let Some((_, graph)) = &mut self.graph {
            self.playing = true;
            return graph.play_state("", clip, options.fade);
        }
        let Some(index) = self.find(clip) else {
            return false;
        };
        let speed = options.speed.filter(|s| s.is_finite());
        if self.clip == Some(index) && self.playing && !options.restart {
            self.speed = speed.unwrap_or(self.speed);
            self.looping = options.looping;
            return true;
        }
        // The outgoing clip fades with its own speed and looping.
        self.begin_fade(options.fade);
        self.speed = speed.unwrap_or(self.speed);
        self.looping = options.looping;
        self.clip = Some(index);
        let duration = self.rig.clips[index].duration;
        self.time = if self.speed < 0.0 { duration } else { 0.0 };
        self.playing = true;
        self.fresh = true;
        true
    }

    /// Stops playback and returns to the rest pose.
    pub fn stop(&mut self) {
        self.stop_with(0.0);
    }

    /// Stops playback, fading back to the rest pose over `fade` seconds.
    /// A graph stops running.
    pub fn stop_with(&mut self, fade: f32) {
        if self.graph.take().is_some() {
            self.clip = None; // fade from the pose shown
        }
        self.begin_fade(fade);
        self.clip = None;
        self.time = 0.0;
        self.playing = false;
    }

    /// Remembers what the pose is blending away from.
    fn begin_fade(&mut self, fade: f32) {
        self.fade = (fade.is_finite() && fade > 0.0).then(|| Fade {
            from: match self.clip {
                // Mid-fade, or nothing playing: freeze what's shown.
                Some(clip) if self.fade.is_none() => FadeFrom::Clip(Layer {
                    clip,
                    time: self.time,
                    speed: self.speed,
                    looping: self.looping,
                    playing: self.playing,
                }),
                _ => FadeFrom::Pose(self.pose.clone()),
            },
            duration: fade,
            elapsed: 0.0,
        });
    }

    /// Jumps to `time` seconds into the current clip (wrapped when it
    /// loops, clamped otherwise).
    pub fn seek(&mut self, time: f32) {
        let Some(duration) = self.current().map(|c| c.duration) else {
            return;
        };
        if !time.is_finite() {
            return;
        }
        self.time = if self.looping && duration > 0.0 {
            time.rem_euclid(duration)
        } else {
            time.clamp(0.0, duration)
        };
        self.sample();
    }

    /// Shows `clip` frozen at `time` seconds (no fade, not playing): the
    /// editor's preview scrubber. Returns `false` for an unknown clip.
    pub fn pose_at(&mut self, clip: &str, time: f32) -> bool {
        let Some(index) = self.find(clip) else {
            return false;
        };
        self.clip = Some(index);
        self.fade = None;
        self.playing = false;
        let duration = self.rig.clips[index].duration;
        self.time = if time.is_finite() {
            time.clamp(0.0, duration)
        } else {
            0.0
        };
        self.sample();
        true
    }

    /// The name of the current clip.
    #[must_use]
    pub fn clip(&self) -> Option<&str> {
        self.current().map(|c| c.name.as_str())
    }

    /// The current clip's length in seconds (0 with no clip).
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.current().map_or(0.0, |c| c.duration)
    }

    /// Whether a crossfade is still blending.
    #[must_use]
    pub fn fading(&self) -> bool {
        self.fade.is_some()
    }

    /// Samples the current clip (or the rest pose) into `self.pose`.
    fn sample(&mut self) {
        self.pose = self.rig.rest_pose();
        if let Some(i) = self.clip {
            sample_clip(
                &self.rig,
                &self.motions,
                i,
                self.time,
                self.root_motion,
                &mut self.pose,
            );
        }
    }

    /// The clip's ground travel during the last advance, in model space
    /// (zero unless [`Self::root_motion`]).
    #[must_use]
    pub fn travel(&self) -> Vec3 {
        if self.root_motion {
            self.travel
        } else {
            Vec3::ZERO
        }
    }

    /// The latest events the playhead passed (up to 32), oldest first.
    /// Their `seq` tells a reader which ones are new to it.
    #[must_use]
    pub fn recent_events(&self) -> &[FiredEvent] {
        &self.events
    }

    fn fire(&mut self, clip: usize, name: &str) {
        let clip = self.rig.clips[clip].name.clone();
        self.events.push(FiredEvent {
            seq: self.next_event,
            clip,
            name: name.to_string(),
        });
        self.next_event += 1;
        if self.events.len() > RECENT_EVENTS {
            self.events.remove(0);
        }
    }

    /// Advances time by `dt` seconds (scaled by [`Self::speed`]) and
    /// samples the pose, blending in any crossfade. A clip that doesn't
    /// loop holds its last frame and stops playing. Spring bones (if the
    /// model has any) swing as if the entity stood still at the origin;
    /// [`Self::advance_in`] gives them the entity's place.
    pub fn advance(&mut self, dt: f32) {
        self.advance_in(dt, Mat4::IDENTITY);
    }

    /// [`Self::advance`] with the entity at `world`, so spring bones swing
    /// as it moves.
    pub fn advance_in(&mut self, dt: f32, world: Mat4) {
        self.advance_clip(dt);
        if let Some(springs) = &mut self.springs {
            springs.update(&self.rig, &mut self.pose, world, dt);
        }
    }

    /// Whether the model has spring bones.
    #[must_use]
    pub fn has_springs(&self) -> bool {
        self.springs.is_some()
    }

    /// Sets the scene wind its spring bones blow in (see
    /// [`SpringSim::set_wind`]).
    pub fn set_wind(&mut self, wind: Vec3) {
        if let Some(springs) = &mut self.springs {
            springs.set_wind(wind);
        }
    }

    /// The spring bones' state, if the model has any.
    #[must_use]
    pub fn springs(&self) -> Option<&SpringSim> {
        self.springs.as_ref()
    }

    /// Stops spring bones swinging: they start again from the animated
    /// pose (after moving the entity somewhere else at once, say).
    pub fn reset_springs(&mut self) {
        if let Some(springs) = &mut self.springs {
            springs.reset();
        }
    }

    /// Runs the graph for `dt` seconds (when one is set and playing).
    /// Returns whether it did.
    fn advance_graph(&mut self, dt: f32) -> bool {
        self.reload_graph();
        let Some((_, graph)) = &mut self.graph else {
            return false;
        };
        let (rig, motions) = (self.rig.clone(), self.motions.clone());
        let sampler = Sampler {
            rig: &rig,
            motions: &motions,
            root_motion: self.root_motion,
        };
        let dt = if self.playing { dt } else { 0.0 };
        let out = graph.advance(&sampler, dt, self.speed);
        let current = graph.current_clip(&rig);
        self.clip = current.map(|(c, _)| c);
        self.time = current.map_or(0.0, |(_, t)| t);
        self.pose = out.pose;
        self.travel = out.travel;
        for (clip, name) in out.events {
            self.fire(clip, &name);
        }
        true
    }

    fn advance_clip(&mut self, dt: f32) {
        if self.advance_graph(dt) {
            return;
        }
        let mut travel = Vec3::ZERO;
        if let Some(clip) = self.clip {
            let mut layer = Layer {
                clip,
                time: self.time,
                speed: self.speed,
                looping: self.looping,
                playing: self.playing,
            };
            let (from, advance) = (layer.time, layer.advance(dt));
            layer.step(self.rig.clips[clip].duration, dt);
            if advance != 0.0 {
                let c = &self.rig.clips[clip];
                let mut passed = Vec::new();
                crossed(c, from, advance, layer.looping, self.fresh, &mut passed);
                passed.sort_by(|a, b| a.0.total_cmp(&b.0));
                let names: Vec<String> = passed.iter().map(|(_, n)| (*n).to_string()).collect();
                for name in names {
                    self.fire(clip, &name);
                }
                travel = layer_travel(&self.rig, &self.motions, clip, from, advance, layer.looping);
            }
            if self.playing && !layer.playing {
                self.fire(clip, END_EVENT);
            }
            self.time = layer.time;
            self.playing = layer.playing;
        }
        self.fresh = false;
        self.sample();
        self.travel = travel;
        let Some(fade) = &mut self.fade else {
            return;
        };
        fade.elapsed += dt.abs();
        let t = (fade.elapsed / fade.duration).clamp(0.0, 1.0);
        if t >= 1.0 {
            self.fade = None;
            return;
        }
        let (rig, motions) = (&self.rig, &self.motions);
        let mut from_travel = Vec3::ZERO;
        let mut from = match &mut fade.from {
            FadeFrom::Pose(pose) => pose.clone(),
            FadeFrom::Clip(layer) => {
                let (start, advance) = (layer.time, layer.advance(dt));
                layer.step(rig.clips[layer.clip].duration, dt);
                from_travel = layer_travel(rig, motions, layer.clip, start, advance, layer.looping);
                let mut pose = rig.rest_pose();
                sample_clip(
                    rig,
                    motions,
                    layer.clip,
                    layer.time,
                    self.root_motion,
                    &mut pose,
                );
                pose
            }
        };
        // Ease in and out, so the blend doesn't start or end with a jolt.
        let w = t * t * (3.0 - 2.0 * t);
        from.blend(&self.pose, w);
        self.pose = from;
        self.travel = from_travel.lerp(travel, w);
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

    /// What scripts see of this animator.
    #[must_use]
    pub fn state(&self) -> AnimationState {
        AnimationState {
            clip: self.clip().map(str::to_string),
            time: self.time,
            duration: self.duration(),
            playing: self.playing,
            speed: self.speed,
            looping: self.looping,
            clips: self.clip_names().into_iter().map(str::to_string).collect(),
            lengths: self.rig.clips.iter().map(|c| c.duration).collect(),
            springs: self.has_springs(),
            root_motion: self.root_motion,
            events: self.events.clone(),
            graph_path: self.graph.as_ref().map(|(p, _)| p.clone()),
            graph: self.graph.as_ref().map(|(_, g)| g.state()),
        }
    }
}

/// Samples clip `index` at `time` into `pose`, pinning its ground travel
/// in place when `root_motion`.
pub(crate) fn sample_clip(
    rig: &Rig,
    motions: &[Option<ClipMotion>],
    index: usize,
    time: f32,
    root_motion: bool,
    pose: &mut Pose,
) {
    let Some(clip) = rig.clips.get(index) else {
        return;
    };
    clip.sample_into(time, pose);
    if root_motion {
        if let Some(Some(motion)) = motions.get(index) {
            motion.pin(clip, time, pose);
        }
    }
}

/// The ground travel of clip `index` from `from` by `advance` seconds.
pub(crate) fn layer_travel(
    rig: &Rig,
    motions: &[Option<ClipMotion>],
    index: usize,
    from: f32,
    advance: f32,
    looping: bool,
) -> Vec3 {
    match (rig.clips.get(index), motions.get(index)) {
        (Some(clip), Some(Some(motion))) => motion.travel(clip, from, advance, looping),
        _ => Vec3::ZERO,
    }
}

/// An animated entity's playback, as scripts read it.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // independent switches, not states
pub struct AnimationState {
    /// The current clip, or `None` in the rest pose.
    pub clip: Option<String>,
    /// Seconds into the current clip.
    pub time: f32,
    /// The current clip's length in seconds (0 with no clip).
    pub duration: f32,
    /// Whether time advances (false once a one-shot clip ends).
    pub playing: bool,
    /// Playback rate.
    pub speed: f32,
    /// Whether the clip starts over when it ends.
    pub looping: bool,
    /// Every clip the model has.
    pub clips: Vec<String>,
    /// Each clip's length in seconds (in the order of `clips`).
    pub lengths: Vec<f32>,
    /// Whether the model has spring bones.
    pub springs: bool,
    /// Whether root motion is on.
    pub root_motion: bool,
    /// The latest events passed (see [`Animator::recent_events`]).
    pub events: Vec<FiredEvent>,
    /// The animation graph's path, when one runs.
    pub graph_path: Option<String>,
    /// The graph's states and parameters, when one runs.
    pub graph: Option<GraphState>,
}

/// A script's request to an entity's animation.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationCommand {
    /// Play a clip (see [`Animator::play_with`]).
    Play {
        /// The clip's name.
        clip: String,
        /// How to play it.
        options: PlayOptions,
    },
    /// Stop, fading to the rest pose over this many seconds.
    Stop(f32),
    /// Jump to this many seconds into the current clip.
    Seek(f32),
    /// Change the playback rate.
    Speed(f32),
    /// Put the spring bones back at rest (see [`Animator::reset_springs`]).
    ResetSprings,
    /// Turn root motion on or off (see [`Animator::root_motion`]).
    RootMotion(bool),
    /// Run this animation graph (a `.anim.ron` path), or none with "".
    Graph(String),
    /// Set a graph parameter (see [`GraphPlayer::set`]).
    Set(String, f32),
    /// Set a graph trigger (see [`GraphPlayer::trigger`]).
    Trigger(String),
    /// Clear a graph trigger nothing used.
    ResetTrigger(String),
    /// Set a graph layer's weight.
    LayerWeight(String, f32),
    /// Go straight to a state of a graph layer ("" for the first layer).
    PlayState {
        /// The layer's name.
        layer: String,
        /// The state's name.
        state: String,
        /// Seconds to crossfade.
        fade: f32,
    },
}

/// A new animator for `entity`'s rigged model (set up from its
/// [`Animation`], if any), or `None` without one.
fn new_animator(world: &SceneWorld, entity: Entity, autoplay: bool) -> Option<Animator> {
    let rig = world
        .get::<&MeshVisual>(entity)
        .ok()?
        .model
        .as_ref()?
        .rig
        .clone()?;
    Some(match world.get::<&Animation>(entity) {
        Ok(animation) => Animator::from_component(rig, &animation, autoplay),
        Err(_) => Animator::new(rig),
    })
}

/// Gives every entity with an [`Animation`] and a rigged model, or a model
/// with spring bones, its [`Animator`] (if it has none yet), playing its
/// clip when `autoplay`.
/// [`animate`] does this itself; editors call it with `autoplay` off to
/// show each model's first frame. Returns how many were added.
pub fn attach_animators(world: &mut SceneWorld, autoplay: bool) -> usize {
    let pending: Vec<Entity> = world
        .query::<hecs::Without<(Entity, &MeshVisual, Option<&Animation>), &Animator>>()
        .iter()
        .filter(|(_, visual, animation)| {
            let rig = visual.model.as_ref().and_then(|m| m.rig.as_ref());
            rig.is_some_and(|rig| animation.is_some() || !rig.springs.is_empty())
        })
        .map(|(e, _, _)| e)
        .collect();
    let mut added = 0;
    for entity in pending {
        if let Some(animator) = new_animator(world, entity, autoplay) {
            if world.insert_one(entity, animator).is_ok() {
                added += 1;
            }
        }
    }
    added
}

/// Applies a script's request to `entity`'s animation, giving a rigged
/// model an animator first if it has none. Returns `false` when the entity
/// can't animate or the clip doesn't exist.
pub fn command_animation(
    world: &mut SceneWorld,
    entity: Entity,
    command: &AnimationCommand,
) -> bool {
    if world.get::<&Animator>(entity).is_err() {
        let Some(animator) = new_animator(world, entity, true) else {
            return false;
        };
        if world.insert_one(entity, animator).is_err() {
            return false;
        }
    }
    let Ok(mut animator) = world.get::<&mut Animator>(entity) else {
        return false;
    };
    match command {
        AnimationCommand::Play { clip, options } => animator.play_with(clip, *options),
        AnimationCommand::Stop(fade) => {
            animator.stop_with(*fade);
            true
        }
        AnimationCommand::Seek(time) => {
            animator.seek(*time);
            true
        }
        AnimationCommand::Speed(speed) => {
            if speed.is_finite() {
                animator.speed = *speed;
            }
            speed.is_finite()
        }
        AnimationCommand::ResetSprings => {
            animator.reset_springs();
            true
        }
        AnimationCommand::RootMotion(on) => {
            animator.root_motion = *on;
            true
        }
        AnimationCommand::Graph(path) if path.is_empty() => {
            animator.clear_graph();
            true
        }
        AnimationCommand::Graph(path) => match automata_assets::load_anim_graph(path) {
            Ok(graph) => {
                animator.set_graph(path, graph);
                true
            }
            Err(e) => {
                tracing::warn!("{e}");
                false
            }
        },
        AnimationCommand::Set(name, value) => {
            animator.graph_mut().is_some_and(|g| g.set(name, *value))
        }
        AnimationCommand::Trigger(name) => animator.graph_mut().is_some_and(|g| g.trigger(name)),
        AnimationCommand::ResetTrigger(name) => {
            animator.graph_mut().is_some_and(|g| g.reset_trigger(name))
        }
        AnimationCommand::LayerWeight(layer, weight) => animator
            .graph_mut()
            .is_some_and(|g| g.set_layer_weight(layer, *weight)),
        AnimationCommand::PlayState { layer, state, fade } => {
            animator.playing = true;
            animator
                .graph_mut()
                .is_some_and(|g| g.play_state(layer, state, *fade))
        }
    }
}

/// `entity`'s playback as scripts see it: its animator's, or (before one
/// is attached) its rigged model's clips at rest. `None` when the entity
/// has no rigged model.
#[must_use]
pub fn animation_state(world: &SceneWorld, entity: Entity) -> Option<AnimationState> {
    if let Ok(animator) = world.get::<&Animator>(entity) {
        return Some(animator.state());
    }
    new_animator(world, entity, true).map(|a| a.state())
}

/// Advances every [`Animator`] by `dt` seconds and poses its entity's
/// [`MeshRenderer`]. Run once per frame after the game update. Entities
/// with an [`Animation`] get their animator here the first time.
///
/// Root motion moves its entity by the clip's travel (turned and scaled
/// like the entity, kept level): into [`CharacterMotion`] for characters,
/// onto the [`Transform`] otherwise.
pub fn animate(world: &mut SceneWorld, dt: f32) {
    attach_animators(world, true);
    // Spring bones blow in the scene's wind.
    let wind = (&mut world.query::<&automata_renderer::Environment>())
        .into_iter()
        .next()
        .map_or(Vec3::ZERO, automata_renderer::Environment::wind_vector);
    let mut moves: Vec<(Entity, Vec3)> = Vec::new();
    for (entity, animator, renderer, place) in world.query_mut::<(
        Entity,
        &mut Animator,
        Option<&mut MeshRenderer>,
        Option<&GlobalTransform>,
    )>() {
        let place = place.map_or(Mat4::IDENTITY, |g| g.0);
        animator.set_wind(wind);
        animator.advance_in(dt, place);
        if let Some(renderer) = renderer {
            renderer.joints = Some(animator.palette());
        }
        let travel = animator.travel();
        if travel != Vec3::ZERO {
            let mut step = place.transform_vector3(travel);
            step.y = 0.0;
            moves.push((entity, step));
        }
    }
    for (entity, step) in moves {
        if let Ok(mut motion) = world.get::<&mut CharacterMotion>(entity) {
            motion.pending += step;
        } else if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
            transform.position += step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_assets::anim::{Channel, Interpolation, Property, RigNode, Trs};
    use glam::{Quat, Vec3};

    fn slide(name: &str, to: f32) -> Clip {
        Clip {
            name: name.into(),
            duration: 2.0,
            channels: vec![Channel {
                node: 0,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 2.0],
                values: vec![0.0, 0.0, 0.0, to, 0.0, 0.0],
            }],
            events: Vec::new(),
        }
    }

    fn rig() -> Arc<Rig> {
        Arc::new(Rig {
            nodes: vec![RigNode {
                name: "spin".into(),
                parent: None,
                rest: Trs::IDENTITY,
            }],
            joints: vec![(0, Mat4::IDENTITY)],
            import: Mat4::IDENTITY,
            clips: vec![slide("slide", 2.0), slide("back", -2.0)],
            springs: automata_assets::springs::Springs::default(),
        })
    }

    fn x(animator: &Animator) -> f32 {
        animator.palette()[0].w_axis.x
    }

    #[test]
    fn runs_a_graph_from_its_file() {
        use automata_assets::anim_graph::{Condition, GraphLayer, Parameter, State, Transition};
        let dir = std::env::temp_dir().join(format!("automata-graph-{}", std::process::id()));
        let path = dir.join("slide.anim.ron");
        let graph = AnimGraph {
            parameters: vec![Parameter::trigger("go")],
            layers: vec![GraphLayer {
                states: vec![
                    State {
                        looping: false,
                        ..State::clip("Rest", "back")
                    },
                    State::clip("Go", "slide"),
                ],
                transitions: vec![Transition {
                    fade: 0.0,
                    ..Transition::new("Rest", "Go", vec![Condition::Triggered("go".into())])
                }],
                ..GraphLayer::default()
            }],
        };
        graph.save(&path, "slide.anim.ron").unwrap();
        let animation = Animation {
            graph: path.to_string_lossy().into_owned(),
            ..Animation::default()
        };

        let mut a = Animator::from_component(rig(), &animation, true);
        assert_eq!(a.state().graph.unwrap().layers[0].state, "Rest");
        a.advance(1.0);
        assert!((x(&a) + 1.0).abs() < 1e-4, "back, halfway: {}", x(&a));
        assert!(a.graph_mut().unwrap().trigger("go"));
        a.advance(0.5);
        assert_eq!(a.clip(), Some("slide"));
        assert!((x(&a) - 0.5).abs() < 1e-4, "{}", x(&a));
        // play() picks a state; stop() leaves the graph.
        assert!(a.play("Rest"));
        assert!(!a.play("back"), "a clip isn't a state");
        a.advance(0.0);
        assert_eq!(a.clip(), Some("back"));
        a.stop();
        assert!(a.graph().is_none());

        // In an editor (no autoplay) the entry state holds its first frame.
        let mut still = Animator::from_component(rig(), &animation, false);
        still.advance(1.0);
        assert!(x(&still).abs() < 1e-6);
        // A graph that doesn't load falls back to the clip.
        let missing = Animation {
            graph: dir.join("nope.anim.ron").to_string_lossy().into_owned(),
            ..Animation::new("slide")
        };
        let fallback = Animator::from_component(rig(), &missing, true);
        assert!(fallback.graph().is_none());
        assert_eq!(fallback.clip(), Some("slide"));
        let _ = std::fs::remove_dir_all(dir);
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
    fn crossfades_ease_from_the_old_clip_to_the_new() {
        let mut a = Animator::new(rig());
        a.play("slide");
        a.advance(1.0); // slide at +1
        let fade = PlayOptions {
            fade: 1.0,
            ..PlayOptions::default()
        };
        assert!(a.play_with("back", fade));
        assert!(a.fading());
        a.advance(0.5);
        // slide is at +1.5, back at -0.5; halfway through the eased blend.
        assert!((x(&a) - 0.5).abs() < 1e-4, "{}", x(&a));
        a.advance(0.6);
        assert!(!a.fading());
        assert!((x(&a) + 1.1).abs() < 1e-4, "only back now: {}", x(&a));

        // Fading out to the rest pose, then a fade interrupted mid-way.
        a.stop_with(1.0);
        a.advance(0.5);
        // back (still playing underneath) is at -1.6, rest at 0.
        assert!((x(&a) + 0.8).abs() < 1e-4, "{}", x(&a));
        a.play_with("slide", fade);
        a.advance(0.0);
        assert!((x(&a) + 0.8).abs() < 1e-4, "starts from what was shown");
    }

    #[test]
    fn the_outgoing_clip_keeps_its_own_looping_while_it_fades() {
        let mut a = Animator::new(rig());
        a.play("slide");
        a.advance(1.9);
        let once = PlayOptions {
            looping: false,
            fade: 1.0,
            ..PlayOptions::default()
        };
        a.play_with("back", once);
        a.advance(0.5);
        // slide wraps to 0.4 (it loops), back is at -0.5: halfway is -0.05.
        assert!((x(&a) + 0.05).abs() < 1e-4, "{}", x(&a));
        assert!(!a.looping);
    }

    #[test]
    fn playing_the_current_clip_again_keeps_it_going() {
        let mut a = Animator::new(rig());
        a.play_with("slide", PlayOptions::default());
        a.advance(1.0);
        let fast = PlayOptions {
            speed: Some(2.0),
            ..PlayOptions::default()
        };
        assert!(a.play_with("slide", fast));
        assert!((a.time - 1.0).abs() < 1e-6 && (a.speed - 2.0).abs() < 1e-6);
        assert!(a.play_with(
            "slide",
            PlayOptions {
                restart: true,
                ..PlayOptions::default()
            }
        ));
        assert!(a.time.abs() < 1e-6, "restart starts over");

        // A finished one-shot plays again.
        let once = PlayOptions {
            looping: false,
            ..PlayOptions::default()
        };
        a.play_with("back", once);
        a.advance(5.0);
        assert!(!a.playing);
        a.play_with("back", once);
        assert!(a.playing && a.time.abs() < 1e-6);

        // Backwards starts at the end; seek and pose_at clamp.
        a.speed = -1.0;
        a.play("slide");
        assert!((a.time - 2.0).abs() < 1e-6);
        a.seek(9.0);
        assert!((a.time - 2.0).abs() < 1e-6, "not looping: clamped");
        assert!(a.pose_at("back", 1.0) && !a.playing);
        assert!((x(&a) + 1.0).abs() < 1e-5);
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

    /// The test rig with events on `slide`: `start` at 0, `step` at 0.5
    /// and 1.5.
    fn eventful() -> Arc<Rig> {
        let mut rig = (*rig()).clone();
        let slide = &mut rig.clips[0];
        slide.add_event(1.5, "step");
        slide.add_event(0.5, "step");
        slide.add_event(0.0, "start");
        Arc::new(rig)
    }

    fn names(a: &Animator, after: u64) -> Vec<String> {
        a.recent_events()
            .iter()
            .filter(|e| e.seq > after)
            .map(|e| e.name.clone())
            .collect()
    }

    #[test]
    fn events_fire_as_the_playhead_passes_them() {
        let mut a = Animator::new(eventful());
        a.play("slide");
        a.advance(0.25);
        assert_eq!(
            names(&a, 0),
            ["start"],
            "an event at 0 fires when the clip starts"
        );
        a.advance(0.5);
        assert_eq!(names(&a, 1), ["step"]);
        // 0.75 → 2.75 wraps once: step at 1.5, start at 2.0 (the lap), step at 2.5.
        a.advance(2.0);
        assert_eq!(names(&a, 2), ["step", "start", "step"]);
        let last = a.recent_events().last().unwrap();
        assert_eq!((last.seq, last.clip.as_str()), (5, "slide"));

        // Backwards from 0.75 to 0.25 passes the step at 0.5 only.
        a.speed = -1.0;
        a.advance(0.5);
        assert_eq!(names(&a, 5), ["step"]);
        // Seeking jumps without firing.
        a.seek(1.9);
        a.speed = 1.0;
        a.advance(0.05);
        assert_eq!(names(&a, 6), Vec::<String>::new());
    }

    #[test]
    fn a_one_shot_clip_fires_end_once_and_old_events_roll_off() {
        let mut a = Animator::new(eventful());
        a.play_with(
            "slide",
            PlayOptions {
                looping: false,
                ..PlayOptions::default()
            },
        );
        a.advance(5.0);
        assert_eq!(names(&a, 0), ["start", "step", "step", END_EVENT]);
        a.advance(1.0);
        assert_eq!(a.recent_events().len(), 4, "a stopped clip fires nothing");
        // Only the latest are kept.
        a.looping = true;
        a.play("slide");
        for _ in 0..40 {
            a.advance(1.0);
        }
        assert_eq!(a.recent_events().len(), RECENT_EVENTS);
        assert_eq!(a.state().events.last(), a.recent_events().last());
    }

    #[test]
    fn root_motion_moves_the_entity_instead_of_the_pose() {
        let mut scene = crate::Scene::new();
        let spawn =
            |scene: &mut crate::Scene, root_motion: bool, extra: Option<CharacterMotion>| {
                let mut a = Animator::new(rig());
                a.root_motion = root_motion;
                a.play("slide");
                let turned = Transform::from_position(Vec3::new(5.0, 0.0, 0.0))
                    .with_rotation(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
                let e = scene.spawn((a, turned, GlobalTransform(turned.to_matrix())));
                if let Some(m) = extra {
                    scene.world_mut().insert_one(e, m).unwrap();
                }
                e
            };
        let walker = spawn(&mut scene, true, None);
        let character = spawn(&mut scene, true, Some(CharacterMotion::default()));
        let slider = spawn(&mut scene, false, None);
        for _ in 0..10 {
            animate(scene.world_mut(), 0.1);
        }
        let world = scene.world();
        let at = |e| world.get::<&Transform>(e).unwrap().position;
        // The clip slides +X at 1 m/s; turned 90° about Y that's -Z.
        assert!(
            at(walker).abs_diff_eq(Vec3::new(5.0, 0.0, -1.0), 1e-4),
            "{}",
            at(walker)
        );
        let w = world.get::<&Animator>(walker).unwrap();
        assert!(x(&w).abs() < 1e-5, "the pose stays over the origin");
        // A character's root motion waits for its controller.
        assert!(at(character).abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-6));
        let pending = world.get::<&CharacterMotion>(character).unwrap().pending;
        assert!(
            pending.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-4),
            "{pending}"
        );
        // Without root motion the model slides and the entity stays.
        assert!(at(slider).abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-6));
        assert!((x(&world.get::<&Animator>(slider).unwrap()) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn root_motion_blends_through_a_crossfade() {
        let mut a = Animator::new(rig());
        a.root_motion = true;
        a.play("slide"); // +1 m/s
        a.advance(0.5);
        a.play_with(
            "back", // -1 m/s
            PlayOptions {
                fade: 1.0,
                ..PlayOptions::default()
            },
        );
        a.advance(0.5); // halfway through the eased blend: they cancel
        assert!(a.travel().abs_diff_eq(Vec3::ZERO, 1e-4), "{}", a.travel());
        a.advance(1.0);
        assert!(a.travel().abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-4));
        a.root_motion = false;
        assert_eq!(a.travel(), Vec3::ZERO);
    }

    fn rigged_visual() -> MeshVisual {
        let mut model = Model::new("robot.glb".into(), Vec::new());
        model.rig = Some(rig());
        MeshVisual {
            model: Some(Arc::new(model)),
            ..MeshVisual::cube(1.0, [1.0; 4])
        }
    }

    #[test]
    fn models_with_spring_bones_swing_where_the_entity_is() {
        use automata_assets::springs::{SpringChain, Springs};
        let node = |name: &str, parent, z| RigNode {
            name: String::from(name),
            parent,
            rest: Trs {
                translation: Vec3::new(0.0, 0.0, z),
                ..Trs::IDENTITY
            },
        };
        let rig = Rig {
            nodes: vec![
                node("Body", None, 0.0),
                node("Tail", Some(0), 0.0),
                node("Tip", Some(1), -1.0),
            ],
            joints: (0..3).map(|i| (i, Mat4::IDENTITY)).collect(),
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: Springs {
                chains: vec![SpringChain {
                    stiffness: 0.0,
                    gravity: 1.0,
                    ..SpringChain::new("Tail")
                }],
                colliders: Vec::new(),
            },
        };
        let mut model = Model::new("tail.glb".into(), Vec::new());
        model.rig = Some(Arc::new(rig));
        let visual = MeshVisual {
            model: Some(Arc::new(model)),
            ..MeshVisual::cube(1.0, [1.0; 4])
        };
        let mut scene = crate::Scene::new();
        let at = Vec3::new(5.0, 2.0, 0.0);
        let e = scene.spawn((visual, GlobalTransform(Mat4::from_translation(at))));
        for _ in 0..120 {
            animate(scene.world_mut(), 1.0 / 60.0);
        }
        let animator = scene.world().get::<&Animator>(e).unwrap();
        assert!(animator.has_springs(), "no Animation needed for springs");
        let (_, tip) = animator.springs().unwrap().tips()[0];
        assert!(
            tip.distance(at + Vec3::new(0.0, -1.0, 0.0)) < 0.05,
            "hangs: {tip}"
        );
        // The pose (and so the palette) shows the tail hanging too.
        let g = animator.rig().globals(animator.pose());
        assert!(g[2].transform_point3(Vec3::ZERO).y < -0.95);
        drop(animator);
        // The scene's wind (toward +X) blows it out sideways.
        scene.spawn((automata_renderer::Environment {
            wind: 1.0,
            wind_direction: 0.0,
            ..automata_renderer::Environment::default()
        },));
        for _ in 0..120 {
            animate(scene.world_mut(), 1.0 / 60.0);
        }
        let animator = scene.world().get::<&Animator>(e).unwrap();
        let (_, tip) = animator.springs().unwrap().tips()[0];
        assert!(tip.x > at.x + 0.3, "streams downwind: {tip}");
    }

    #[test]
    fn animation_components_attach_play_and_take_commands() {
        let mut scene = crate::Scene::new();
        let playing = scene.spawn((rigged_visual(), Animation::new("slide")));
        let held = scene.spawn((
            rigged_visual(),
            Animation {
                play_on_start: false,
                ..Animation::new("back")
            },
        ));
        let bare = scene.spawn((rigged_visual(),));
        let plain = scene.spawn((MeshVisual::cube(1.0, [1.0; 4]),));

        // Before any animator exists, scripts still see the clips.
        let state = animation_state(scene.world(), bare).unwrap();
        assert_eq!(state.clips, ["slide", "back"]);
        assert_eq!(state.lengths, [2.0, 2.0]);
        assert_eq!(state.clip, None);
        assert!(animation_state(scene.world(), plain).is_none());

        animate(scene.world_mut(), 0.5);
        let world = scene.world_mut();
        let s = animation_state(world, playing).unwrap();
        assert_eq!(s.clip.as_deref(), Some("slide"));
        assert!(s.playing && (s.time - 0.5).abs() < 1e-6);
        let s = animation_state(world, held).unwrap();
        assert_eq!(s.clip.as_deref(), Some("back"));
        assert!(!s.playing && s.time.abs() < 1e-6);
        assert!(
            world.get::<&Animator>(bare).is_err(),
            "no Animation, no animator"
        );

        let play = |clip: &str| AnimationCommand::Play {
            clip: clip.into(),
            options: PlayOptions::default(),
        };
        assert!(command_animation(world, bare, &play("back")));
        assert!(!command_animation(world, bare, &play("nope")));
        assert!(!command_animation(world, plain, &play("back")));
        assert!(command_animation(
            world,
            playing,
            &AnimationCommand::Seek(1.5)
        ));
        assert!(command_animation(
            world,
            playing,
            &AnimationCommand::Speed(2.0)
        ));
        assert!(command_animation(world, held, &AnimationCommand::Stop(0.0)));
        let s = animation_state(world, playing).unwrap();
        assert!((s.time - 1.5).abs() < 1e-6 && (s.speed - 2.0).abs() < 1e-6);
        assert_eq!(animation_state(world, held).unwrap().clip, None);
        assert!(animation_state(world, bare).unwrap().playing);
    }

    #[test]
    fn default_clip_prefers_idle() {
        let mut r = (*rig()).clone();
        assert_eq!(Animation::default_clip(&r), Some("slide"));
        r.clips.push(slide("Idle_Loop", 1.0));
        assert_eq!(Animation::default_clip(&r), Some("Idle_Loop"));
        r.clips.clear();
        assert_eq!(Animation::default_clip(&r), None);
    }
}
