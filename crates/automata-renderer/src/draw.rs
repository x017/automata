//! The draw list: a frame's worth of rendering commands.
//!
//! Game code records a [`DrawList`] during its update; the renderer
//! executes it when presenting the frame. The list is pure CPU data — no GPU
//! interaction happens until presentation, and headless applications can
//! record frames without a GPU.
//!
//! Render order within a frame: the frame is cleared to the clear color, 3D
//! meshes are drawn (with depth testing), then water surfaces, then 2D
//! sprites on top.

use std::sync::Arc;

use glam::{Mat4, Vec2};

use crate::camera::{Camera2D, Camera3D};
use crate::material::Material;
use crate::mesh::Mesh;
use crate::texture::Texture;
use crate::Color;

/// A frame's rendering commands.
///
/// Recreated by the engine each frame; obtained from the application context
/// during update.
#[derive(Debug)]
pub struct DrawList {
    pub(crate) clear_color: Color,
    pub(crate) camera_2d: Option<Camera2D>,
    pub(crate) camera_3d: Option<Camera3D>,
    pub(crate) sun: Option<crate::Sun>,
    pub(crate) environment: Option<crate::Environment>,
    pub(crate) sprites: Vec<Sprite>,
    pub(crate) meshes: Vec<MeshDraw>,
    pub(crate) waters: Vec<crate::water::WaterDraw>,
    pub(crate) point_lights: Vec<crate::PointLight>,
    pub(crate) time: Option<f32>,
    pub(crate) ripples: Option<crate::RippleMap>,
}

impl DrawList {
    /// Creates an empty draw list with the default clear color.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the color the frame is cleared to before drawing.
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Sets the 2D camera for this frame's sprites.
    ///
    /// Without a camera, sprites are drawn in screen-space pixels with the
    /// origin at the window's top-left corner and +Y pointing down.
    pub fn set_camera_2d(&mut self, camera: Camera2D) {
        self.camera_2d = Some(camera);
    }

    /// Sets the 3D camera for this frame's meshes.
    ///
    /// Without a camera, meshes are viewed from a default camera at
    /// `(0, 0, 5)` looking at the origin.
    pub fn set_camera_3d(&mut self, camera: Camera3D) {
        self.camera_3d = Some(camera);
    }

    /// Sets the directional sun for this frame's meshes.
    ///
    /// Without a sun, meshes are lit by a default shadowless light.
    pub fn set_sun(&mut self, sun: crate::Sun) {
        self.sun = Some(sun);
    }

    /// Sets the frame's environment (ambient, fog, exposure, tone mapping).
    ///
    /// Without one, [`Environment::default`](crate::Environment::default)
    /// applies, which matches the original unfogged, untonemapped look.
    pub fn set_environment(&mut self, environment: crate::Environment) {
        self.environment = Some(environment);
    }

    /// Draws a sprite. Sprites render after meshes and ignore depth.
    pub fn sprite(&mut self, sprite: Sprite) {
        self.sprites.push(sprite);
    }

    /// Draws a mesh with the given model transform and material.
    pub fn mesh(&mut self, mesh: &Mesh, model: Mat4, material: &Material) {
        self.meshes.push(MeshDraw {
            mesh: mesh.clone(),
            model,
            material: material.clone(),
            joints: None,
        });
    }

    /// Draws a skinned mesh posed by `joints`: one matrix per joint, taking
    /// a vertex from bind space to model space (then `model` places it in
    /// the world). Entries past [`crate::MAX_JOINTS`] are ignored. A mesh
    /// without skin data draws as with [`Self::mesh`].
    pub fn skinned_mesh(
        &mut self,
        mesh: &Mesh,
        model: Mat4,
        material: &Material,
        joints: &Arc<[Mat4]>,
    ) {
        self.meshes.push(MeshDraw {
            mesh: mesh.clone(),
            model,
            material: material.clone(),
            joints: mesh.is_skinned().then(|| Arc::clone(joints)),
        });
    }

    /// Draws a water surface (see [`crate::Water`]) placed by `model`.
    pub fn water(&mut self, water: crate::Water, model: Mat4) {
        self.waters.push(crate::water::WaterDraw { water, model });
    }

    /// Adds a point light. Glowing meshes add their own; when a frame has
    /// more lights than the quality preset shades, the ones that matter
    /// most to the camera win.
    pub fn point_light(&mut self, light: crate::PointLight) {
        self.point_lights.push(light);
    }

    /// The point lights added this frame (not counting glowing meshes).
    #[must_use]
    pub fn point_lights(&self) -> &[crate::PointLight] {
        &self.point_lights
    }

    /// Sets the animation time (seconds) for time-driven effects such as
    /// water waves. Without one, the renderer uses its own clock.
    pub fn set_time(&mut self, seconds: f32) {
        self.time = Some(seconds);
    }

