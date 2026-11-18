//! The draw list: a frame's worth of rendering commands.
//!
//! Game code records a [`DrawList`] during its update; the renderer
//! executes it when presenting the frame. The list is pure CPU data — no GPU
//! interaction happens until presentation, and headless applications can
//! record frames without a GPU.
//!
//! Render order within a frame: the frame is cleared to the clear color, 3D
//! meshes are drawn (with depth testing), then 2D sprites are drawn on top.

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
    pub(crate) sprites: Vec<Sprite>,
    pub(crate) meshes: Vec<MeshDraw>,
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
        });
    }

    /// Resets the list for the next frame, keeping allocations. Called by
    /// the engine after each presented frame.
    pub fn clear(&mut self) {
        self.camera_2d = None;
        self.camera_3d = None;
        self.sprites.clear();
        self.meshes.clear();
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
            sprites: Vec::new(),
            meshes: Vec::new(),
        }
    }
}

/// A single mesh draw command.
#[derive(Debug, Clone)]
pub(crate) struct MeshDraw {
    pub mesh: Mesh,
    pub model: Mat4,
    pub material: Material,
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
