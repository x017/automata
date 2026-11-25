//! Running animation graphs ([`automata_assets::anim_graph`]).
//!
//! A [`GraphPlayer`] holds a graph's parameter values and, per layer, the
//! state it's in, how far through its motion it is, and the crossfade
//! from the previous state. An [`Animator`](crate::Animator) given a graph
//! hands each step to it: transitions are checked first (Any State ones
//! before the current state's own, first match wins), then every layer's
//! motion advances and the layers are stacked into one pose.
//!
//! Blend spaces keep their clips in step: they share one normalized time,
//! advancing at the weighted average of the clips' lengths, so a walk and
//! a run put their feet down together. Events come from the clip that
//! weighs most; root motion from the first layer, blended by weight.

use std::sync::Arc;

use automata_assets::anim_graph::{AnimGraph, Condition, Motion, ParamKind, ANY_STATE};
use automata_assets::{ClipMotion, Pose, Rig};
use glam::{Vec2, Vec3};

use crate::animation::{crossed, layer_travel, sample_clip, END_EVENT};

/// What a graph step reads: the rig, its clips' ground travel, and
/// whether root motion is on.
pub(crate) struct Sampler<'a> {
    pub rig: &'a Rig,
    pub motions: &'a [Option<ClipMotion>],
    pub root_motion: bool,
}

/// What a graph step produced.
pub(crate) struct GraphOutput {
    pub pose: Pose,
    /// Ground travel (model space), from the first layer.
    pub travel: Vec3,
    /// Events passed: (clip index, name).
    pub events: Vec<(usize, String)>,
}

#[derive(Debug, Clone, Copy)]
enum Blend {
    /// One clip (or none).
    Single,
    /// Along a parameter; the points are sorted.
    D1(usize),
    /// Over two parameters.
    D2(usize, usize),
}

/// A state's motion with clips and parameters looked up.
#[derive(Debug, Clone)]
struct RMotion {
    /// (clip index, where it sits). Empty: the state plays nothing.
    points: Vec<(usize, Vec2)>,
    blend: Blend,
}

#[derive(Debug, Clone)]
struct RState {
    motion: RMotion,
    speed: f32,
    speed_param: Option<usize>,
    looping: bool,
}

#[derive(Debug, Clone)]
struct RTransition {
    /// `None`: from any state.
    from: Option<usize>,
    to: usize,
    when: Vec<(usize, Condition)>,
    fade: f32,
    exit: Option<f32>,
}

#[derive(Debug, Clone)]
struct RLayer {
    name: String,
    states: Vec<RState>,
    transitions: Vec<RTransition>,
    entry: usize,
    /// Which rig nodes the layer drives (`None`: all).
    mask: Option<Vec<bool>>,
}

/// One state's playback.
#[derive(Debug, Clone, Copy)]
struct Playhead {
    state: usize,
    /// How far through the motion, 0 to 1.
    t: f32,
    /// How far through in all (laps included) since the state began.
    total: f32,
    playing: bool,
    /// Just begun: an event at its first moment fires.
    fresh: bool,
}

impl Playhead {
    fn start(state: usize) -> Self {
        Self {
            state,
            t: 0.0,
            total: 0.0,
            playing: true,
            fresh: true,
        }
    }
}

/// A layer's own result: its pose and how present it is (0 for an Empty
/// state, which lets the layers below show through).
#[derive(Debug, Clone)]
struct LayerPose {
    pose: Pose,
    presence: f32,
}

#[derive(Debug, Clone)]
enum FadeFrom {
    /// The previous state, still playing underneath.
    State(Playhead),
    /// What the layer showed when a fade was interrupted.
    Frozen(LayerPose),
}

#[derive(Debug, Clone)]
struct LayerFade {
    from: FadeFrom,
    duration: f32,
    elapsed: f32,
}

#[derive(Debug, Clone)]
struct LayerRun {
    head: Playhead,
    fade: Option<LayerFade>,
    weight: f32,
    last: Option<LayerPose>,
}

/// A layer's playback, as scripts and the editor see it.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphLayerState {
    /// The layer's name.
    pub name: String,
    /// The state it's in (or fading to).
    pub state: String,
    /// How far through the state's motion, 0 to 1.
    pub time: f32,
    /// The state it's fading from, while a transition blends.
    pub from: Option<String>,
    /// How far the fade has got, 0 to 1 (1 with no fade).
    pub fade: f32,
    /// The layer's weight.
    pub weight: f32,
}

