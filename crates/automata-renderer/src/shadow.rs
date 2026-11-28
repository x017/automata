//! Sun shadow setup: the light-space matrices the shadow pass renders with
//! and the mesh shader samples through.
//!
//! One cascade fits a single orthographic map around every mesh (cheap,
//! stable, blurry in big levels). Several cascades split the camera's view
//! distance so each slice gets its own map: the slice nearest the camera
//! covers a few metres at full resolution, the farthest covers the horizon.
//! Each cascade's box is a bounding *sphere* of its view slice, snapped to
//! whole shadow texels, so shadows don't shimmer as the camera turns or
//! moves.

use glam::{Mat4, Vec3, Vec4Swizzles};

use crate::Camera3D;

/// The most cascades a frame can use (the size of the shader's arrays).
pub(crate) const MAX_CASCADES: usize = 4;

/// Blend between uniform and logarithmic splits (0 = uniform, 1 = log).
const SPLIT_LAMBDA: f32 = 0.8;

/// A world-space bounding sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

/// The frame's shadow cascades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShadowCascades {
    /// Light view-projection per cascade (unused entries are identity).
    pub view_proj: [Mat4; MAX_CASCADES],
    /// Light view (before projection) per cascade, for culling.
    pub view: [Mat4; MAX_CASCADES],
    /// Half-extent of each cascade's square box (world units).
    pub half_extent: [f32; MAX_CASCADES],
    /// Depth extent of each cascade's box (world units).
    pub depth_range: [f32; MAX_CASCADES],
    /// Far edge of each cascade as view depth (distance along the camera's
    /// forward axis); single-map setups use `f32::MAX`.
    pub splits: [f32; MAX_CASCADES],
    /// How many entries are in use (`1..=MAX_CASCADES`).
    pub count: usize,
}

impl ShadowCascades {
    /// A single cascade around the whole scene (the classic setup).
    pub fn fit_scene(scene: Sphere, direction: Vec3, map_size: u32) -> Self {
        let radius = scene.radius.max(1.0) * 1.1; // 10% margin
        let (view, view_proj, depth) =
            cascade_matrices(scene.center, radius, scene, direction, map_size);
        let mut out = Self::empty();
        out.depth_range[0] = depth;
        out.view[0] = view;
        out.view_proj[0] = view_proj;
        out.half_extent[0] = radius;
        out.splits[0] = f32::MAX;
        out.count = 1;
        out
    }

    /// The frame's `count` cascades: slices of the camera's view out to
    /// `distance`, or less when the whole scene ends sooner (rounded up in
    /// ~19% steps, so the slices only change size now and then and don't
    /// shimmer). A single cascade covers the scene or that view, whichever
    /// is smaller.
    #[allow(clippy::too_many_arguments)]
    pub fn fit(
        camera: &Camera3D,
        aspect: f32,
        scene: Sphere,
        direction: Vec3,
        map_size: u32,
        count: usize,
        distance: f32,
    ) -> Self {
        let reach = camera.position.distance(scene.center) + scene.radius;
        let far = quantize_up(reach).min(distance.max(1.0));
        let view = Self::fit_camera(camera, aspect, scene, direction, map_size, count, far);
        if count > 1 {
            return view;
        }
        let whole = Self::fit_scene(scene, direction, map_size);
        if view.half_extent[0] < whole.half_extent[0] {
            view
        } else {
            whole
        }
    }

