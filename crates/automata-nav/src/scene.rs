//! Navigation for a whole scene: what it's baked from, and loading the
//! saved mesh (or baking afresh when the level changed).

use std::path::Path;

use automata_physics::PhysicsWorld3D;
use automata_scene::{GlobalTransform, Name, Transform};
use glam::Vec3;

use crate::{bake, geometry_hash, BakeError, Link, NavAgent, NavLink, NavMesh, Navigation};

/// What a scene's mesh is baked from: the static colliders' triangles
/// (agents left out) and its [`NavLink`]s, as `(vertices, triangles,
/// links)`. `physics` must have been synced with the scene.
#[must_use]
pub fn bake_input(
    world: &hecs::World,
    physics: &PhysicsWorld3D,
) -> (Vec<Vec3>, Vec<[u32; 3]>, Vec<Link>) {
    let skip = |e: hecs::Entity| world.get::<&NavAgent>(e).is_ok();
    let (vertices, triangles) = physics.static_geometry(&skip);
    (vertices, triangles, links(world))
}

/// Where an entity is in the world.
fn place(world: &hecs::World, e: hecs::Entity) -> Option<Vec3> {
    if let Ok(g) = world.get::<&GlobalTransform>(e) {
        return Some(g.0.w_axis.truncate());
    }
    world.get::<&Transform>(e).ok().map(|t| t.position)
}

/// The scene's hand-placed links, from each [`NavLink`] to the entity it
/// names (links to nobody are left out).
#[must_use]
pub fn links(world: &hecs::World) -> Vec<Link> {
    let mut out = Vec::new();
    for (e, link) in &mut world.query::<(hecs::Entity, &NavLink)>() {
        let Some(start) = place(world, e) else {
            continue;
        };
        let end = world
            .query::<(hecs::Entity, &Name)>()
            .iter()
            .find(|(_, n)| n.0 == link.to)
            .and_then(|(t, _)| place(world, t));
        if let Some(end) = end {
            out.push(Link {
                start,
                end,
                two_way: link.two_way,
                auto: false,
            });
        }
    }
    out
}

/// The scene's [`Navigation`] settings (the first entity's that has one).
#[must_use]
pub fn navigation(world: &hecs::World) -> Option<Navigation> {
    world.query::<&Navigation>().iter().next().cloned()
}

/// Bakes the scene's mesh with `settings`.
///
/// # Errors
///
/// See [`bake`].
pub fn bake_scene(
    world: &hecs::World,
    physics: &PhysicsWorld3D,
    settings: crate::NavSettings,
) -> Result<NavMesh, BakeError> {
    let (v, t, l) = bake_input(world, physics);
    bake(&v, &t, &l, settings)
}

/// A scene's mesh, ready to walk, and what the player should hear about
/// how it got it.
#[derive(Debug)]
pub struct Prepared {
    /// The mesh (`None` when it couldn't be baked).
    pub mesh: Option<NavMesh>,
    /// A warning for the console, if any.
    pub warning: Option<String>,
}