/// A graph's playback, as scripts and the editor see it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphState {
    /// Every layer's, bottom first.
    pub layers: Vec<GraphLayerState>,
    /// Every parameter's value (bools and triggers as 0 or 1).
    pub params: Vec<(String, f32)>,
}

/// Plays an [`AnimGraph`] on a rig (see the module docs).
#[derive(Debug, Clone)]
pub struct GraphPlayer {
    graph: Arc<AnimGraph>,
    kinds: Vec<ParamKind>,
    params: Vec<f32>,
    layers: Vec<RLayer>,
    run: Vec<LayerRun>,
    problems: Vec<String>,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl GraphPlayer {
    /// A player for `graph` on `rig`, every layer in its entry state and
    /// every parameter at its starting value. What doesn't fit the rig
    /// (unknown clips or bones) is left out and listed in
    /// [`Self::problems`].
    #[must_use]
    pub fn new(graph: Arc<AnimGraph>, rig: &Rig) -> Self {
        let clips = rig.clip_names();
        let bones: Vec<&str> = rig.nodes.iter().map(|n| n.name.as_str()).collect();
        let problems = graph.problems(Some(&clips), Some(&bones));
        let find_param = |name: &str| graph.parameters.iter().position(|p| p.name == name);
        let find_clip = |name: &str| rig.clips.iter().position(|c| c.name == name);
        let mut layers = Vec::new();
        for layer in &graph.layers {
            let states = layer
                .states
                .iter()
                .map(|s| RState {
                    motion: resolve_motion(&s.motion, &find_clip, &find_param),
                    speed: s.speed,
                    speed_param: (!s.speed_param.is_empty())
                        .then(|| find_param(&s.speed_param))
                        .flatten(),
                    looping: s.looping,
                })
                .collect();
            let transitions = layer
                .transitions
                .iter()
                .filter_map(|t| {
                    let from = if t.from == ANY_STATE {
                        None
                    } else {
                        Some(layer.state_index(&t.from)?)
                    };
                    let to = layer.state_index(&t.to)?;
                    let when = t
                        .when
                        .iter()
                        .map(|c| Some((find_param(c.param())?, c.clone())))
                        .collect::<Option<Vec<_>>>()?;
                    if from.is_none() && when.is_empty() {
                        return None; // would fire forever
                    }
                    Some(RTransition {
                        from,
                        to,
                        when,
                        fade: t.fade.max(0.0),
                        exit: t.effective_exit_time(),
                    })
                })
                .collect();
            let mask = (!layer.mask.is_empty()).then(|| {
                let mut on = vec![false; rig.nodes.len()];
                for (i, node) in rig.nodes.iter().enumerate() {
                    on[i] = layer.mask.contains(&node.name) || node.parent.is_some_and(|p| on[p]);
                }
                on
            });
            layers.push(RLayer {
                name: layer.name.clone(),
                states,
                transitions,
                entry: layer.entry_index().unwrap_or(0),
                mask,
            });
        }
        let run = layers
            .iter()
            .zip(&graph.layers)
            .map(|(l, g)| LayerRun {
                head: Playhead::start(l.entry),
                fade: None,
                weight: g.weight.clamp(0.0, 1.0),
                last: None,
            })
            .collect();
        let kinds: Vec<ParamKind> = graph.parameters.iter().map(|p| p.kind).collect();
        let params = kinds.iter().map(|k| k.initial()).collect();
        Self {
            graph,
            kinds,
            params,
            layers,
            run,
            problems,
        }
    }

    /// The graph being played.
    #[must_use]
    pub fn graph(&self) -> &Arc<AnimGraph> {
        &self.graph
    }

    /// What didn't fit the rig when the player was made.
    #[must_use]
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    fn param_index(&self, name: &str) -> Option<usize> {
        self.graph.parameters.iter().position(|p| p.name == name)
    }

    /// Sets a parameter: a float as given, an int rounded, a bool or
    /// trigger on when `value` is at least 0.5. Returns `false` for an
    /// unknown parameter or a value that isn't a number.
    pub fn set(&mut self, name: &str, value: f32) -> bool {
        let Some(i) = self.param_index(name) else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        self.params[i] = match self.kinds[i] {
            ParamKind::Float(_) => value,
            ParamKind::Int(_) => value.round(),
            ParamKind::Bool(_) | ParamKind::Trigger => f32::from(u8::from(value >= 0.5)),
        };
        true
    }

    /// A parameter's value (bools and triggers as 0 or 1).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<f32> {
        self.param_index(name).map(|i| self.params[i])
    }