    /// `count` cascades splitting `camera`'s view up to `distance` (and the
    /// camera's far plane); `scene` bounds every shadow caster, so casters
    /// outside a slice (a tall wall behind the camera) still land in its
    /// map.
    pub fn fit_camera(
        camera: &Camera3D,
        aspect: f32,
        scene: Sphere,
        direction: Vec3,
        map_size: u32,
        count: usize,
        distance: f32,
    ) -> Self {
        let count = count.clamp(1, MAX_CASCADES);
        let near = camera.near.max(0.01);
        let far = distance.min(camera.far).max(near + 1.0);
        let splits = split_depths(near, far, count);

        let forward = (camera.target - camera.position)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        let tan_y = (camera.fov_y_degrees.to_radians() * 0.5).tan();
        let tan_x = tan_y * aspect.max(0.01);

        let mut out = Self::empty();
        let mut slice_near = near;
        for (i, &slice_far) in splits.iter().take(count).enumerate() {
            // The slice's bounding sphere, in closed form: the sphere through
            // both end rectangles' corners, centred on the view axis. Its
            // radius depends only on the slice, so it doesn't change as the
            // camera turns (a requirement for shimmer-free shadows).
            let k2 = tan_x * tan_x + tan_y * tan_y;
            let (n, f) = (slice_near, slice_far);
            let mut t = f32::midpoint(n, f) * (1.0 + k2);
            t = t.min(f); // a very wide slice: centre at the far plane
            let r_near = ((t - n).powi(2) + (n * n * k2)).sqrt();
            let r_far = ((f - t).powi(2) + (f * f * k2)).sqrt();
            let radius = r_near.max(r_far);
            let center = camera.position + forward * t;
            let (view, view_proj, depth) =
                cascade_matrices(center, radius, scene, direction, map_size);
            out.depth_range[i] = depth;
            out.view[i] = view;
            out.view_proj[i] = view_proj;
            out.half_extent[i] = radius;
            out.splits[i] = slice_far;
            slice_near = slice_far;
        }
        out.count = count;
        out
    }

    fn empty() -> Self {
        Self {
            view_proj: [Mat4::IDENTITY; MAX_CASCADES],
            view: [Mat4::IDENTITY; MAX_CASCADES],
            half_extent: [1.0; MAX_CASCADES],
            depth_range: [1.0; MAX_CASCADES],
            splits: [f32::MAX; MAX_CASCADES],
            count: 1,
        }
    }

    /// Whether a world-space sphere can cast into cascade `i` (its box is
    /// open toward the light, so only the sideways extent matters).
    pub fn touches(&self, i: usize, sphere: Sphere) -> bool {
        let p = (self.view[i] * sphere.center.extend(1.0)).xyz();
        let reach = self.half_extent[i] + sphere.radius;
        p.x.abs() <= reach && p.y.abs() <= reach
    }
}

/// `x` rounded up to the next of the steps 2^(k/4) (each ~19% above the
/// last).
fn quantize_up(x: f32) -> f32 {
    let x = x.max(1.0);
    ((x.log2() * 4.0).ceil() * 0.25).exp2()
}

/// The far view depth of each of `count` slices between `near` and `far`
/// ("practical" split scheme: a blend of logarithmic and uniform).
pub(crate) fn split_depths(near: f32, far: f32, count: usize) -> [f32; MAX_CASCADES] {
    let mut out = [far; MAX_CASCADES];
    let count = count.clamp(1, MAX_CASCADES);
    for (i, slot) in out.iter_mut().enumerate().take(count) {
        let p = (i + 1) as f32 / count as f32;
        let log = near * (far / near).powf(p);
        let uniform = near + (far - near) * p;
        *slot = SPLIT_LAMBDA * log + (1.0 - SPLIT_LAMBDA) * uniform;
    }
    out[count - 1] = far;
    out
}

