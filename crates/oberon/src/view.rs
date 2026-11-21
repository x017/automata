//! Scene-view math: the editor camera, world↔screen projection, mouse
//! picking, and the transform gizmo (move / rotate / scale).
//!
//! Everything here is pure math over the document — no drawing, no GPU — so
//! it is unit-tested headless. The UI draws the overlays from these results
//! and turns gizmo drags into transform edits.

#![forbid(unsafe_code)]

use automata_core::{EntityData, PrimitiveData, TransformData};
use glam::{EulerRot, Mat4, Quat, Vec3, Vec4};

use crate::doc::Document;
use crate::ui::Rect;

/// The editor's scene-view camera: an orbit around a pivot point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewCamera {
    /// Orbit yaw in radians.
    pub yaw: f32,
    /// Orbit pitch in radians.
    pub pitch: f32,
    /// Distance from the pivot.
    pub distance: f32,
    /// The pivot the camera orbits and looks at.
    pub target: [f32; 3],
}

impl Default for ViewCamera {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.45,
            distance: 11.0,
            target: [0.0, 0.5, 0.0],
        }
    }
}

impl ViewCamera {
    /// The camera's world-space eye position.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        Vec3::from(self.target)
            + Vec3::new(
                self.distance * cp * sy,
                self.distance * sp,
                self.distance * cp * cy,
            )
    }

    /// Builds the engine [`automata_renderer::Camera3D`] for this view.
    #[must_use]
    pub fn to_camera3d(&self) -> automata_renderer::Camera3D {
        let mut cam = automata_renderer::Camera3D::new(self.eye(), Vec3::from(self.target));
        // Keep near/far proportional to the zoom so close-ups don't clip and
        // far views keep depth precision.
        cam.near = (self.distance * 0.01).clamp(0.01, 1.0);
        cam.far = (self.distance * 200.0).max(500.0);
        cam
    }

    /// The orbit view that sees what `cam` sees (pivot at its look-at
    /// point).
    #[must_use]
    pub fn from_camera3d(cam: &automata_renderer::Camera3D) -> Self {
        let offset = cam.position - cam.target;
        let distance = offset.length().max(0.05);
        let pitch = (offset.y / distance)
            .clamp(-1.0, 1.0)
            .asin()
            .clamp(-1.54, 1.54);
        let yaw = if offset.x.abs() + offset.z.abs() < 1e-6 {
            0.0
        } else {
            offset.x.atan2(offset.z)
        };
        Self {
            yaw,
            pitch,
            distance,
            target: cam.target.to_array(),
        }
    }

    /// Orbits by a mouse delta in pixels.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.54, 1.54);
    }

    /// Pans the pivot by a mouse delta in pixels (screen-space drag).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let eye = self.eye();
        let forward = (Vec3::from(self.target) - eye).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let k = self.distance * 0.0016;
        let t = Vec3::from(self.target) + (-dx * right + dy * up) * k;
        self.target = t.to_array();
    }

    /// Zooms by scroll-wheel notches (positive = in).
    pub fn zoom(&mut self, notches: f32) {
        self.distance = (self.distance * 0.88_f32.powf(notches)).clamp(0.3, 2_000.0);
    }

    /// Frames a point with a given radius (the "F" focus shortcut).
    pub fn focus(&mut self, center: Vec3, radius: f32) {
        self.target = center.to_array();
        self.distance = (radius * 3.0).clamp(1.5, 500.0);
    }
}

/// Maps between world space and a viewport rectangle on screen.
#[derive(Debug, Clone, Copy)]
pub struct Projector {
    view_proj: Mat4,
    inv_view_proj: Mat4,
    rect: Rect,
    eye: Vec3,
}

impl Projector {
    /// A projector for `cam` drawing into `rect` (logical pixels).
    #[must_use]
    pub fn new(cam: &ViewCamera, rect: Rect) -> Self {
        let size = (rect.w.max(1.0) as u32, rect.h.max(1.0) as u32);
        let view_proj = cam.to_camera3d().view_proj(size);
        Self {
            view_proj,
            inv_view_proj: view_proj.inverse(),
            rect,
            eye: cam.eye(),
        }
    }