    /// Sets a trigger (or turns a bool on). It stays set until a
    /// transition that tests it fires. Returns `false` for an unknown
    /// parameter or a number.
    pub fn trigger(&mut self, name: &str) -> bool {
        match self.param_index(name) {
            Some(i) if matches!(self.kinds[i], ParamKind::Trigger | ParamKind::Bool(_)) => {
                self.params[i] = 1.0;
                true
            }
            _ => false,
        }
    }

    /// Clears a trigger that no transition used.
    pub fn reset_trigger(&mut self, name: &str) -> bool {
        match self.param_index(name) {
            Some(i) if self.kinds[i] == ParamKind::Trigger => {
                self.params[i] = 0.0;
                true
            }
            _ => false,
        }
    }

    fn layer_index(&self, layer: &str) -> Option<usize> {
        if layer.is_empty() {
            return (!self.layers.is_empty()).then_some(0);
        }
        self.layers.iter().position(|l| l.name == layer)
    }

    /// Sets a layer's weight (0 hides it, 1 shows it fully). Returns
    /// `false` for an unknown layer.
    pub fn set_layer_weight(&mut self, layer: &str, weight: f32) -> bool {
        match self.layer_index(layer) {
            Some(i) if weight.is_finite() => {
                self.run[i].weight = weight.clamp(0.0, 1.0);
                true
            }
            _ => false,
        }
    }

    /// Goes straight to `state` in `layer` (empty: the first layer),
    /// crossfading over `fade` seconds, whatever the transitions say.
    /// Returns `false` for an unknown layer or state.
    pub fn play_state(&mut self, layer: &str, state: &str, fade: f32) -> bool {
        let Some(l) = self.layer_index(layer) else {
            return false;
        };
        let Some(s) = self.graph.layers[l].state_index(state) else {
            return false;
        };
        self.begin(l, s, fade);
        true
    }

    /// Starts state `to` on layer `l`, fading from what's shown.
    fn begin(&mut self, l: usize, to: usize, fade: f32) {
        let run = &mut self.run[l];
        run.fade = (fade.is_finite() && fade > 0.0).then(|| LayerFade {
            from: match (&run.fade, &run.last) {
                (Some(_), Some(last)) => FadeFrom::Frozen(last.clone()),
                _ => FadeFrom::State(run.head),
            },
            duration: fade,
            elapsed: 0.0,
        });
        run.head = Playhead::start(to);
    }

    /// The transition layer `l` takes now, if any.
    fn pick(&self, l: usize) -> Option<usize> {
        let layer = &self.layers[l];
        let head = self.run[l].head;
        let ready = |t: &RTransition| {
            t.when.iter().all(|(p, c)| c.holds(self.params[*p]))
                && t.exit.is_none_or(|e| {
                    // A state that doesn't loop and has stopped is done.
                    head.total >= e - 1e-4 || !head.playing && !layer.states[head.state].looping
                })
        };
        let any = layer
            .transitions
            .iter()
            .position(|t| t.from.is_none() && t.to != head.state && ready(t));
        any.or_else(|| {
            layer
                .transitions
                .iter()
                .position(|t| t.from == Some(head.state) && ready(t))
        })
    }

