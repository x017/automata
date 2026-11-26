//! Agents: entities that find their own way to a point or another entity.

use std::collections::HashMap;

use automata_physics::{CharacterController, CharacterVelocity};
use automata_scene::{Entity, GlobalTransform, Name, Transform};
use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::{NavMesh, PathPoint};

/// How fast characters fall between jumps (m/s², as Oberon's player).
const GRAVITY: f32 = 20.0;
/// How often a path to a moving target is found again (seconds).
const REPATH: f32 = 0.3;
/// How quickly agents turn to face where they're going (per second).
const TURN_RATE: f32 = 10.0;
/// How quickly floating agents settle to their height (per second).
const SETTLE_RATE: f32 = 8.0;
/// How far beyond touching agents start to give way (metres).
const PERSONAL_SPACE: f32 = 0.6;

/// Where an agent is heading.
#[derive(Debug, Clone, PartialEq)]
pub enum NavTarget {
    /// A fixed point.
    Point(Vec3),
    /// Wherever the entity with this name is (followed as it moves).
    Entity(String),
}

/// An entity that walks the navigation mesh by itself (a component):
/// give it a target and it finds the way, steers round corners and
/// other agents, and takes jump links. A character (with a
/// [`CharacterController`]) is moved through its controller and falls
/// with gravity; anything else glides `hover` metres above the mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavAgent {
    /// Top speed (m/s).
    pub speed: f32,
    /// How quickly it speeds up, slows down and turns (m/s²).
    pub acceleration: f32,
    /// How much room it takes up when keeping clear of other agents (m).
    pub radius: f32,
    /// How high its origin floats above the mesh (m); for things that
    /// aren't characters (a drone hovering at head height).
    pub hover: f32,
    /// How close to the target counts as there (m).
    pub stop_distance: f32,
    /// Whether it steers clear of other agents.
    pub avoid: bool,
    /// Whether it turns to face where it's going.
    pub turn: bool,
    /// Where it's heading (set by scripts; not saved).
    #[serde(skip)]
    pub target: Option<NavTarget>,
    /// How it's getting there (not saved).
    #[serde(skip)]
    pub state: AgentState,
}

impl Default for NavAgent {
    fn default() -> Self {
        Self {
            speed: 3.5,
            acceleration: 12.0,
            radius: 0.4,
            hover: 0.0,
            stop_distance: 0.1,
            avoid: true,
            turn: true,
            target: None,
            state: AgentState::default(),
        }
    }
}

/// A script's change to an agent: where to go and how fast. Fields left
/// `None` stay as they are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NavEdit {
    /// A new target (`Some(None)` stops it).
    #[allow(clippy::option_option)] // "unchanged" vs "stop"
    pub target: Option<Option<NavTarget>>,
    /// A new top speed.
    pub speed: Option<f32>,
    /// A new stopping distance.
    pub stop_distance: Option<f32>,
    /// A new hover height (a drone bobbing).
    pub hover: Option<f32>,
}

impl NavEdit {
    /// Makes the change (non-finite or negative numbers are ignored).
    pub fn apply(&self, agent: &mut NavAgent) {
        if let Some(target) = &self.target {
            if *target != agent.target {
                agent.target.clone_from(target);
                // A new target: find the way afresh.
                agent.state.path.clear();
                agent.state.arrived = target.is_none();
            }
        }
        if let Some(v) = self.speed.filter(|v| v.is_finite() && *v >= 0.0) {
            agent.speed = v;
        }
        if let Some(v) = self.stop_distance.filter(|v| v.is_finite() && *v >= 0.0) {
            agent.stop_distance = v;
        }
        if let Some(v) = self.hover.filter(|v| v.is_finite()) {
            agent.hover = v;
        }
    }
}

/// An agent's progress (read it to see what it's doing).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentState {
    /// The way it's following (corners, starting where it was).
    pub path: Vec<PathPoint>,
    /// The corner it's heading for.
    pub next: usize,
    /// Its velocity (m/s), as steered.
    pub velocity: Vec3,
    /// Whether it's where it was going (or going nowhere).
    pub arrived: bool,
    /// Whether it's in the middle of a jump link.
    pub jumping: bool,
    goal: Option<Vec3>,
    repath_in: f32,
    jump: Option<Jump>,
    vertical: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Jump {
    from: Vec3,
    to: Vec3,
    t: f32,
    duration: f32,
    height: f32,
    /// How far the entity's origin is above the path.
    lift: f32,
}

