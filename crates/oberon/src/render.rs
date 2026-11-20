//! The viewport renderer: the engine's real GPU output for the scene.
//!
//! Oberon builds a live ECS world from the document, materializes its mesh
//! visuals, records a [`DrawList`], and renders it offscreen with the
//! engine's wgpu renderer, then reads the pixels back to composite into the
//! editor UI. When no GPU adapter is available (headless CI), the viewport
//! falls back to a software wireframe so the editor still shows the scene.

use automata_renderer::{DrawList, Renderer};
use automata_scene::Scene;

use crate::doc::Document;

/// Renders the document's scene to an RGBA8 pixel buffer of `width`×`height`.
///
/// Uses the engine's offscreen GPU renderer when available; otherwise (or on
/// render failure) falls back to a wireframe projection so the viewport is
/// never blank.
pub struct ViewportRenderer {
    renderer: Option<Renderer>,
    scene: Scene,
}

impl Default for ViewportRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportRenderer {
    /// Creates a viewport renderer. GPU init is deferred to first render.
    pub fn new() -> Self {
        Self {
            renderer: None,
            scene: Scene::new(),
        }
    }

    /// Renders the document into an RGBA8 buffer. Returns `None` only when
    /// even the fallback cannot run (never in practice).
    pub fn render(&mut self, doc: &Document, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
        let width = width.max(1);
        let height = height.max(1);
        // GPU path (lazy-init; the editor may start before a GPU is ready).
        if self.renderer.is_none() {
            self.renderer = pollster::block_on(Renderer::new_offscreen((width, height))).ok();
        }
        let mut frame = None;
        if let Some(renderer) = &mut self.renderer {
            if renderer.size() != (width, height) {
                renderer.resize(width, height);
            }
            // Build the live world from the document.
            let file = doc.to_scene_file();
            file.apply_to_world(self.scene.world_mut());
            automata_scene::materialize_visuals(self.scene.world_mut(), renderer);
            let mut draw = DrawList::new();
            draw.set_clear_color(wgpu_color(0.05, 0.055, 0.07, 1.0));
            automata_scene::record_draw_list(self.scene.world(), &mut draw);
            let target = renderer.create_render_target((width, height));
            renderer.render_offscreen(&draw, &target);
            frame = Some(renderer.read_texture_rgba(&target, (width, height)));
        }
        match frame {
            Some(f) => (f, width, height),
            // Software fallback: a wireframe projection of the scene.
            None => (render_wireframe::render(doc, width, height), width, height),
        }
    }
}

fn wgpu_color(r: f64, g: f64, b: f64, a: f64) -> automata_renderer::Color {
    automata_renderer::Color { r, g, b, a }
}

/// A CPU wireframe fallback used when no GPU adapter is present.
pub mod render_wireframe {
    use crate::doc::Document;

    /// Renders the document's meshes as wireframes onto an RGBA buffer.
    pub fn render(doc: &Document, width: u32, height: u32) -> Vec<u8> {
        let (w, h) = (width as usize, height as usize);
        let mut px = vec![0u8; w * h * 4];
        // Background.
        for chunk in px.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[13, 14, 18, 255]);
        }
        // Grid + a simple perspective projection of each cube.
        let cam = Camera::default();
        draw_grid(&mut px, w, h, &cam);
        for entity in &doc.entities {
            let Some(t) = &entity.data.transform else {
                continue;
            };
            let (cx, cy, cz) = (t.position[0], t.position[1], t.position[2]);
            let (sx, sy, sz) = (t.scale[0], t.scale[1], t.scale[2]);
            if entity.data.mesh_visual.is_some() {
                draw_cube(
                    &mut px,
                    w,
                    h,
                    &cam,
                    (cx, cy, cz),
                    (sx, sy, sz),
                    (232, 234, 240, 255),
                );
            } else {
                draw_marker(&mut px, w, h, &cam, (cx, cy, cz));
            }
        }
        px
    }

    #[derive(Clone, Copy)]
    struct Camera {
        yaw: f32,
        pitch: f32,
        dist: f32,
    }

    impl Default for Camera {
        fn default() -> Self {
            Self {
                yaw: 0.6,
                pitch: 0.5,
                dist: 10.0,
            }
        }
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