    /// The camera eye position.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        self.eye
    }

    /// Projects a world point to screen coordinates, or `None` when it is
    /// behind the camera.
    #[must_use]
    pub fn project(&self, p: Vec3) -> Option<(f32, f32)> {
        let clip = self.view_proj * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some((
            self.rect.x + (ndc.x * 0.5 + 0.5) * self.rect.w,
            self.rect.y + (0.5 - ndc.y * 0.5) * self.rect.h,
        ))
    }

    /// A world-space ray `(origin, direction)` through screen point `(sx, sy)`.
    #[must_use]
    pub fn ray(&self, sx: f32, sy: f32) -> (Vec3, Vec3) {
        let nx = (sx - self.rect.x) / self.rect.w.max(1.0) * 2.0 - 1.0;
        let ny = 1.0 - (sy - self.rect.y) / self.rect.h.max(1.0) * 2.0;
        let unproject = |z: f32| {
            let v = self.inv_view_proj * Vec4::new(nx, ny, z, 1.0);
            v.truncate() / v.w
        };
        // Depth range is 0 (near) .. 1 (far).
        let near = unproject(0.0);
        let far = unproject(1.0);
        (near, (far - near).normalize_or_zero())
    }

    /// World units per screen pixel at the depth of `p` (gizmo sizing).
    #[must_use]
    pub fn world_per_pixel(&self, p: Vec3) -> f32 {
        let Some(a) = self.project(p) else {
            return 0.01;
        };
        let right = {
            let forward = (p - self.eye).normalize_or_zero();
            forward.cross(Vec3::Y).normalize_or_zero()
        };
        let right = if right == Vec3::ZERO { Vec3::X } else { right };
        match self.project(p + right) {
            Some(b) => {
                let px = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                if px > 1e-3 {
                    1.0 / px
                } else {
                    0.01
                }
            }
            None => 0.01,
        }
    }
}

/// The local-space half extents of an entity's pickable volume (before its
/// transform scale), or a small marker box for entities without a mesh.
#[must_use]
pub fn local_half_extents(data: &EntityData) -> Vec3 {
    match data.mesh_visual.as_ref().map(|m| &m.primitive) {
        Some(PrimitiveData::Cube { size }) => Vec3::splat(size * 0.5),
        Some(PrimitiveData::Quad { size }) => Vec3::new(size * 0.5, size * 0.5, 0.01),
        Some(PrimitiveData::Custom { mesh }) => {
            let mut m = Vec3::splat(0.05);
            for v in &mesh.vertices {
                m = m.max(Vec3::from(v.position).abs());
            }
            m
        }
        None => Vec3::splat(0.3),
    }
}

/// An entity's model matrix from its transform data.
#[must_use]
pub fn model_matrix(t: &TransformData) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::from(t.scale),
        rotation_of(t),
        Vec3::from(t.position),
    )
}

/// The rotation quaternion of a transform.
#[must_use]
pub fn rotation_of(t: &TransformData) -> Quat {
    Quat::from_euler(
        EulerRot::XYZ,
        t.rotation_euler[0],
        t.rotation_euler[1],
        t.rotation_euler[2],
    )
}

/// The world-space corners of an entity's bounding box (for the selection
/// outline). Index bits: x = bit 0, y = bit 1, z = bit 2.
#[must_use]
pub fn box_corners(data: &EntityData) -> Option<[Vec3; 8]> {
    let t = data.transform?;
    let model = model_matrix(&t);
    let h = local_half_extents(data);
    let mut out = [Vec3::ZERO; 8];
    for (i, corner) in out.iter_mut().enumerate() {
        let local = Vec3::new(
            if i & 1 == 0 { -h.x } else { h.x },
            if i & 2 == 0 { -h.y } else { h.y },
            if i & 4 == 0 { -h.z } else { h.z },
        );
        *corner = model.transform_point3(local);
    }
    Some(out)
}

/// Ray vs. oriented box (given by `model` and local `half` extents). Returns
/// the hit distance along the world ray.
#[must_use]
pub fn ray_box(origin: Vec3, dir: Vec3, model: Mat4, half: Vec3) -> Option<f32> {
    let det = model.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = model.inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;
    for a in 0..3 {
        let (oa, da, ha) = (o[a], d[a], half[a]);
        if da.abs() < 1e-9 {
            if oa < -ha || oa > ha {
                return None;
            }
        } else {
            let t1 = (-ha - oa) / da;
            let t2 = (ha - oa) / da;
            t_min = t_min.max(t1.min(t2));
            t_max = t_max.min(t1.max(t2));
            if t_min > t_max {
                return None;
            }
        }
    }
    if t_max < 0.0 {
        return None;
    }
    // `d` is the world direction mapped to local space, so the parameter `t`
    // is shared: the world hit point is origin + t * dir.
    Some(t_min.max(0.0))
}

