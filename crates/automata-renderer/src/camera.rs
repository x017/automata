//! Cameras producing view-projection matrices for rendering.
//!
//! Conventions: right-handed world space, +Y up, view space looks down -Z.
//! Projection matrices use wgpu's NDC convention (Z in `[0, 1]`, Y up) via
//! `glam::camera::rh::proj::directx`.

use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Vec2, Vec3};

/// A 2D orthographic camera.
///
/// The view is centered on [`Self::center`] and spans
/// `2 × half_height` world units vertically; the horizontal span follows the
/// viewport's aspect ratio. World space has +Y up.
#[derive(Debug, Clone, Copy)]
pub struct Camera2D {
    /// Center of the view in world units.
    pub center: Vec2,
    /// Half of the vertical view extent, in world units.
    pub half_height: f32,
}

impl Camera2D {
    /// Creates a 2D camera centered at `center` with the given vertical
    /// half-extent.
    #[must_use]
    pub fn new(center: Vec2, half_height: f32) -> Self {
        Self {
            center,
            half_height,
        }
    }

    /// The view-projection matrix for a viewport of `size` pixels.
    #[must_use]
    pub fn view_proj(&self, viewport: (u32, u32)) -> Mat4 {
        let aspect = viewport.0 as f32 / viewport.1.max(1) as f32;
        let half_width = self.half_height * aspect;
        directx::orthographic(
            self.center.x - half_width,
            self.center.x + half_width,
            self.center.y - self.half_height,
            self.center.y + self.half_height,
            -1.0,
            1.0,
        )
    }
}

/// A 3D perspective camera.
#[derive(Debug, Clone, Copy)]
pub struct Camera3D {
    /// Camera position in world space.
    pub position: Vec3,
    /// The point the camera looks at.
    pub target: Vec3,
    /// The up direction. Must be normalized and not parallel to the view
    /// direction.
    pub up: Vec3,
    /// Vertical field of view, in degrees.
    pub fov_y_degrees: f32,
    /// Near clip plane distance.
    pub near: f32,
    /// Far clip plane distance.
    pub far: f32,
}

impl Camera3D {
    /// Creates a camera at `position` looking at `target`, with a 60°
    /// vertical field of view and clip planes at 0.1 and 1000.
    #[must_use]
    pub fn new(position: Vec3, target: Vec3) -> Self {
        Self {
            position,
            target,
            up: Vec3::Y,
            fov_y_degrees: 60.0,
            near: 0.1,
            far: 1000.0,
        }
    }

    /// The view-projection matrix for a viewport of `size` pixels.
    #[must_use]
    pub fn view_proj(&self, viewport: (u32, u32)) -> Mat4 {
        let aspect = viewport.0 as f32 / viewport.1.max(1) as f32;
        let view = view::look_at_mat4(self.position, self.target, self.up);
        let proj =
            directx::perspective(self.fov_y_degrees.to_radians(), aspect, self.near, self.far);
        proj * view
    }
}

impl Default for Camera3D {
    /// A camera at `(0, 0, 5)` looking at the origin — useful as a fallback
    /// when a frame draws meshes without setting a camera.
    fn default() -> Self {
        Self::new(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO)
    }
}

/// The implicit 2D camera used when a frame draws sprites without setting
/// one: one world unit per pixel, origin at the top-left corner, +Y down.
pub(crate) fn screen_space_camera(viewport: (u32, u32)) -> Mat4 {
    directx::orthographic(0.0, viewport.0 as f32, viewport.1 as f32, 0.0, -1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: (u32, u32) = (1600, 900);

    #[test]
    fn camera_3d_looks_down_minus_z() {
        // Points straight ahead of the camera project to the NDC center.
        let camera = Camera3D::new(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
        let ndc = camera.view_proj(VIEWPORT).project_point3(Vec3::ZERO);
        assert!(
            ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5,
            "target should be centered: {ndc:?}"
        );
        assert!(
            (0.0..=1.0).contains(&ndc.z),
            "wgpu NDC z must be in [0, 1]: {ndc:?}"
        );
    }

    #[test]
    fn camera_3d_near_plane_maps_to_zero_depth() {
        let camera = Camera3D::new(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
        let point_on_near_plane = Vec3::new(0.0, 0.0, 5.0 - camera.near);
        let ndc = camera
            .view_proj(VIEWPORT)
            .project_point3(point_on_near_plane);
        assert!(
            ndc.z.abs() < 1e-4,
            "near plane should map to depth 0 in wgpu NDC: {ndc:?}"
        );
    }

    #[test]
    fn camera_2d_centers_its_center() {
        let camera = Camera2D::new(Vec2::new(100.0, 50.0), 10.0);
        let ndc = camera
            .view_proj(VIEWPORT)
            .project_point3(Vec3::new(100.0, 50.0, 0.0));
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5, "{ndc:?}");
    }

    #[test]
    fn camera_2d_respects_aspect_ratio() {
        let camera = Camera2D::new(Vec2::ZERO, 10.0);
        let vp = camera.view_proj(VIEWPORT);
        // The top edge (y = +half_height) maps to NDC y = +1.
        let top = vp.project_point3(Vec3::new(0.0, 10.0, 0.0));
        assert!((top.y - 1.0).abs() < 1e-5, "{top:?}");
        // The right edge spans half_width = half_height * aspect.
        let right = vp.project_point3(Vec3::new(10.0 * (1600.0 / 900.0), 0.0, 0.0));
        assert!((right.x - 1.0).abs() < 1e-4, "{right:?}");
    }

    #[test]
    fn screen_space_maps_pixels_top_left_origin() {
        let vp = screen_space_camera((800, 600));
        let origin = vp.project_point3(Vec3::ZERO);
        assert!((origin.x + 1.0).abs() < 1e-5 && (origin.y - 1.0).abs() < 1e-5);
        let corner = vp.project_point3(Vec3::new(800.0, 600.0, 0.0));
        assert!((corner.x - 1.0).abs() < 1e-5 && (corner.y + 1.0).abs() < 1e-5);
    }
}