    /// Checks transitions, advances every layer by `dt` seconds (times
    /// `speed`), and stacks the layers into a pose.
    pub(crate) fn advance(&mut self, s: &Sampler<'_>, dt: f32, speed: f32) -> GraphOutput {
        for l in 0..self.layers.len() {
            if let Some(t) = self.pick(l) {
                let (to, fade) = {
                    let t = &self.layers[l].transitions[t];
                    for (p, c) in &t.when {
                        if matches!(c, Condition::Triggered(_)) {
                            self.params[*p] = 0.0;
                        }
                    }
                    (t.to, t.fade)
                };
                self.begin(l, to, fade);
            }
        }
        let mut pose = s.rig.rest_pose();
        let mut travel = Vec3::ZERO;
        let mut events = Vec::new();
        for l in 0..self.layers.len() {
            let (own, layer_travel) = self.advance_layer(l, s, dt * speed, &mut events);
            if l == 0 {
                travel = layer_travel;
            }
            let weight = if l == 0 {
                1.0
            } else {
                self.run[l].weight * own.presence
            };
            let mask = self.layers[l].mask.as_deref();
            for (n, (out, mine)) in pose.0.iter_mut().zip(&own.pose.0).enumerate() {
                if mask.is_none_or(|m| m.get(n).copied().unwrap_or(false)) {
                    *out = if l == 0 {
                        *mine
                    } else {
                        out.lerp(mine, weight)
                    };
                }
            }
            self.run[l].last = Some(own);
        }
        GraphOutput {
            pose,
            travel,
            events,
        }
    }

    /// Advances layer `l` (its state and any fade) and returns its pose
    /// and ground travel.
    fn advance_layer(
        &mut self,
        l: usize,
        s: &Sampler<'_>,
        dt: f32,
        events: &mut Vec<(usize, String)>,
    ) -> (LayerPose, Vec3) {
        let mut head = self.run[l].head;
        let travel = self.step(l, &mut head, s, dt, Some(events));
        self.run[l].head = head;
        let mut own = self.sample(l, &head, s);
        let Some(mut fade) = self.run[l].fade.take() else {
            return (own, travel);
        };
        fade.elapsed += dt.abs();
        let w = smooth(fade.elapsed / fade.duration);
        let (from, from_travel) = match &mut fade.from {
            FadeFrom::Frozen(pose) => (pose.clone(), Vec3::ZERO),
            FadeFrom::State(old) => {
                let t = self.step(l, old, s, dt, None);
                (self.sample(l, old, s), t)
            }
        };
        own = blend_layer(&from, &own, w);
        let travel = from_travel.lerp(travel, w);
        if fade.elapsed < fade.duration {
            self.run[l].fade = Some(fade);
        }
        (own, travel)
    }

    /// The motion's clips and weights for the current parameters.
    fn weights(&self, motion: &RMotion) -> Vec<(usize, f32)> {
        let points = &motion.points;
        match (motion.blend, points.len()) {
            (_, 0) => Vec::new(),
            (Blend::Single, _) | (_, 1) => vec![(points[0].0, 1.0)],
            (Blend::D1(param), _) => {
                let value = self.params[param];
                if value <= points[0].1.x {
                    return vec![(points[0].0, 1.0)];
                }
                for pair in points.windows(2) {
                    let (lo, hi) = (pair[0].1.x, pair[1].1.x);
                    if value <= hi {
                        let along = if hi > lo {
                            (value - lo) / (hi - lo)
                        } else {
                            1.0
                        };
                        return vec![(pair[0].0, 1.0 - along), (pair[1].0, along)];
                    }
                }
                vec![(points[points.len() - 1].0, 1.0)]
            }
            (Blend::D2(x, y), _) => {
                gradient_band(points, Vec2::new(self.params[x], self.params[y]))
            }
        }
    }