/// Picks the nearest entity under screen point `(sx, sy)`.
#[must_use]
pub fn pick(doc: &Document, proj: &Projector, sx: f32, sy: f32) -> Option<u64> {
    let (origin, dir) = proj.ray(sx, sy);
    let mut best: Option<(f32, u64)> = None;
    for entity in &doc.entities {
        let Some(t) = entity.data.transform else {
            continue;
        };
        let (model, half) = if entity.data.mesh_visual.is_some() {
            (model_matrix(&t), local_half_extents(&entity.data))
        } else {
            // Mesh-less entities (lights, cameras, empties) get a fixed-size
            // marker box, unaffected by scale and rotation.
            (
                Mat4::from_translation(Vec3::from(t.position)),
                Vec3::splat(0.3),
            )
        };
        if let Some(dist) = ray_box(origin, dir, model, half) {
            // Prefer small markers over the big meshes behind/around them.
            let bias = if entity.data.mesh_visual.is_some() {
                0.0
            } else {
                -0.5
            };
            let score = dist + bias;
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, entity.id));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// The transform gizmo's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode {
    /// Translate along world axes.
    #[default]
    Move,
    /// Rotate about world axes.
    Rotate,
    /// Scale along local axes.
    Scale,
}

impl GizmoMode {
    /// Every mode, in toolbar order.
    pub const ALL: [Self; 3] = [Self::Move, Self::Rotate, Self::Scale];

    /// The display label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
        }
    }
}

/// Screen-space gizmo geometry for one entity in one frame.
#[derive(Debug, Clone)]
pub struct Gizmo {
    /// The mode it was built for.
    pub mode: GizmoMode,
    /// The pivot on screen.
    pub origin: (f32, f32),
    /// World-space pivot.
    pub origin_world: Vec3,
    /// World-space handle length.
    pub length: f32,
    /// World-space axis directions (world axes for move/rotate, local axes
    /// for scale).
    pub axes: [Vec3; 3],
    /// Screen positions of each axis handle's tip (move/scale).
    pub tips: [Option<(f32, f32)>; 3],
    /// Screen polylines of each rotation ring (rotate).
    pub rings: [Vec<(f32, f32)>; 3],
}

/// Number of samples per rotation ring.
const RING_SAMPLES: usize = 64;
/// Pixel distance within which a handle is grabbed.
pub const GRAB_RADIUS: f32 = 8.0;

