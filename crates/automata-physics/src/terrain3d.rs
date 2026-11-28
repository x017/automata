//! Terrains as ground: every [`Terrain`] with `collide` set gets a fixed
//! body with a Rapier heightfield, rebuilt when it's sculpted and moved
//! with its entity.

use automata_scene::{Entity, SceneWorld, Terrain, Transform};
use r3::dynamics::{RigidBodyBuilder, RigidBodyHandle};
use r3::geometry::{Array2, ColliderBuilder};
use r3::math::{Pose, Vector};
use r3::parry::shape::HeightFieldFlags;
use rapier3d as r3;

use super::PhysicsWorld3D;
use crate::world::entity_to_user_data;

/// A terrain's body: what it was built from, to tell when to rebuild.
#[derive(Debug, Clone, Copy)]
pub(super) struct TerrainBody {
    body: RigidBodyHandle,
    revision: u64,
    size: f32,
    transform: Transform,
}

/// `terrain`'s heights as a parry heightfield matrix: rows run along Z,
/// columns along X, stored column by column.
fn heightfield(terrain: &Terrain) -> Array2<f32> {
    let r = terrain.resolution() as usize;
    let heights = terrain.heights();
    let mut data = vec![0.0; r * r];
    for z in 0..r {
        for x in 0..r {
            data[x * r + z] = heights[z * r + x];
        }
    }
    Array2::new(r, r, data)
}

impl PhysicsWorld3D {
    /// Builds, rebuilds, moves and removes terrain bodies to match the
    /// scene's [`Terrain`]s.
    pub(super) fn sync_terrains(&mut self, scene: &SceneWorld) {
        let mut seen = Vec::new();
        for (entity, terrain, transform) in
            &mut scene.query::<(Entity, &Terrain, Option<&Transform>)>()
        {
            // An explicit collider or body takes over.
            if !terrain.collide
                || scene.get::<&crate::Collider3D>(entity).is_ok()
                || scene.get::<&crate::RigidBody>(entity).is_ok()
            {
                continue;
            }
            seen.push(entity);
            let t = transform.copied().unwrap_or_default();
            let size = terrain.size;
            match self.terrains.get(&entity).copied() {
                Some(last)
                    if last.revision == terrain.revision()
                        && last.size.to_bits() == size.to_bits()
                        && last.transform.scale == t.scale =>
                {
                    if last.transform.position != t.position
                        || last.transform.rotation != t.rotation
                    {
                        if let Some(rb) = self.bodies.get_mut(last.body) {
                            rb.set_position(
                                Pose::from_parts(
                                    Vector::new(t.position.x, t.position.y, t.position.z),
                                    t.rotation,
                                ),
                                true,
                            );
                        }
                        self.terrains.insert(
                            entity,
                            TerrainBody {
                                transform: t,
                                ..last
                            },
                        );
                    }
                }
                _ => {
                    self.remove_terrain(entity);
                    self.add_terrain(entity, terrain, t);
                }
            }
        }
        let gone: Vec<Entity> = self
            .terrains
            .keys()
            .filter(|e| !seen.contains(e))
            .copied()
            .collect();
        for entity in gone {
            self.remove_terrain(entity);
        }
    }

    fn add_terrain(&mut self, entity: Entity, terrain: &Terrain, t: Transform) {
        let body = self.bodies.insert(
            RigidBodyBuilder::fixed()
                .translation(Vector::new(t.position.x, t.position.y, t.position.z))
                .rotation(t.rotation.to_scaled_axis())
                .build(),
        );
        let scale = Vector::new(
            terrain.size * t.scale.x,
            t.scale.y,
            terrain.size * t.scale.z,
        );
        let collider = ColliderBuilder::heightfield_with_flags(
            heightfield(terrain),
            scale,
            HeightFieldFlags::FIX_INTERNAL_EDGES,
        )
        .friction(0.8)
        .user_data(entity_to_user_data(entity))
        .build();
        self.colliders
            .insert_with_parent(collider, body, &mut self.bodies);
        self.terrains.insert(
            entity,
            TerrainBody {
                body,
                revision: terrain.revision(),
                size: terrain.size,
                transform: t,
            },
        );
        tracing::trace!(?entity, "physics: built terrain heightfield");
    }