    /// Moves `head` on by `dt` seconds of the state's motion, noting the
    /// events its strongest clip passes into `events` (when given).
    /// Returns the ground travel.
    fn step(
        &self,
        l: usize,
        head: &mut Playhead,
        s: &Sampler<'_>,
        dt: f32,
        events: Option<&mut Vec<(usize, String)>>,
    ) -> Vec3 {
        let state = &self.layers[l].states[head.state];
        let weights = self.weights(&state.motion);
        let fresh = std::mem::take(&mut head.fresh);
        if weights.is_empty() || !head.playing {
            return Vec3::ZERO;
        }
        let length: f32 = weights
            .iter()
            .map(|(c, w)| w * s.rig.clips[*c].duration)
            .sum();
        let rate = state.speed * state.speed_param.map_or(1.0, |p| self.params[p]);
        if length <= 1e-6 {
            return Vec3::ZERO;
        }
        let dn = dt * rate / length;
        if dn == 0.0 {
            return Vec3::ZERO;
        }
        let mut travel = Vec3::ZERO;
        for (c, w) in &weights {
            let d = s.rig.clips[*c].duration;
            travel += *w * layer_travel(s.rig, s.motions, *c, head.t * d, dn * d, state.looping);
        }
        let strongest = weights
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(c, _)| *c);
        let mut passed = Vec::new();
        let clip = &s.rig.clips[strongest];
        crossed(
            clip,
            head.t * clip.duration,
            dn * clip.duration,
            state.looping,
            fresh,
            &mut passed,
        );
        passed.sort_by(|a, b| a.0.total_cmp(&b.0));
        head.total += dn.abs();
        head.t += dn;
        if state.looping {
            head.t = head.t.rem_euclid(1.0);
        } else if head.t >= 1.0 || head.t <= 0.0 && dn < 0.0 {
            head.t = head.t.clamp(0.0, 1.0);
            head.playing = false;
            passed.push((f32::MAX, END_EVENT));
        }
        if let Some(events) = events {
            events.extend(passed.iter().map(|(_, n)| (strongest, (*n).to_string())));
        }
        travel
    }

    /// The pose of layer `l`'s state under `head`.
    fn sample(&self, l: usize, head: &Playhead, s: &Sampler<'_>) -> LayerPose {
        let state = &self.layers[l].states[head.state];
        let weights = self.weights(&state.motion);
        let mut pose = s.rig.rest_pose();
        if weights.is_empty() {
            return LayerPose {
                pose,
                presence: 0.0,
            };
        }
        let mut total = 0.0;
        for (clip, weight) in weights {
            if weight <= 0.0 {
                continue;
            }
            let time = head.t * s.rig.clips[clip].duration;
            let mut one = s.rig.rest_pose();
            sample_clip(s.rig, s.motions, clip, time, s.root_motion, &mut one);
            if total > 0.0 {
                total += weight;
                pose.blend(&one, weight / total);
            } else {
                total = weight;
                pose = one;
            }
        }
        LayerPose {
            pose,
            presence: 1.0,
        }
    }

    /// The first layer's strongest clip and its time in seconds: what the
    /// animator reports as playing.
    pub(crate) fn current_clip(&self, rig: &Rig) -> Option<(usize, f32)> {
        let head = self.run.first()?.head;
        let state = &self.layers[0].states[head.state];
        let (clip, _) = self
            .weights(&state.motion)
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        Some((clip, head.t * rig.clips[clip].duration))
    }

    /// The playback, as scripts and the editor see it.
    #[must_use]
    pub fn state(&self) -> GraphState {
        let layers = self
            .run
            .iter()
            .zip(&self.graph.layers)
            .zip(&self.layers)
            .map(|((run, g), r)| GraphLayerState {
                name: r.name.clone(),
                state: g.states[run.head.state].name.clone(),
                time: run.head.t,
                from: run.fade.as_ref().map(|f| match &f.from {
                    FadeFrom::State(h) => g.states[h.state].name.clone(),
                    FadeFrom::Frozen(_) => String::new(),
                }),
                fade: run
                    .fade
                    .as_ref()
                    .map_or(1.0, |f| (f.elapsed / f.duration).clamp(0.0, 1.0)),
                weight: run.weight,
            })
            .collect();
        let params = self
            .graph
            .parameters
            .iter()
            .zip(&self.params)
            .map(|(p, v)| (p.name.clone(), *v))
            .collect();
        GraphState { layers, params }
    }
}

/// Looks up a motion's clips and parameters (unknown ones are dropped).
fn resolve_motion(
    motion: &Motion,
    find_clip: &dyn Fn(&str) -> Option<usize>,
    find_param: &dyn Fn(&str) -> Option<usize>,
) -> RMotion {
    let empty = RMotion {
        points: Vec::new(),
        blend: Blend::Single,
    };
    match motion {
        Motion::Empty => empty,
        Motion::Clip(c) => RMotion {
            points: find_clip(c).map(|i| (i, Vec2::ZERO)).into_iter().collect(),
            blend: Blend::Single,
        },
        Motion::Blend1D { param, points } => {
            let Some(param) = find_param(param) else {
                return empty;
            };
            let mut points: Vec<(usize, Vec2)> = points
                .iter()
                .filter_map(|p| Some((find_clip(&p.clip)?, Vec2::new(p.at, 0.0))))
                .collect();
            points.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
            RMotion {
                points,
                blend: Blend::D1(param),
            }
        }
        Motion::Blend2D { x, y, points } => {
            let (Some(x), Some(y)) = (find_param(x), find_param(y)) else {
                return empty;
            };
            let points = points
                .iter()
                .filter_map(|p| Some((find_clip(&p.clip)?, Vec2::from(p.at))))
                .collect();
            RMotion {
                points,
                blend: Blend::D2(x, y),
            }
        }
    }
}