    /// Sets the simulated ripples drawn on this frame's water (see
    /// [`crate::RippleMap`]; physics simulates them).
    pub fn set_ripples(&mut self, ripples: crate::RippleMap) {
        self.ripples = Some(ripples);
    }

    /// The number of water surfaces recorded so far.
    #[must_use]
    pub fn water_count(&self) -> usize {
        self.waters.len()
    }

    /// The number of sprite draw commands recorded so far.
    #[must_use]
    pub fn sprite_count(&self) -> usize {
        self.sprites.len()
    }

    /// The number of mesh draw commands recorded so far.
    #[must_use]
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// The active 2D camera, if one was set.
    #[must_use]
    pub fn camera_2d(&self) -> Option<Camera2D> {
        self.camera_2d
    }

    /// The active 3D camera, if one was set.
    #[must_use]
    pub fn camera_3d(&self) -> Option<Camera3D> {
        self.camera_3d
    }

    /// The clear color for the frame.
    #[must_use]
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// The frame's directional sun, if one was set.
    #[must_use]
    pub fn sun(&self) -> Option<crate::Sun> {
        self.sun
    }

    /// The frame's environment, if one was set.
    #[must_use]
    pub fn environment(&self) -> Option<crate::Environment> {
        self.environment
    }

    /// Resets the list for the next frame, keeping allocations. Called by
    /// the engine after each presented frame.
    pub fn clear(&mut self) {
        self.camera_2d = None;
        self.camera_3d = None;
        self.sun = None;
        self.environment = None;
        self.sprites.clear();
        self.meshes.clear();
        self.waters.clear();
        self.point_lights.clear();
        self.time = None;
        self.ripples = None;
        // The clear color intentionally persists across frames.
    }
}

impl Default for DrawList {
    fn default() -> Self {
        Self {
            clear_color: Color {
                r: 0.02,
                g: 0.02,
                b: 0.04,
                a: 1.0,
            },
            camera_2d: None,
            camera_3d: None,
            sun: None,
            environment: None,
            sprites: Vec::new(),
            meshes: Vec::new(),
            waters: Vec::new(),
            point_lights: Vec::new(),
            time: None,
            ripples: None,
        }
    }
}

/// A single mesh draw command.
#[derive(Debug, Clone)]
pub(crate) struct MeshDraw {
    pub mesh: Mesh,
    pub model: Mat4,
    pub material: Material,
    /// The joint palette, for skinned meshes.
    pub joints: Option<Arc<[Mat4]>>,
}

/// A textured 2D quad.
///
/// The position refers to the sprite's center. Sizes are in the units of the
/// active 2D camera — pixels under the default screen-space camera.
#[derive(Debug, Clone)]
pub struct Sprite {
    /// The texture to sample.
    pub texture: Texture,
    /// Center position.
    pub position: Vec2,
    /// Size in the 2D camera's units.
    pub size: Vec2,
    /// Rotation around the center, in radians (counter-clockwise).
    pub rotation: f32,
    /// Tint color, multiplied with the texture.
    pub color: [f32; 4],
    /// Texture coordinate window `(u0, v0, u1, v1)` — sub-rectangles select
    /// frames from a sprite sheet.
    pub uv: [f32; 4],
}

impl Sprite {
    /// Creates a sprite showing the full texture, unrotated, untinted, sized
    /// to the texture's pixel dimensions, centered at the origin.
    #[must_use]
    pub fn new(texture: Texture) -> Self {
        let (w, h) = texture.size();
        Self {
            texture,
            position: Vec2::ZERO,
            size: Vec2::new(w as f32, h as f32),
            rotation: 0.0,
            color: [1.0, 1.0, 1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// Sets the center position.
    #[must_use]
    pub fn position(mut self, position: Vec2) -> Self {
        self.position = position;
        self
    }

    /// Sets the size.
    #[must_use]
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    /// Sets the rotation in radians.
    #[must_use]
    pub fn rotation(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// Sets the tint color.
    #[must_use]
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    /// Sets the texture coordinate window `(u0, v0, u1, v1)`.
    #[must_use]
    pub fn uv(mut self, uv: [f32; 4]) -> Self {
        self.uv = uv;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_resets_frame_state_but_keeps_clear_color() {
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_2d(Camera2D::new(Vec2::ZERO, 100.0));
        draw.clear();

        assert_eq!(draw.clear_color, Color::BLACK);
        assert!(draw.camera_2d.is_none());
        assert!(draw.sprites.is_empty());
        assert!(draw.meshes.is_empty());
    }

    #[test]
    fn default_clear_color_is_dark() {
        let draw = DrawList::new();
        assert!(draw.clear_color.a > 0.99);
        assert!(draw.clear_color.r < 0.1);
    }
}