/// A light view and orthographic projection for a box of half-extent
/// `radius` around `center`, deep enough to contain the whole `scene`, with
/// the box snapped to the shadow map's texel grid. Returns the view, the
/// view-projection and the box's depth extent.
fn cascade_matrices(
    center: Vec3,
    radius: f32,
    scene: Sphere,
    direction: Vec3,
    map_size: u32,
) -> (Mat4, Mat4, f32) {
    use glam::camera::rh::{proj::directx, view};

    let direction = direction.try_normalize().unwrap_or(Vec3::NEG_Y);
    let up = if direction.y.abs() > 0.95 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    // Rotation-only light view; translate afterwards so the snap happens in
    // light space.
    let rotation = view::look_at_mat4(Vec3::ZERO, direction, up);
    let mut c = (rotation * center.extend(1.0)).xyz();
    let texel = (2.0 * radius) / map_size.max(1) as f32;
    c.x = (c.x / texel).round() * texel;
    c.y = (c.y / texel).round() * texel;
    let light_view = Mat4::from_translation(-c) * rotation;

    // Depth range: the cascade's own sphere plus every caster in the scene.
    // View space looks down -Z, so "in front" is negative z.
    let scene_z = (light_view * scene.center.extend(1.0)).z;
    let z_max = (scene_z + scene.radius).max(radius) + 1.0;
    let z_min = (scene_z - scene.radius).min(-radius) - 1.0;
    let proj = directx::orthographic(-radius, radius, -radius, radius, -z_max, -z_min);
    (light_view, proj * light_view, z_max - z_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera3D {
        Camera3D {
            position: Vec3::new(0.0, 2.0, 10.0),
            target: Vec3::new(0.0, 2.0, 0.0),
            up: Vec3::Y,
            fov_y_degrees: 60.0,
            near: 0.1,
            far: 1000.0,
            ortho: 0.0,
        }
    }

    const SCENE: Sphere = Sphere {
        center: Vec3::ZERO,
        radius: 200.0,
    };

    fn sun() -> Vec3 {
        Vec3::new(-0.5, -1.0, -0.35)
    }

    #[test]
    fn splits_grow_and_end_at_far() {
        let s = split_depths(0.1, 120.0, 4);
        assert!(s[0] > 0.1 && s[0] < s[1] && s[1] < s[2] && s[2] < s[3]);
        assert!((s[3] - 120.0).abs() < 1e-4);
        // Mostly logarithmic: the first slice is short.
        assert!(s[0] < 15.0, "{s:?}");
        assert_eq!(split_depths(0.1, 50.0, 1)[0], 50.0);
    }

    #[test]
    fn every_slice_point_lands_inside_its_cascade() {
        let cam = camera();
        let c = ShadowCascades::fit_camera(&cam, 16.0 / 9.0, SCENE, sun(), 2048, 4, 120.0);
        assert_eq!(c.count, 4);
        let forward = (cam.target - cam.position).normalize();
        let tan_y = 30.0_f32.to_radians().tan();
        let tan_x = tan_y * 16.0 / 9.0;
        let mut near = cam.near;
        for i in 0..4 {
            for depth in [near, c.splits[i]] {
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let p = cam.position
                        + forward * depth
                        + Vec3::X * sx * tan_x * depth
                        + Vec3::Y * sy * tan_y * depth;
                    let ndc = c.view_proj[i].project_point3(p);
                    assert!(
                        ndc.x.abs() <= 1.0001 && ndc.y.abs() <= 1.0001,
                        "cascade {i}: {ndc:?}"
                    );
                    assert!((0.0..=1.0).contains(&ndc.z), "depth {i}: {ndc:?}");
                }
            }
            near = c.splits[i];
        }
        // Nearer cascades are smaller (more texels per metre).
        assert!(c.half_extent[0] < c.half_extent[1] && c.half_extent[2] < c.half_extent[3]);
    }

    #[test]
    fn scene_casters_behind_a_slice_stay_in_depth_range() {
        let cam = camera();
        let c = ShadowCascades::fit_camera(&cam, 1.5, SCENE, sun(), 2048, 4, 120.0);
        // A caster up the light ray from the camera, far outside slice 0.
        let caster = cam.position - sun().normalize() * 150.0;
        let ndc = c.view_proj[0].project_point3(caster);
        assert!((0.0..=1.0).contains(&ndc.z), "{ndc:?}");
    }

    #[test]
    fn cascades_snap_to_texels_so_small_moves_dont_shimmer() {
        let mut cam = camera();
        let a = ShadowCascades::fit_camera(&cam, 1.5, SCENE, sun(), 2048, 4, 120.0);
        cam.position.x += 0.0001;
        cam.target.x += 0.0001;
        let b = ShadowCascades::fit_camera(&cam, 1.5, SCENE, sun(), 2048, 4, 120.0);
        assert_eq!(
            a.view_proj[3], b.view_proj[3],
            "a sub-texel move keeps the matrix"
        );
        // Turning in place keeps the size (the sphere is rotation-invariant).
        cam.target = cam.position + Vec3::new(1.0, 0.0, -1.0);
        let turned = ShadowCascades::fit_camera(&cam, 1.5, SCENE, sun(), 2048, 4, 120.0);
        assert!((turned.half_extent[1] - a.half_extent[1]).abs() < 1e-4);
    }

    #[test]
    fn culling_keeps_nearby_meshes_and_drops_far_ones() {
        let cam = camera();
        let c = ShadowCascades::fit_camera(&cam, 1.5, SCENE, sun(), 2048, 4, 120.0);
        let near_mesh = Sphere {
            center: cam.position + Vec3::new(0.0, -2.0, -2.0),
            radius: 0.5,
        };
        let far_mesh = Sphere {
            center: cam.position + Vec3::new(80.0, 0.0, -60.0),
            radius: 1.0,
        };
        assert!(c.touches(0, near_mesh));
        assert!(!c.touches(0, far_mesh));
        assert!(c.touches(3, far_mesh));
    }

    #[test]
    fn single_cascade_covers_the_scene() {
        let scene = Sphere {
            center: Vec3::new(5.0, 0.0, 5.0),
            radius: 10.0,
        };
        let c = ShadowCascades::fit_scene(scene, sun(), 2048);
        assert_eq!(c.count, 1);
        for p in [Vec3::new(14.0, 0.0, 5.0), Vec3::new(5.0, 9.0, -4.0)] {
            let ndc = c.view_proj[0].project_point3(p);
            assert!(ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0, "{ndc:?}");
            assert!((0.0..=1.0).contains(&ndc.z), "{ndc:?}");
        }
    }

    #[test]
    fn small_scenes_pull_the_cascades_in_and_big_ones_stop_at_the_distance() {
        let small = Sphere {
            center: Vec3::ZERO,
            radius: 15.0,
        };
        let cam = camera();
        let near = ShadowCascades::fit(&cam, 1.5, small, sun(), 2048, 2, 120.0);
        let far = ShadowCascades::fit(&cam, 1.5, SCENE, sun(), 2048, 2, 120.0);
        // The small scene ends ~25 m away: its slices stop soon after.
        assert!(
            near.splits[1] >= 25.0 && near.splits[1] < 32.0,
            "{:?}",
            near.splits
        );
        assert!(near.half_extent[0] < far.half_extent[0]);
        assert!((far.splits[1] - 120.0).abs() < 1e-3, "{:?}", far.splits);
        // A shorter shadow distance is sharper.
        let short = ShadowCascades::fit(&cam, 1.5, SCENE, sun(), 2048, 2, 40.0);
        assert!(short.half_extent[1] < far.half_extent[1]);
        // Walking a little doesn't resize the slices (no shimmer).
        let mut stepped = cam;
        stepped.position.x += 0.3;
        stepped.target.x += 0.3;
        let moved = ShadowCascades::fit(&stepped, 1.5, small, sun(), 2048, 2, 120.0);
        assert_eq!(moved.half_extent, near.half_extent);
    }

    #[test]
    fn one_cascade_takes_the_smaller_of_scene_and_view() {
        let cam = camera();
        let small = Sphere {
            center: Vec3::ZERO,
            radius: 10.0,
        };
        let c = ShadowCascades::fit(&cam, 1.5, small, sun(), 1024, 1, 120.0);
        assert!((c.half_extent[0] - 11.0).abs() < 1e-3, "the scene map");
        assert_eq!(c.splits[0], f32::MAX, "which never fades");
        let huge = Sphere {
            center: Vec3::ZERO,
            radius: 2000.0,
        };
        let c = ShadowCascades::fit(&cam, 1.5, huge, sun(), 1024, 1, 60.0);
        assert!(c.half_extent[0] < 100.0, "the view: {}", c.half_extent[0]);
        assert!((c.splits[0] - 60.0).abs() < 1e-3);
    }

    #[test]
    fn distances_round_up_in_small_steps() {
        assert!((quantize_up(16.0) - 16.0).abs() < 1e-3);
        let q = quantize_up(17.0);
        assert!((17.0..17.0 * 1.2).contains(&q), "{q}");
        assert_eq!(quantize_up(17.0), quantize_up(18.0));
    }
}