    fn remove_terrain(&mut self, entity: Entity) {
        if let Some(old) = self.terrains.remove(&entity) {
            self.bodies.remove(
                old.body,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.soft_bodies,
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Collider3D, RayFilter, RigidBody, Shape3D};
    use automata_scene::{Scene, Sculpt};
    use glam::{Vec2, Vec3};

    fn slope() -> Terrain {
        // Rises 0.25 m per metre along +X, so a mix-up of rows and
        // columns would show.
        let mut t = Terrain::new(32.0, 17);
        t.set_heights_with(|x, _| 4.0 + 0.25 * x);
        t
    }

    #[test]
    fn rays_and_bodies_meet_the_terrain_where_it_is_drawn() {
        let mut scene = Scene::new();
        let ground = scene.spawn((Transform::from_position(Vec3::new(5.0, 1.0, 0.0)), slope()));
        let mut physics = PhysicsWorld3D::new();
        physics.sync_from_scene(scene.world_mut());
        physics.step(scene.world_mut(), 1.0 / 60.0);
        for (x, z) in [(5.0, 0.0), (12.0, -3.0), (-2.0, 9.0)] {
            let hit = physics
                .cast_ray(
                    Vec3::new(x, 50.0, z),
                    -Vec3::Y,
                    100.0,
                    &RayFilter::default(),
                )
                .expect("over the terrain");
            assert_eq!(hit.entity, ground);
            let want = 1.0 + 4.0 + 0.25 * (x - 5.0);
            assert!(
                (hit.point.y - want).abs() < 1e-3,
                "{x},{z}: {} vs {want}",
                hit.point.y
            );
        }
        assert!(physics
            .cast_ray(
                Vec3::new(30.0, 50.0, 0.0),
                -Vec3::Y,
                100.0,
                &RayFilter::default()
            )
            .is_none());

        // A ball dropped on it comes to rest on it.
        let ball = scene.spawn((
            Transform::from_position(Vec3::new(5.0, 12.0, 0.0)),
            RigidBody::dynamic(),
            Collider3D::new(Shape3D::Ball { radius: 0.5 }),
        ));
        physics.sync_from_scene(scene.world_mut());
        for _ in 0..120 {
            physics.step(scene.world_mut(), 1.0 / 60.0);
        }
        // (It rolls downhill, so measure from the ground under it.)
        let p = scene.world().get::<&Transform>(ball).unwrap().position;
        let above = p.y - (1.0 + 4.0 + 0.25 * (p.x - 5.0));
        assert!(
            above > 0.4 && above < 0.7,
            "the ball rides on the ground: {above} above it"
        );

        // Sculpting rebuilds it; turning collision off removes it.
        scene
            .world_mut()
            .get::<&mut Terrain>(ground)
            .unwrap()
            .sculpt(Sculpt::Raise, Vec2::ZERO, 6.0, 3.0);
        physics.sync_from_scene(scene.world_mut());
        physics.step(scene.world_mut(), 1.0 / 60.0);
        let raised = physics
            .cast_ray(
                Vec3::new(5.0, 50.0, 0.0),
                -Vec3::Y,
                100.0,
                &RayFilter::default(),
            )
            .unwrap();
        assert!(raised.point.y > 5.5);
        // Snapshots (script raycasts) see it as a grid of triangles that
        // matches the live world.
        let snap = physics.collider_snapshot();
        let (_, shape, _) = snap
            .shapes()
            .into_iter()
            .find(|(e, _, _)| *e == ground)
            .unwrap();
        let crate::WorldShape::Mesh { mesh, center, .. } = shape else {
            panic!("a terrain is a mesh to tools: {shape:?}");
        };
        assert_eq!(snap.mesh_grid(mesh), Some([17, 17]));
        let (verts, tris) = snap.mesh_triangles(mesh).unwrap();
        assert_eq!((verts.len(), tris.len()), (17 * 17, 16 * 16 * 2));
        let corner = verts[16] + center; // last column of the first row: +X, -Z
        assert!((corner.x - 21.0).abs() < 1e-4 && (corner.z + 16.0).abs() < 1e-4);
        assert!((corner.y - (1.0 + 4.0 + 0.25 * 16.0)).abs() < 1e-4);
        let snap_hit = snap
            .cast_ray(
                Vec3::new(5.0, 50.0, 0.0),
                -Vec3::Y,
                100.0,
                &RayFilter::default(),
            )
            .unwrap();
        assert!((snap_hit.point.y - raised.point.y).abs() < 1e-4);

        scene
            .world_mut()
            .get::<&mut Terrain>(ground)
            .unwrap()
            .collide = false;
        physics.sync_from_scene(scene.world_mut());
        physics.step(scene.world_mut(), 1.0 / 60.0);
        let hit = physics.cast_ray(
            Vec3::new(-8.0, 50.0, 0.0),
            -Vec3::Y,
            100.0,
            &RayFilter::default(),
        );
        assert!(hit.is_none(), "no ground left: {hit:?}");
    }
}