impl Gizmo {
    /// Builds the gizmo for transform `t`, or `None` if the pivot is
    /// off-screen (behind the camera).
    #[must_use]
    pub fn build(mode: GizmoMode, t: &TransformData, proj: &Projector) -> Option<Self> {
        let origin_world = Vec3::from(t.position);
        let origin = proj.project(origin_world)?;
        // ~110 px long regardless of zoom.
        let length = proj.world_per_pixel(origin_world) * 110.0;
        let axes = match mode {
            GizmoMode::Scale => {
                let q = rotation_of(t);
                [q * Vec3::X, q * Vec3::Y, q * Vec3::Z]
            }
            _ => [Vec3::X, Vec3::Y, Vec3::Z],
        };
        let mut tips = [None; 3];
        let mut rings: [Vec<(f32, f32)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for a in 0..3 {
            match mode {
                GizmoMode::Move | GizmoMode::Scale => {
                    tips[a] = proj.project(origin_world + axes[a] * length);
                }
                GizmoMode::Rotate => {
                    let (u, v) = ring_basis(a);
                    let r = length * 0.85;
                    for i in 0..=RING_SAMPLES {
                        let th = i as f32 / RING_SAMPLES as f32 * std::f32::consts::TAU;
                        let p = origin_world + (u * th.cos() + v * th.sin()) * r;
                        if let Some(s) = proj.project(p) {
                            rings[a].push(s);
                        }
                    }
                }
            }
        }
        Some(Self {
            mode,
            origin,
            origin_world,
            length,
            axes,
            tips,
            rings,
        })
    }

    /// The axis handle under screen point `(x, y)`, if any.
    #[must_use]
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for a in 0..3 {
            let d = match self.mode {
                GizmoMode::Move | GizmoMode::Scale => match self.tips[a] {
                    Some(tip) => dist_to_segment((x, y), self.origin, tip),
                    None => f32::INFINITY,
                },
                GizmoMode::Rotate => self.rings[a]
                    .windows(2)
                    .map(|w| dist_to_segment((x, y), w[0], w[1]))
                    .fold(f32::INFINITY, f32::min),
            };
            if d < GRAB_RADIUS && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, a));
            }
        }
        best.map(|(_, a)| a)
    }

    /// Applies a mouse drag of `(dx, dy)` pixels at `(x, y)` on handle
    /// `axis` to transform `t`, returning the edited transform.
    #[must_use]
    pub fn drag(
        &self,
        axis: usize,
        t: &TransformData,
        (x, y): (f32, f32),
        (dx, dy): (f32, f32),
    ) -> TransformData {
        let mut out = *t;
        let axis = axis.min(2);
        match self.mode {
            GizmoMode::Move | GizmoMode::Scale => {
                let Some(tip) = self.tips[axis] else {
                    return out;
                };
                let (sx, sy) = (tip.0 - self.origin.0, tip.1 - self.origin.1);
                let len = (sx * sx + sy * sy).sqrt();
                if len < 2.0 {
                    return out; // axis points at the camera: no stable drag
                }
                // Fraction of the handle length dragged along its screen axis.
                let k = (dx * sx + dy * sy) / (len * len);
                if self.mode == GizmoMode::Move {
                    let delta = self.axes[axis] * (k * self.length);
                    for (i, p) in out.position.iter_mut().enumerate() {
                        *p += delta[i];
                    }
                } else {
                    let s = &mut out.scale[axis];
                    *s = (*s * (1.0 + k)).clamp(0.001, 100_000.0);
                }
            }
            GizmoMode::Rotate => {
                let ring = &self.rings[axis];
                if ring.len() < 3 {
                    return out;
                }
                // Tangent of the ring at the sample nearest the cursor.
                let (i, _) = ring
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i, (p.0 - x).powi(2) + (p.1 - y).powi(2)))
                    .fold(
                        (0, f32::INFINITY),
                        |acc, it| if it.1 < acc.1 { it } else { acc },
                    );
                let prev = ring[i.saturating_sub(1)];
                let next = ring[(i + 1).min(ring.len() - 1)];
                let (tx, ty) = (next.0 - prev.0, next.1 - prev.1);
                let tlen = (tx * tx + ty * ty).sqrt();
                let steps = ((i + 1).min(ring.len() - 1) - i.saturating_sub(1)) as f32;
                if tlen < 1e-3 || steps < 1.0 {
                    return out;
                }
                // Screen pixels per radian along the ring at this point.
                let px_per_rad = tlen / (steps * std::f32::consts::TAU / RING_SAMPLES as f32);
                let angle = (dx * tx + dy * ty) / tlen / px_per_rad.max(1.0);
                let q = Quat::from_axis_angle(self.axes[axis], angle) * rotation_of(t);
                let (ex, ey, ez) = q.to_euler(EulerRot::XYZ);
                out.rotation_euler = [ex, ey, ez];
            }
        }
        out
    }
}

/// A right-handed basis `(u, v)` spanning the plane perpendicular to world
/// axis `a`, with `u × v = axis`.
fn ring_basis(a: usize) -> (Vec3, Vec3) {
    match a {
        0 => (Vec3::Y, Vec3::Z),
        1 => (Vec3::Z, Vec3::X),
        _ => (Vec3::X, Vec3::Y),
    }
}