/// What agents step against.
pub struct AgentWorld<'a> {
    /// The mesh (without one, agents head straight for their targets).
    pub nav: Option<&'a NavMesh>,
    /// The time step (seconds).
    pub dt: f32,
    /// Whether a character stands on something.
    pub grounded: &'a dyn Fn(Entity) -> bool,
}

/// What each agent knows of the others.
#[derive(Debug, Clone, Copy)]
struct Neighbour {
    foot: Vec3,
    radius: f32,
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Moves every [`NavAgent`] one step: finds and follows paths, steers,
/// jumps links. Characters get a [`CharacterVelocity`] for the physics
/// step to carry out; other agents' transforms move directly. Agents
/// with nowhere to go leave their transform alone (scripts may move
/// them).
pub fn step_agents(world: &mut hecs::World, at: &AgentWorld<'_>) {
    if at.dt <= 0.0 || !at.dt.is_finite() {
        return;
    }
    let agents: Vec<(Entity, bool)> = world
        .query::<(Entity, &NavAgent, &Transform, Option<&CharacterController>)>()
        .iter()
        .map(|(e, _, _, c)| (e, c.is_some()))
        .collect();
    if agents.is_empty() {
        return;
    }
    let wanted: Vec<String> = agents
        .iter()
        .filter_map(|(e, _)| match world.get::<&NavAgent>(*e).ok()?.target {
            Some(NavTarget::Entity(ref name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut places: HashMap<String, Vec3> = HashMap::new();
    if !wanted.is_empty() {
        for (name, t, g) in &mut world.query::<(&Name, &Transform, Option<&GlobalTransform>)>() {
            if wanted.contains(&name.0) && !places.contains_key(&name.0) {
                let p = g.map_or(t.position, |g| g.0.w_axis.truncate());
                places.insert(name.0.clone(), p);
            }
        }
    }
    let neighbours: Vec<Neighbour> = agents
        .iter()
        .map(|(e, character)| {
            let a = world.get::<&NavAgent>(*e).ok();
            let t = world.get::<&Transform>(*e).ok();
            match (a, t) {
                (Some(a), Some(t)) => Neighbour {
                    foot: foot(&a, t.position, *character),
                    radius: a.radius,
                },
                _ => Neighbour {
                    foot: Vec3::splat(f32::INFINITY),
                    radius: 0.0,
                },
            }
        })
        .collect();
    for (me, (entity, character)) in agents.iter().enumerate() {
        let (Ok(mut agent), Ok(mut t)) = (
            world.get::<&mut NavAgent>(*entity),
            world.get::<&mut Transform>(*entity),
        ) else {
            continue;
        };
        let goal = match &agent.target {
            None => None,
            Some(NavTarget::Point(p)) => Some(*p),
            Some(NavTarget::Entity(name)) => places.get(name).copied(),
        };
        let grounded = *character && (at.grounded)(*entity);
        let velocity = step_one(
            &mut agent,
            &mut t,
            Body {
                character: *character,
                grounded,
            },
            goal,
            &neighbours,
            me,
            at,
        );
        drop((agent, t));
        if let Some(v) = velocity {
            let _ = world.insert_one(*entity, CharacterVelocity::new(v));
        }
    }
}

fn foot(agent: &NavAgent, position: Vec3, character: bool) -> Vec3 {
    if character {
        position
    } else {
        position - Vec3::Y * agent.hover
    }
}

#[derive(Debug, Clone, Copy)]
struct Body {
    character: bool,
    grounded: bool,
}

/// One agent's step. Returns the velocity for a character's controller.
fn step_one(
    agent: &mut NavAgent,
    t: &mut Transform,
    body: Body,
    goal: Option<Vec3>,
    others: &[Neighbour],
    me: usize,
    at: &AgentWorld<'_>,
) -> Option<Vec3> {
    let dt = at.dt;
    let here = foot(agent, t.position, body.character);
    if agent.state.jump.is_some() {
        return jump_step(agent, t, body, here, dt);
    }
    let Some(goal) = goal.filter(|g| g.is_finite()) else {
        agent.state.path.clear();
        agent.state.goal = None;
        agent.state.arrived = true;
        return coast(agent, t, body, here, at);
    };
    let st = &mut agent.state;
    st.repath_in -= dt;
    let moved = st.goal.is_none_or(|g| g.distance(goal) > 0.3);
    if st.path.is_empty() || (st.repath_in <= 0.0 && moved) || st.repath_in <= -1.0 {
        st.path = at
            .nav
            .and_then(|nav| nav.find_path(here, goal, true))
            .unwrap_or_else(|| {
                vec![
                    PathPoint {
                        position: here,
                        link: false,
                    },
                    PathPoint {
                        position: goal,
                        link: false,
                    },
                ]
            });
        st.next = 1;
        st.goal = Some(goal);
        st.repath_in = REPATH;
    }
    // Pass corners already reached; at a link's start, jump.
    let reach = (agent.radius * 0.5).max(0.15).max(agent.speed * dt);
    while st.next < st.path.len() {
        let c = st.path[st.next];
        if c.link {
            let from = st.path[st.next - 1].position;
            if flat(from - here).length() <= reach {
                let length = flat(c.position - from).length();
                let rise = c.position.y - from.y;
                st.jump = Some(Jump {
                    from: Vec3::new(here.x, from.y, here.z),
                    to: c.position,
                    t: 0.0,
                    duration: (length / agent.speed.max(0.5)).max(0.35) + rise.abs() * 0.08,
                    height: 0.4 + rise.max(0.0) + length * 0.15,
                    lift: if body.character {
                        t.position.y - from.y
                    } else {
                        agent.hover
                    },
                });
                st.jumping = true;
                return jump_step(agent, t, body, here, dt);
            }
            break;
        }
        if st.next + 1 < st.path.len() && flat(c.position - here).length() <= reach {
            st.next += 1;
            continue;
        }
        break;
    }
    let last = st.next + 1 >= st.path.len();
    let corner = st.path[st.next.min(st.path.len() - 1)].position;
    let to = flat(corner - here);
    let dist = to.length();
    let stop = agent.stop_distance.max(0.05);
    let mut desired = Vec3::ZERO;
    if st.next >= st.path.len() || (last && dist <= stop) {
        st.arrived = true;
    } else {
        st.arrived = false;
        let mut speed = agent.speed.max(0.0);
        if last {
            speed = speed.min((2.0 * agent.acceleration * (dist - stop).max(0.0)).sqrt());
        }
        desired = to / dist.max(1e-6) * speed;
    }
    if agent.avoid {
        desired = avoid(agent, here, desired, others, me);
    }
    steer(agent, t, body, here, desired, at)
}

/// Gives way to the agents nearby: pushes apart from the ones too close
/// and sidesteps the ones in the way.
fn avoid(agent: &NavAgent, here: Vec3, desired: Vec3, others: &[Neighbour], me: usize) -> Vec3 {
    let heading = desired.normalize_or_zero();
    let mut out = desired;
    for (k, o) in others.iter().enumerate() {
        if k == me || !o.foot.is_finite() || (o.foot.y - here.y).abs() > 2.0 {
            continue;
        }
        let off = flat(here - o.foot);
        let d = off.length();
        let range = agent.radius + o.radius + PERSONAL_SPACE;
        if d >= range {
            continue;
        }
        // Exactly on top of each other: split by who's first.
        let away = if d < 1e-4 {
            if me < k {
                Vec3::X
            } else {
                Vec3::NEG_X
            }
        } else {
            off / d
        };
        let w = (range - d) / range;
        out += away * agent.speed * w * 1.2;
        if heading.dot(-away) > 0.5 {
            // Someone ahead: step to the right of them.
            out += Vec3::new(-heading.z, 0.0, heading.x) * agent.speed * w * 0.6;
        }
    }
    out.clamp_length_max(agent.speed.max(0.0))
}

/// Slows to a stop with nowhere to go. Characters keep falling; others
/// are left where they are once still.
fn coast(
    agent: &mut NavAgent,
    t: &mut Transform,
    body: Body,
    here: Vec3,
    at: &AgentWorld<'_>,
) -> Option<Vec3> {
    if !body.character && agent.state.velocity.length_squared() < 1e-6 {
        agent.state.velocity = Vec3::ZERO;
        return None;
    }
    steer(agent, t, body, here, Vec3::ZERO, at)
}

/// Accelerates towards `desired` and moves (or, for a character, says how
/// to move), staying on the mesh.
fn steer(
    agent: &mut NavAgent,
    t: &mut Transform,
    body: Body,
    here: Vec3,
    desired: Vec3,
    at: &AgentWorld<'_>,
) -> Option<Vec3> {
    let dt = at.dt;
    let st = &mut agent.state;
    let dv = flat(desired) - flat(st.velocity);
    st.velocity = flat(st.velocity) + dv.clamp_length_max(agent.acceleration.max(0.0) * dt);
    let wish = here + st.velocity * dt;
    let next = at.nav.map_or(wish, |nav| nav.move_along(here, wish));
    // Blocked: lose the speed that went nowhere.
    let actual = flat(next - here) / dt;
    if actual.length() + 1e-3 < st.velocity.length() {
        st.velocity = actual;
    }
    face(agent.turn, t, st.velocity, dt);
    if body.character {
        st.vertical = if body.grounded && st.vertical <= 0.0 {
            -2.0
        } else {
            (st.vertical - GRAVITY * dt).max(-50.0)
        };
        return Some(Vec3::new(st.velocity.x, st.vertical, st.velocity.z));
    }
    let y = if at.nav.is_some() {
        let target = next.y + agent.hover;
        t.position.y + (target - t.position.y) * (1.0 - (-SETTLE_RATE * dt).exp())
    } else {
        t.position.y
    };
    t.position = Vec3::new(next.x, y, next.z);
    None
}

/// One step along a jump link's arc.
fn jump_step(
    agent: &mut NavAgent,
    t: &mut Transform,
    body: Body,
    here: Vec3,
    dt: f32,
) -> Option<Vec3> {
    let st = &mut agent.state;
    let mut j = st.jump?;
    j.t = (j.t + dt / j.duration).min(1.0);
    let s = j.t;
    let on_arc = j.from.lerp(j.to, s) + Vec3::Y * (4.0 * j.height * s * (1.0 - s));
    let goal = on_arc + Vec3::Y * j.lift;
    st.velocity = flat(j.to - j.from) / j.duration;
    if s >= 1.0 {
        st.jump = None;
        st.jumping = false;
        st.next += 1;
        st.vertical = 0.0;
        st.repath_in = 0.0;
    } else {
        st.jump = Some(j);
    }
    face(agent.turn, t, st.velocity, dt);
    if body.character {
        let origin = here; // a character's origin is its foot position
        return Some((goal - origin) / dt);
    }
    t.position = goal;
    None
}

/// Turns towards the direction of travel (engine forward is -Z).
fn face(turn: bool, t: &mut Transform, velocity: Vec3, dt: f32) {
    let v = Vec2::new(velocity.x, velocity.z);
    if !turn || v.length() < 0.1 {
        return;
    }
    let want = Quat::from_rotation_y((-v.x).atan2(-v.y));
    t.rotation = t
        .rotation
        .slerp(want, 1.0 - (-TURN_RATE * dt).exp())
        .normalize();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bake, NavSettings};

    fn arena() -> NavMesh {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        crate::bake::tests::add_box(
            &mut v,
            &mut t,
            Vec3::new(-8.0, -1.0, -8.0),
            Vec3::new(8.0, 0.0, 8.0),
        );
        crate::bake::tests::add_box(
            &mut v,
            &mut t,
            Vec3::new(-0.5, 0.0, -8.0),
            Vec3::new(0.5, 2.0, 4.0),
        );
        bake(&v, &t, &[], NavSettings::default()).unwrap()
    }

    fn run(world: &mut hecs::World, nav: Option<&NavMesh>, seconds: f32) {
        let grounded = |_: Entity| true;
        let at = AgentWorld {
            nav,
            dt: 1.0 / 60.0,
            grounded: &grounded,
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for _ in 0..(seconds * 60.0) as usize {
            step_agents(world, &at);
        }
    }

    #[test]
    fn an_agent_walks_round_the_wall_to_its_target() {
        let nav = arena();
        let mut world = hecs::World::new();
        let goal = Vec3::new(4.0, 0.0, -4.0);
        let drone = world.spawn((
            Transform::from_position(Vec3::new(-4.0, 1.5, -4.0)),
            NavAgent {
                hover: 1.5,
                target: Some(NavTarget::Point(goal)),
                ..NavAgent::default()
            },
        ));
        let mut max_z = f32::MIN;
        for _ in 0..40 {
            run(&mut world, Some(&nav), 0.25);
            let p = world.get::<&Transform>(drone).unwrap().position;
            assert!(!(p.x.abs() < 0.5 && p.z < 4.0), "{p} went through the wall");
            max_z = max_z.max(p.z);
        }
        let p = world.get::<&Transform>(drone).unwrap().position;
        assert!(flat(p - goal).length() < 0.2, "{p}");
        assert!((p.y - 1.5).abs() < 0.05, "it hovers: {p}");
        assert!(max_z > 4.0, "it went round the end");
        let a = world.get::<&NavAgent>(drone).unwrap();
        assert!(a.state.arrived);
        // Facing the way it last went (+x then -z, roughly).
        assert!(world.get::<&Transform>(drone).unwrap().rotation != Quat::IDENTITY);
    }

    #[test]
    fn agents_follow_a_named_entity_and_keep_apart() {
        let nav = arena();
        let mut world = hecs::World::new();
        world.spawn((
            Name::new("Player"),
            Transform::from_position(Vec3::new(-5.0, 0.8, 5.0)),
        ));
        let agents: Vec<Entity> = (0..3)
            .map(|i| {
                world.spawn((
                    Transform::from_position(Vec3::new(5.0, 0.0, -5.0 + i as f32 * 0.3)),
                    NavAgent {
                        stop_distance: 1.5,
                        target: Some(NavTarget::Entity("Player".into())),
                        ..NavAgent::default()
                    },
                ))
            })
            .collect();
        run(&mut world, Some(&nav), 10.0);
        let at: Vec<Vec3> = agents
            .iter()
            .map(|a| world.get::<&Transform>(*a).unwrap().position)
            .collect();
        for (i, p) in at.iter().enumerate() {
            assert!(flat(*p - Vec3::new(-5.0, 0.0, 5.0)).length() < 3.0, "{p}");
            for q in &at[i + 1..] {
                assert!(flat(*p - *q).length() > 0.6, "{p} {q} overlap");
            }
        }
    }

    #[test]
    fn characters_get_a_velocity_and_idle_agents_are_left_alone() {
        let nav = arena();
        let mut world = hecs::World::new();
        let walker = world.spawn((
            Transform::from_position(Vec3::new(-4.0, 0.8, 0.0)),
            CharacterController::new(),
            NavAgent {
                target: Some(NavTarget::Point(Vec3::new(-4.0, 0.0, 4.0))),
                ..NavAgent::default()
            },
        ));
        let idle = world.spawn((
            Transform::from_position(Vec3::new(3.0, 2.0, 3.0)),
            NavAgent::default(),
        ));
        run(&mut world, Some(&nav), 0.5);
        let v = world.get::<&CharacterVelocity>(walker).unwrap().velocity;
        assert!(v.z > 1.0 && v.y < 0.0, "{v}");
        // The transform is the physics' job.
        assert_eq!(
            world.get::<&Transform>(walker).unwrap().position,
            Vec3::new(-4.0, 0.8, 0.0)
        );
        assert_eq!(
            world.get::<&Transform>(idle).unwrap().position,
            Vec3::new(3.0, 2.0, 3.0)
        );
        assert!(world.get::<&NavAgent>(idle).unwrap().state.arrived);
    }

    #[test]
    fn agents_jump_links_between_platforms() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        crate::bake::tests::add_box(
            &mut v,
            &mut t,
            Vec3::new(-4.0, -1.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        crate::bake::tests::add_box(
            &mut v,
            &mut t,
            Vec3::new(1.0, -0.5, -2.0),
            Vec3::new(5.0, 0.5, 2.0),
        );
        let nav = bake(&v, &t, &[], NavSettings::default()).unwrap();
        let mut world = hecs::World::new();
        let goal = Vec3::new(3.0, 0.5, 0.0);
        let bot = world.spawn((
            Transform::from_position(Vec3::new(-3.0, 0.0, 0.0)),
            NavAgent {
                target: Some(NavTarget::Point(goal)),
                ..NavAgent::default()
            },
        ));
        let mut highest = f32::MIN;
        let mut jumped = false;
        for _ in 0..240 {
            run(&mut world, Some(&nav), 1.0 / 60.0);
            let p = world.get::<&Transform>(bot).unwrap().position;
            highest = highest.max(p.y);
            jumped |= world.get::<&NavAgent>(bot).unwrap().state.jumping;
        }
        let p = world.get::<&Transform>(bot).unwrap().position;
        assert!(jumped && highest > 0.8, "it arced over the gap ({highest})");
        assert!(
            flat(p - goal).length() < 0.2 && (p.y - 0.5).abs() < 0.05,
            "{p}"
        );
    }

    #[test]
    fn without_a_mesh_agents_head_straight_there() {
        let mut world = hecs::World::new();
        let goal = Vec3::new(3.0, 0.0, 0.0);
        let bot = world.spawn((
            Transform::from_position(Vec3::ZERO),
            NavAgent {
                target: Some(NavTarget::Point(goal)),
                ..NavAgent::default()
            },
        ));
        run(&mut world, None, 3.0);
        let p = world.get::<&Transform>(bot).unwrap().position;
        assert!(p.distance(goal) < 0.15, "{p}");
    }
}