/// Gets a scene ready to navigate when it starts: loads the mesh its
/// [`Navigation`] was baked to (`file`, relative to `base`, the scene's
/// folder) if it still matches the level, else bakes it afresh. A scene
/// without a `Navigation` but with agents bakes with the default
/// settings; a scene with neither gets `None`. `physics` must have been
/// synced with the scene.
#[must_use]
pub fn prepare(
    world: &hecs::World,
    physics: &PhysicsWorld3D,
    base: Option<&Path>,
) -> Option<Prepared> {
    let nav = navigation(world);
    if nav.is_none() && world.query::<&NavAgent>().iter().next().is_none() {
        return None;
    }
    let nav = nav.unwrap_or_default();
    let settings = nav.settings.sanitized();
    let (v, t, l) = bake_input(world, physics);
    let mut warning = None;
    if !nav.file.is_empty() {
        let path = base.map_or_else(|| Path::new(&nav.file).to_path_buf(), |b| b.join(&nav.file));
        match NavMesh::load(&path) {
            Ok(mesh) if mesh.hash == geometry_hash(&v, &t, &l, &settings) => {
                return Some(Prepared {
                    mesh: Some(mesh),
                    warning: None,
                });
            }
            Ok(_) => {
                warning = Some(format!(
                    "navigation: {} is out of date (the level changed since it was baked), \
                     so it was baked again; press Bake in Oberon to save it",
                    nav.file
                ));
            }
            Err(e) => {
                warning = Some(format!(
                    "navigation: couldn't load {}: {e}; baked it again",
                    nav.file
                ));
            }
        }
    }
    match bake(&v, &t, &l, settings) {
        Ok(mesh) => Some(Prepared {
            mesh: Some(mesh),
            warning,
        }),
        Err(e) => Some(Prepared {
            mesh: None,
            warning: Some(format!("navigation: {e}")),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_physics::{Collider3D, RigidBody};

    fn level() -> hecs::World {
        let mut world = hecs::World::new();
        world.spawn((
            Transform::from_position(Vec3::new(0.0, -0.5, 0.0)),
            Collider3D::cuboid(10.0, 1.0, 10.0),
        ));
        world.spawn((
            Name::new("Crate"),
            Transform::from_position(Vec3::new(2.0, 0.5, 0.0)),
            RigidBody::dynamic(),
            Collider3D::cuboid(1.0, 1.0, 1.0),
        ));
        world.spawn((
            Name::new("Drone"),
            Transform::from_position(Vec3::new(-2.0, 1.5, 0.0)),
            Collider3D::ball(0.5),
            NavAgent::default(),
        ));
        world.spawn((
            Transform::from_position(Vec3::new(-3.0, 0.0, -3.0)),
            NavLink {
                to: "Crate".into(),
                two_way: false,
            },
        ));
        world
    }

    #[test]
    fn scenes_bake_from_static_colliders_only() {
        let mut world = level();
        let mut physics = PhysicsWorld3D::new();
        physics.sync_from_scene(&mut world);
        let (v, t, l) = bake_input(&world, &physics);
        // The floor's box alone: the crate moves, the drone is an agent.
        assert_eq!((v.len(), t.len()), (8, 12));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].end, Vec3::new(2.0, 0.5, 0.0));
        let prepared = prepare(&world, &physics, None).unwrap();
        assert!(prepared.warning.is_none());
        assert!(prepared.mesh.unwrap().area() > 80.0);
    }

    #[test]
    fn saved_meshes_are_used_until_the_level_changes() {
        let dir = std::env::temp_dir().join(format!("automata-nav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut world = level();
        let mut physics = PhysicsWorld3D::new();
        physics.sync_from_scene(&mut world);
        // A radius that tells the saved mesh from a fresh default bake.
        let s = crate::NavSettings {
            agent_radius: 1.0,
            ..crate::NavSettings::default()
        };
        let mesh = bake_scene(&world, &physics, s).unwrap();
        std::fs::write(dir.join("level.nav.ron"), mesh.to_ron().unwrap()).unwrap();
        world.spawn((Navigation {
            settings: s,
            file: "level.nav.ron".into(),
        },));
        let p = prepare(&world, &physics, Some(&dir)).unwrap();
        assert!(p.warning.is_none());
        assert_eq!(p.mesh.unwrap(), mesh);

        // A new wall: the saved mesh is stale, so it's baked again.
        world.spawn((
            Transform::from_position(Vec3::new(0.0, 1.0, 3.0)),
            Collider3D::cuboid(4.0, 2.0, 0.5),
        ));
        physics.sync_from_scene(&mut world);
        let p = prepare(&world, &physics, Some(&dir)).unwrap();
        assert!(p.warning.unwrap().contains("out of date"));
        assert_ne!(p.mesh.unwrap(), mesh);
        let _ = std::fs::remove_dir_all(dir);

        // Neither navigation nor agents: nothing to do.
        let mut bare = hecs::World::new();
        bare.spawn((Transform::IDENTITY, Collider3D::cuboid(1.0, 1.0, 1.0)));
        physics = PhysicsWorld3D::new();
        physics.sync_from_scene(&mut bare);
        assert!(prepare(&bare, &physics, None).is_none());
    }
}