/// Distance from point `p` to segment `a`–`b` (screen space).
#[must_use]
pub fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 < 1e-6 {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + abx * t, a.1 + aby * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// The corners of a camera's view pyramid for drawing: the eye followed
/// by the four corners of a slice `depth` units ahead (top-left, top-right,
/// bottom-right, bottom-left), for an `aspect` = width / height view.
#[must_use]
pub fn camera_frustum(cam: &automata_renderer::Camera3D, aspect: f32, depth: f32) -> [Vec3; 5] {
    let forward = (cam.target - cam.position).normalize_or(Vec3::NEG_Z);
    let right = forward.cross(cam.up).normalize_or(Vec3::X);
    let up = right.cross(forward);
    let half_h = (cam.fov_y_degrees.to_radians() * 0.5).tan() * depth;
    let half_w = half_h * aspect;
    let center = cam.position + forward * depth;
    [
        cam.position,
        center - right * half_w + up * half_h,
        center + right * half_w + up * half_h,
        center + right * half_w - up * half_h,
        center - right * half_w - up * half_h,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_view_reproduces_a_scene_camera() {
        let cam =
            automata_renderer::Camera3D::new(Vec3::new(3.0, 4.0, 5.0), Vec3::new(0.0, 1.0, 0.0));
        let view = ViewCamera::from_camera3d(&cam);
        assert!(
            (view.eye() - cam.position).length() < 1e-4,
            "{:?}",
            view.eye()
        );
        let f = camera_frustum(&cam, 16.0 / 9.0, 1.0);
        assert_eq!(f[0], cam.position);
        let center = (f[1] + f[3]) * 0.5;
        assert!(((center - cam.position).length() - 1.0).abs() < 1e-4);
    }

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, 800.0, 600.0)
    }

    #[test]
    fn target_projects_to_viewport_center() {
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let (x, y) = proj.project(Vec3::from(cam.target)).unwrap();
        assert!(
            (x - 400.0).abs() < 0.5 && (y - 300.0).abs() < 0.5,
            "{x},{y}"
        );
    }

    #[test]
    fn center_ray_points_at_target() {
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let (o, d) = proj.ray(400.0, 300.0);
        let to_target = (Vec3::from(cam.target) - o).normalize();
        assert!(d.dot(to_target) > 0.9999);
    }

    #[test]
    fn orbit_moves_the_eye_but_keeps_distance() {
        let mut cam = ViewCamera::default();
        let before = cam.eye();
        cam.orbit(120.0, 0.0);
        assert!((cam.eye() - before).length() > 1.0);
        let d = (cam.eye() - Vec3::from(cam.target)).length();
        assert!((d - cam.distance).abs() < 1e-3);
    }

    #[test]
    fn pan_moves_the_pivot() {
        let mut cam = ViewCamera::default();
        let before = cam.target;
        cam.pan(50.0, 0.0);
        assert_ne!(before, cam.target);
    }

    #[test]
    fn clicking_the_cube_picks_it() {
        let doc = Document::starter();
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap();
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let (sx, sy) = proj.project(Vec3::new(0.0, 0.5, 0.0)).unwrap();
        assert_eq!(pick(&doc, &proj, sx, sy), Some(cube.id));
    }

    #[test]
    fn clicking_empty_sky_picks_nothing() {
        let doc = Document::starter();
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        // Top-left corner looks up into the sky, away from every entity.
        assert_eq!(pick(&doc, &proj, 2.0, 2.0), None);
    }

    #[test]
    fn ray_box_respects_rotation() {
        let model = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let hit = ray_box(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z, model, Vec3::splat(0.5));
        // A 45°-rotated unit cube's corner sits at z = +0.707.
        assert!((hit.unwrap() - (5.0 - 0.5 * 2f32.sqrt())).abs() < 1e-3);
    }

    #[test]
    fn move_gizmo_drag_moves_along_its_axis() {
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let t = TransformData::default();
        let g = Gizmo::build(GizmoMode::Move, &t, &proj).unwrap();
        let tip = g.tips[0].unwrap();
        let (dx, dy) = (tip.0 - g.origin.0, tip.1 - g.origin.1);
        // Drag the full length of the X handle → move by one handle length.
        let out = g.drag(0, &t, tip, (dx, dy));
        assert!((out.position[0] - g.length).abs() < 1e-3);
        assert!(out.position[1].abs() < 1e-6 && out.position[2].abs() < 1e-6);
    }

    #[test]
    fn gizmo_hit_finds_the_handle() {
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let t = TransformData::default();
        let g = Gizmo::build(GizmoMode::Move, &t, &proj).unwrap();
        let tip = g.tips[1].unwrap();
        let mid = (
            f32::midpoint(g.origin.0, tip.0),
            f32::midpoint(g.origin.1, tip.1),
        );
        assert_eq!(g.hit(mid.0, mid.1), Some(1));
    }

    #[test]
    fn rotate_gizmo_changes_rotation() {
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let t = TransformData::default();
        let g = Gizmo::build(GizmoMode::Rotate, &t, &proj).unwrap();
        let p = g.rings[1][RING_SAMPLES / 8];
        let out = g.drag(1, &t, p, (30.0, 0.0));
        assert_ne!(out.rotation_euler, t.rotation_euler);
    }

    #[test]
    fn scale_gizmo_grows_scale() {
        let cam = ViewCamera::default();
        let proj = Projector::new(&cam, rect());
        let t = TransformData::default();
        let g = Gizmo::build(GizmoMode::Scale, &t, &proj).unwrap();
        let tip = g.tips[0].unwrap();
        let (dx, dy) = (tip.0 - g.origin.0, tip.1 - g.origin.1);
        let out = g.drag(0, &t, tip, (dx * 0.5, dy * 0.5));
        assert!((out.scale[0] - 1.5).abs() < 1e-3);
    }
}
