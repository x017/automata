//! The viewport renderer: the engine's real GPU output for the scene.
//!
//! Oberon builds a live ECS world from the document, materializes its mesh
//! visuals, records a [`DrawList`], and renders it into an offscreen texture
//! **on the window's own GPU device** (the same [`Renderer`] that presents
//! the editor), then reads the pixels back to composite into the UI. When no
//! renderer is available (headless CI), the viewport falls back to a
//! software wireframe — driven by the same orbit camera — so the editor still
//! shows the scene.

#![forbid(unsafe_code)]

use automata_renderer::{DrawList, RenderTarget, Renderer};
use automata_scene::Scene;

use crate::doc::Document;
use crate::ui::ViewCamera;

/// Renders the document's scene to an RGBA8 pixel buffer of `width`×`height`.
///
/// All GPU work goes through a caller-provided [`Renderer`] (the window's), so
/// the editor runs on exactly one device — the real GPU. Without one, it
/// falls back to a wireframe projection so the viewport is never blank.
pub struct ViewportRenderer {
    scene: Scene,
    /// Reused offscreen render target and its size (recreated on resize).
    target: Option<(RenderTarget, (u32, u32))>,
    /// Reused output buffer (overwritten each frame, never grown past size).
    frame: Vec<u8>,
    /// The document change counter the GPU world was last built from. When it
    /// matches the document's current counter, the world is reused as-is.
    built_counter: Option<u64>,
}

impl Default for ViewportRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportRenderer {
    /// Creates a viewport renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scene: Scene::new(),
            target: None,
            frame: Vec::new(),
            built_counter: None,
        }
    }

    /// Forces the next render to rebuild the GPU world (call when the whole
    /// document is replaced, since a fresh document restarts its counter).
    pub fn invalidate(&mut self) {
        self.built_counter = None;
    }

    /// Renders the document into the internal RGBA8 buffer and returns it.
    ///
    /// `renderer` is the GPU renderer to draw with (the window's); `None`
    /// selects the software wireframe. `cam` is the editor's orbit camera and
    /// drives both paths, so dragging in the viewport moves the view.
    pub fn render(
        &mut self,
        renderer: Option<&mut Renderer>,
        doc: &Document,
        cam: ViewCamera,
        width: u32,
        height: u32,
    ) -> (&[u8], u32, u32) {
        let width = width.max(1);
        let height = height.max(1);
        match renderer {
            Some(renderer) => {
                // Reuse the render target across frames (recreate on resize).
                if self.target.as_ref().map(|(_, size)| *size) != Some((width, height)) {
                    let target = renderer.create_render_target((width, height));
                    self.target = Some((target, (width, height)));
                }
                let (target, _) = self.target.as_ref().expect("target just ensured");

                // Rebuild the live world ONLY when the document changed.
                // Rebuilding every frame respawned the ECS world and
                // re-uploaded every mesh/material each frame.
                let counter = doc.change_counter();
                if self.built_counter != Some(counter) {
                    let file = doc.to_scene_file();
                    file.apply_to_world(self.scene.world_mut());
                    automata_scene::materialize_visuals(self.scene.world_mut(), renderer);
                    self.built_counter = Some(counter);
                }
                let mut draw = DrawList::new();
                draw.set_clear_color(wgpu_color(0.05, 0.055, 0.07, 1.0));
                automata_scene::record_draw_list(self.scene.world(), &mut draw);
                // The editor's orbit camera overrides any scene camera so the
                // viewport is freely navigable regardless of scene contents.
                draw.set_camera_3d(cam.to_camera3d());
                renderer.render_offscreen(&draw, target);
                renderer.read_texture_rgba_into(target, (width, height), &mut self.frame);
            }
            None => {
                render_wireframe::render_into(doc, cam, width, height, &mut self.frame);
            }
        }
        (self.frame.as_slice(), width, height)
    }
}

fn wgpu_color(r: f64, g: f64, b: f64, a: f64) -> automata_renderer::Color {
    automata_renderer::Color { r, g, b, a }
}

/// A CPU wireframe fallback used when no GPU adapter is present.
pub mod render_wireframe {
    use crate::doc::Document;
    use crate::ui::ViewCamera;

    /// Renders the document's meshes as wireframes onto an RGBA buffer,
    /// viewed from the editor orbit camera `view`.
    #[must_use]
    pub fn render(doc: &Document, view: ViewCamera, width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::new();
        render_into(doc, view, width, height, &mut px);
        px
    }