/// Gradient band interpolation (freeform 2D blending): each point weighs
/// in by how far `at` is from crossing over to every other point.
fn gradient_band(points: &[(usize, Vec2)], at: Vec2) -> Vec<(usize, f32)> {
    let mut weights: Vec<(usize, f32)> = points
        .iter()
        .enumerate()
        .map(|(i, (clip, pi))| {
            let mut w = 1.0_f32;
            for (j, (_, pj)) in points.iter().enumerate() {
                if i == j {
                    continue;
                }
                let edge = *pj - *pi;
                let len2 = edge.length_squared();
                if len2 > 1e-12 {
                    w = w.min(1.0 - (at - *pi).dot(edge) / len2);
                }
            }
            (*clip, w.max(0.0))
        })
        .collect();
    let total: f32 = weights.iter().map(|(_, w)| w).sum();
    if total <= 1e-6 {
        // Shouldn't happen, but fall back to the nearest point.
        let nearest = points
            .iter()
            .min_by(|a, b| {
                a.1.distance_squared(at)
                    .total_cmp(&b.1.distance_squared(at))
            })
            .map_or(0, |p| p.0);
        return vec![(nearest, 1.0)];
    }
    for (_, w) in &mut weights {
        *w /= total;
    }
    weights
}

/// Blends a layer's pose from `from` towards `to` by `w`. An empty side
/// (presence 0) takes the other's pose, so only the presence fades.
fn blend_layer(from: &LayerPose, to: &LayerPose, w: f32) -> LayerPose {
    let pose = match (from.presence > 0.0, to.presence > 0.0) {
        (true, true) => {
            let mut p = from.pose.clone();
            p.blend(&to.pose, w);
            p
        }
        (true, false) => from.pose.clone(),
        _ => to.pose.clone(),
    };
    LayerPose {
        pose,
        presence: from.presence + (to.presence - from.presence) * w,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_assets::anim::{Channel, Interpolation, Property, RigNode, Trs};
    use automata_assets::anim_graph::{
        BlendPoint1D, BlendPoint2D, GraphLayer, Parameter, State, Transition,
    };
    use automata_assets::Clip;
    use glam::Mat4;

    /// A clip holding node `node` at x = `x` (y = `y`) for `duration`.
    fn hold(name: &str, node: usize, x: f32, y: f32, duration: f32) -> Clip {
        Clip {
            name: name.into(),
            duration,
            channels: vec![Channel {
                node,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, duration],
                values: vec![x, y, 0.0, x, y, 0.0],
            }],
            events: Vec::new(),
        }
    }

    /// Two nodes: "hips" (0) and its child "arm" (1).
    fn rig() -> Rig {
        let node = |name: &str, parent| RigNode {
            name: name.into(),
            parent,
            rest: Trs::IDENTITY,
        };
        Rig {
            nodes: vec![node("hips", None), node("arm", Some(0))],
            joints: vec![(0, Mat4::IDENTITY), (1, Mat4::IDENTITY)],
            import: Mat4::IDENTITY,
            clips: vec![
                hold("Idle", 0, 0.0, 0.0, 1.0),
                hold("Walk", 0, 2.0, 0.0, 1.0),
                hold("Run", 0, 5.0, 0.0, 0.5),
                hold("Wave", 1, 0.0, 3.0, 1.0),
                hold("Jump", 0, 0.0, 9.0, 0.5),
                hold("Left", 0, -1.0, 0.0, 1.0),
                hold("Right", 0, 1.0, 0.0, 1.0),
            ],
            springs: automata_assets::springs::Springs::default(),
        }
    }

    fn walk_graph() -> AnimGraph {
        AnimGraph {
            parameters: vec![
                Parameter::float("speed", 0.0),
                Parameter::trigger("jump"),
                Parameter::trigger("wave"),
                Parameter::float("x", 0.0),
                Parameter::float("y", 0.0),
            ],
            layers: vec![base_layer(), arms_layer()],
        }
    }

    /// Waving on the arm, over the base.
    fn arms_layer() -> GraphLayer {
        GraphLayer {
            name: "Arms".into(),
            mask: vec!["arm".into()],
            states: vec![
                State {
                    name: "Off".into(),
                    ..State::default()
                },
                State {
                    looping: false,
                    ..State::clip("Wave", "Wave")
                },
            ],
            transitions: vec![
                Transition {
                    fade: 0.5,
                    ..Transition::new("Off", "Wave", vec![Condition::Triggered("wave".into())])
                },
                Transition {
                    fade: 0.0,
                    ..Transition::new("Wave", "Off", vec![])
                },
            ],
            ..GraphLayer::default()
        }
    }

    /// Move (1D), Jump and Strafe (2D).
    fn base_layer() -> GraphLayer {
        GraphLayer {
            name: "Base".into(),
            states: vec![
                State {
                    name: "Move".into(),
                    motion: Motion::Blend1D {
                        param: "speed".into(),
                        points: vec![
                            BlendPoint1D {
                                clip: "Walk".into(),
                                at: 2.0,
                            },
                            BlendPoint1D {
                                clip: "Idle".into(),
                                at: 0.0,
                            },
                            BlendPoint1D {
                                clip: "Run".into(),
                                at: 5.0,
                            },
                        ],
                    },
                    ..State::default()
                },
                State {
                    looping: false,
                    ..State::clip("Jump", "Jump")
                },
                State {
                    name: "Strafe".into(),
                    motion: Motion::Blend2D {
                        x: "x".into(),
                        y: "y".into(),
                        points: vec![
                            BlendPoint2D {
                                clip: "Idle".into(),
                                at: [0.0, 0.0],
                            },
                            BlendPoint2D {
                                clip: "Left".into(),
                                at: [-1.0, 0.0],
                            },
                            BlendPoint2D {
                                clip: "Right".into(),
                                at: [1.0, 0.0],
                            },
                            BlendPoint2D {
                                clip: "Walk".into(),
                                at: [0.0, 1.0],
                            },
                        ],
                    },
                    ..State::default()
                },
            ],
            transitions: vec![
                Transition {
                    fade: 0.0,
                    ..Transition::new(ANY_STATE, "Jump", vec![Condition::Triggered("jump".into())])
                },
                // No conditions: when the jump ends.
                Transition {
                    fade: 0.0,
                    ..Transition::new("Jump", "Move", vec![])
                },
            ],
            ..GraphLayer::default()
        }
    }

    fn step(p: &mut GraphPlayer, rig: &Rig, dt: f32) -> GraphOutput {
        let motions: Vec<Option<ClipMotion>> = vec![None; rig.clips.len()];
        let s = Sampler {
            rig,
            motions: &motions,
            root_motion: false,
        };
        p.advance(&s, dt, 1.0)
    }

    fn at(out: &GraphOutput, node: usize) -> Vec3 {
        out.pose.0[node].translation
    }

    #[test]
    fn blend_1d_follows_the_parameter() {
        let rig = rig();
        let mut p = GraphPlayer::new(Arc::new(walk_graph()), &rig);
        assert!(p.problems().is_empty(), "{:?}", p.problems());
        assert!(
            (at(&step(&mut p, &rig, 0.1), 0).x).abs() < 1e-5,
            "idle at 0"
        );
        p.set("speed", 1.0);
        assert!((at(&step(&mut p, &rig, 0.1), 0).x - 1.0).abs() < 1e-4);
        p.set("speed", 3.5); // halfway from walk (2) to run (5)
        assert!((at(&step(&mut p, &rig, 0.1), 0).x - 3.5).abs() < 1e-4);
        p.set("speed", 9.0);
        assert!(
            (at(&step(&mut p, &rig, 0.1), 0).x - 5.0).abs() < 1e-4,
            "clamped to run"
        );
        assert!(!p.set("nope", 1.0));
        assert_eq!(p.state().layers[0].state, "Move");
    }

    #[test]
    fn blend_spaces_keep_clips_in_step() {
        let rig = rig();
        let mut p = GraphPlayer::new(Arc::new(walk_graph()), &rig);
        // Halfway between Walk (1 s) and Run (0.5 s): a 0.75 s cycle.
        p.set("speed", 3.5);
        step(&mut p, &rig, 0.375);
        assert!(
            (p.state().layers[0].time - 0.5).abs() < 1e-4,
            "{:?}",
            p.state()
        );
    }

    #[test]
    fn blend_2d_weights_by_closeness() {
        let rig = rig();
        let mut p = GraphPlayer::new(Arc::new(walk_graph()), &rig);
        assert!(p.play_state("", "Strafe", 0.0));
        p.set("x", -1.0);
        assert!(
            (at(&step(&mut p, &rig, 0.1), 0).x + 1.0).abs() < 1e-4,
            "on Left"
        );
        p.set("x", 0.5);
        let x = at(&step(&mut p, &rig, 0.1), 0).x;
        assert!((x - 0.5).abs() < 1e-3, "between Idle and Right: {x}");
        p.set("x", 0.0);
        p.set("y", 1.0);
        assert!(
            (at(&step(&mut p, &rig, 0.1), 0).x - 2.0).abs() < 1e-4,
            "on Walk"
        );
        let w = gradient_band(
            &[(0, Vec2::ZERO), (1, Vec2::X), (2, Vec2::Y)],
            Vec2::new(0.3, 0.3),
        );
        let total: f32 = w.iter().map(|(_, w)| w).sum();
        assert!((total - 1.0).abs() < 1e-5);
        assert!(w.iter().all(|(_, w)| *w > 0.0), "{w:?}");
    }

    #[test]
    fn triggers_fire_once_and_states_end() {
        let rig = rig();
        let mut p = GraphPlayer::new(Arc::new(walk_graph()), &rig);
        assert!(p.trigger("jump"));
        let out = step(&mut p, &rig, 0.1);
        assert!((at(&out, 0).y - 9.0).abs() < 1e-4, "jumping");
        assert_eq!(p.get("jump"), Some(0.0), "the trigger was used up");
        assert_eq!(p.state().layers[0].state, "Jump");
        // Jump (0.5 s) ends, fires "end", and goes back to Move.
        let out = step(&mut p, &rig, 0.5);
        assert!(out.events.iter().any(|(c, n)| *c == 4 && n == END_EVENT));
        step(&mut p, &rig, 0.01);
        assert_eq!(p.state().layers[0].state, "Move");
        assert!(!p.trigger("speed"), "floats aren't triggers");
    }

    #[test]
    fn masked_layers_fade_in_over_the_base() {
        let rig = rig();
        let mut p = GraphPlayer::new(Arc::new(walk_graph()), &rig);
        p.set("speed", 2.0);
        step(&mut p, &rig, 0.1);
        p.trigger("wave");
        let out = step(&mut p, &rig, 0.25); // halfway through a 0.5 s fade in
        assert!((at(&out, 0).x - 2.0).abs() < 1e-4, "the hips keep walking");
        assert!((at(&out, 1).y - 1.5).abs() < 1e-3, "{}", at(&out, 1).y);
        let out = step(&mut p, &rig, 0.3);
        assert!((at(&out, 1).y - 3.0).abs() < 1e-4, "fully waving");
        assert_eq!(p.state().layers[1].state, "Wave");
        assert!(p.set_layer_weight("Arms", 0.5));
        let out = step(&mut p, &rig, 0.1);
        assert!((at(&out, 1).y - 1.5).abs() < 1e-3);
        assert!(!p.set_layer_weight("Legs", 1.0));
    }

    #[test]
    fn interrupted_fades_start_from_what_was_shown() {
        let rig = rig();
        let mut g = walk_graph();
        g.layers[0].transitions[0].fade = 1.0;
        let mut p = GraphPlayer::new(Arc::new(g), &rig);
        p.trigger("jump");
        step(&mut p, &rig, 0.5); // halfway into Jump (y 9): y 4.5
        assert!(p.play_state("Base", "Strafe", 1.0));
        let out = step(&mut p, &rig, 0.0);
        assert!((at(&out, 0).y - 4.5).abs() < 1e-3, "{}", at(&out, 0).y);
        assert_eq!(p.state().layers[0].from.as_deref(), Some(""));
    }
}
