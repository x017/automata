//! The scene-to-renderer bridge: renderable components and the system that
//! records a scene into the frame's [`DrawList`].
//!
//! Games attach [`MeshRenderer`] / [`SpriteRenderer`] components to
//! entities; [`record_draw_list`] walks the world once per frame and emits
//! draw commands using each entity's [`GlobalTransform`]. The first entity
//! carrying a [`Camera3D`] or [`Camera2D`] component becomes the frame's
//! camera.

use automata_renderer::{Camera2D, Camera3D, DrawList, Material, Mesh, Sprite, Texture};
use glam::{EulerRot, Vec2};

use crate::scene::SceneWorld;
use crate::transform::GlobalTransform;

/// Renders a 3D mesh with a material.
#[derive(Debug, Clone)]
pub struct MeshRenderer {
    /// The mesh to draw.
    pub mesh: Mesh,
    /// The material to draw it with.
    pub material: Material,
}

impl MeshRenderer {
    /// Creates a mesh renderer.
    #[must_use]
    pub fn new(mesh: Mesh, material: Material) -> Self {
        Self { mesh, material }
    }
}

/// Renders a textured 2D quad.
///
/// The sprite is centered on the entity's position; the entity's Z rotation
/// rotates it, and its scale multiplies `size`.
#[derive(Debug, Clone)]
pub struct SpriteRenderer {
    /// The texture to sample.
    pub texture: Texture,
    /// Unscaled size in the 2D camera's units.
    pub size: Vec2,
    /// Tint color, multiplied with the texture.
    pub color: [f32; 4],
    /// Texture coordinate window `(u0, v0, u1, v1)`.
    pub uv: [f32; 4],
}

impl SpriteRenderer {
    /// Creates a sprite renderer showing the full texture, untinted, at the
    /// texture's pixel size.
    #[must_use]
    pub fn new(texture: Texture) -> Self {
        let (w, h) = texture.size();
        Self {
            texture,
            size: Vec2::new(w as f32, h as f32),
            color: [1.0, 1.0, 1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// Sets the unscaled size.
    #[must_use]
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    /// Sets the tint color.
    #[must_use]
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    /// Sets the texture coordinate window.
    #[must_use]
    pub fn uv(mut self, uv: [f32; 4]) -> Self {
        self.uv = uv;
        self
    }
}

/// Records a scene into a draw list: cameras, then meshes, then sprites.
///
/// Entities without a [`GlobalTransform`] are skipped — either their
/// [`crate::Transform`] hasn't propagated yet (the engine propagates once
/// per frame) or they were never meant to render.
pub fn record_draw_list(world: &SceneWorld, draw: &mut DrawList) {
    // Cameras: the first entity with each camera component wins.
    if let Some(camera) = (&mut world.query::<&Camera3D>()).into_iter().next() {
        draw.set_camera_3d(*camera);
    }
    if let Some(camera) = (&mut world.query::<&Camera2D>()).into_iter().next() {
        draw.set_camera_2d(*camera);
    }

    for (renderer, global) in &mut world.query::<(&MeshRenderer, &GlobalTransform)>() {
        draw.mesh(&renderer.mesh, global.matrix(), &renderer.material);
    }

    for (renderer, global) in &mut world.query::<(&SpriteRenderer, &GlobalTransform)>() {
        let (scale, rotation, translation) = global.matrix().to_scale_rotation_translation();
        let (z_rotation, _, _) = rotation.to_euler(EulerRot::ZYX);
        draw.sprite(
            Sprite::new(renderer.texture.clone())
                .position(translation.truncate())
                .size(renderer.size * scale.truncate())
                .rotation(z_rotation)
                .color(renderer.color)
                .uv(renderer.uv),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;
    use glam::Vec3;

    #[test]
    fn camera_components_set_the_frame_cameras() {
        let mut scene = Scene::new();
        scene.spawn((Camera3D::new(Vec3::new(0.0, 2.0, 8.0), Vec3::ZERO),));
        scene.spawn((Camera2D::new(Vec2::ZERO, 100.0),));

        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);

        // Cameras were adopted by the draw list.
        assert!(draw.camera_2d().is_some());
        assert!(draw.camera_3d().is_some());
        assert_eq!(draw.camera_3d().unwrap().position, Vec3::new(0.0, 2.0, 8.0));
    }

    #[test]
    fn entities_without_global_transform_are_skipped() {
        // No Transform -> no GlobalTransform -> nothing recorded. The
        // renderable components need GPU handles, so this test only checks
        // the empty-scene behavior and the camera path above; the full
        // render path is covered by the renderer crate's GPU tests.
        let scene = Scene::new();
        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);
        assert_eq!(draw.sprite_count(), 0);
        assert_eq!(draw.mesh_count(), 0);
    }
}