    /// Like [`render`], but reuses `px` (resized to `width*height*4`).
    pub fn render_into(
        doc: &Document,
        view: ViewCamera,
        width: u32,
        height: u32,
        px: &mut Vec<u8>,
    ) {
        let (w, h) = (width as usize, height as usize);
        px.clear();
        px.resize(w * h * 4, 0);
        // Background.
        for chunk in px.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[13, 14, 18, 255]);
        }
        // Grid + a simple perspective projection of each cube.
        let cam = Camera {
            yaw: view.yaw,
            pitch: view.pitch,
            dist: view.distance,
        };
        draw_grid(px, w, h, &cam);
        for entity in &doc.entities {
            let Some(t) = &entity.data.transform else {
                continue;
            };
            let (cx, cy, cz) = (t.position[0], t.position[1], t.position[2]);
            let (sx, sy, sz) = (t.scale[0], t.scale[1], t.scale[2]);
            if entity.data.mesh_visual.is_some() {
                draw_cube(
                    px,
                    w,
                    h,
                    &cam,
                    (cx, cy, cz),
                    (sx, sy, sz),
                    (232, 234, 240, 255),
                );
            } else {
                draw_marker(px, w, h, &cam, (cx, cy, cz));
            }
        }
    }

    #[derive(Clone, Copy)]
    struct Camera {
        yaw: f32,
        pitch: f32,
        dist: f32,
    }

    impl Camera {
        fn project(&self, p: (f32, f32, f32), w: usize, h: usize) -> Option<(f32, f32)> {
            let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
            let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
            // Orbit the eye around the origin.
            let eye = (self.dist * cp * sy, self.dist * sp, self.dist * cp * cy);
            // View space: translate, then yaw then pitch.
            let (px, py, pz) = (p.0 - eye.0, p.1 - eye.1, p.2 - eye.2);
            let (x1, z1) = (px * cy - pz * sy, px * sy + pz * cy);
            let (y2, z2) = (py * cp - z1 * sp, py * sp + z1 * cp);
            let depth = -z2;
            if depth < 0.1 {
                return None;
            }
            let f = (h as f32) * 0.9;
            Some((
                w as f32 / 2.0 + x1 * f / depth,
                h as f32 / 2.0 - y2 * f / depth,
            ))
        }
    }

    fn set(px: &mut [u8], w: usize, h: usize, x: i32, y: i32, c: (u8, u8, u8, u8)) {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            let i = ((y as usize) * w + x as usize) * 4;
            px[i] = c.0;
            px[i + 1] = c.1;
            px[i + 2] = c.2;
            px[i + 3] = c.3;
        }
    }

    fn line(px: &mut [u8], w: usize, h: usize, a: (f32, f32), b: (f32, f32), c: (u8, u8, u8, u8)) {
        let (mut x0, mut y0) = (a.0 as i32, a.1 as i32);
        let (x1, y1) = (b.0 as i32, b.1 as i32);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            set(px, w, h, x0, y0, c);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn edge(
        px: &mut [u8],
        w: usize,
        h: usize,
        cam: &Camera,
        a: (f32, f32, f32),
        b: (f32, f32, f32),
        c: (u8, u8, u8, u8),
    ) {
        if let (Some(pa), Some(pb)) = (cam.project(a, w, h), cam.project(b, w, h)) {
            line(px, w, h, pa, pb, c);
        }
    }

    fn draw_grid(px: &mut [u8], w: usize, h: usize, cam: &Camera) {
        let half = 10;
        for i in -half..=half {
            let i = i as f32;
            let (ca, cb) = if i == 0.0 {
                ((224, 96, 100, 255), (96, 148, 224, 255))
            } else {
                ((44, 48, 58, 255), (44, 48, 58, 255))
            };
            edge(
                px,
                w,
                h,
                cam,
                (i, 0.0, -half as f32),
                (i, 0.0, half as f32),
                ca,
            );
            edge(
                px,
                w,
                h,
                cam,
                (-half as f32, 0.0, i),
                (half as f32, 0.0, i),
                cb,
            );
        }
    }

    fn draw_cube(
        px: &mut [u8],
        w: usize,
        h: usize,
        cam: &Camera,
        c: (f32, f32, f32),
        s: (f32, f32, f32),
        col: (u8, u8, u8, u8),
    ) {
        let (hx, hy, hz) = (s.0 / 2.0, s.1 / 2.0, s.2 / 2.0);
        let v = [
            (-hx, -hy, -hz),
            (hx, -hy, -hz),
            (hx, hy, -hz),
            (-hx, hy, -hz),
            (-hx, -hy, hz),
            (hx, -hy, hz),
            (hx, hy, hz),
            (-hx, hy, hz),
        ];
        let world: Vec<(f32, f32, f32)> = v
            .iter()
            .map(|(x, y, z)| (c.0 + x, c.1 + y, c.2 + z))
            .collect();
        let idx = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];
        for (a, b) in idx {
            edge(px, w, h, cam, world[a], world[b], col);
        }
    }

    fn draw_marker(px: &mut [u8], w: usize, h: usize, cam: &Camera, c: (f32, f32, f32)) {
        let m = 0.3;
        let col = (150, 156, 170, 255);
        edge(px, w, h, cam, (c.0 - m, c.1, c.2), (c.0 + m, c.1, c.2), col);
        edge(px, w, h, cam, (c.0, c.1 - m, c.2), (c.0, c.1 + m, c.2), col);
        edge(px, w, h, cam, (c.0, c.1, c.2 - m), (c.0, c.1, c.2 + m), col);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_viewport_follows_the_orbit_camera() {
        let doc = Document::starter();
        let mut vp = ViewportRenderer::new();
        let a = vp.render(None, &doc, ViewCamera::default(), 320, 200).0.to_vec();
        let turned = ViewCamera {
            yaw: 2.0,
            ..ViewCamera::default()
        };
        let b = vp.render(None, &doc, turned, 320, 200).0.to_vec();
        assert_eq!(a.len(), 320 * 200 * 4);
        assert_ne!(a, b, "orbiting must change the rendered view");
    }
}
