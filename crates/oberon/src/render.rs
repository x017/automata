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
use automata_scene::{Scene, SceneWorld};

use crate::doc::Document;
use crate::play::PlaySession;
use crate::view::ViewCamera;

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
    /// Meshes drawn by the last scene-view render.
    meshes: usize,
    /// The camera preview's render target and pixels.
    preview_target: Option<(RenderTarget, (u32, u32))>,
    preview_frame: Vec<u8>,
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
            meshes: 0,
            preview_target: None,
            preview_frame: Vec::new(),
        }
    }

    /// Meshes drawn by the last scene-view render (0 for the wireframe).
    #[must_use]
    pub fn mesh_count(&self) -> usize {
        self.meshes
    }

    /// Renders the edit-time world (as built by the last [`Self::sync`])
    /// through `camera` into a separate `width`×`height` buffer: the
    /// selected camera's preview. Returns `None` without a GPU renderer.
    pub fn render_camera(
        &mut self,
        renderer: Option<&mut Renderer>,
        camera: automata_renderer::Camera3D,
        width: u32,
        height: u32,
    ) -> Option<(&[u8], u32, u32)> {
        let renderer = renderer?;
        self.built_counter?;
        let size = (width.max(1), height.max(1));
        Self::draw_world(
            renderer,
            &mut self.preview_target,
            &mut self.preview_frame,
            self.scene.world(),
            Some(camera),
            size,
        );
        Some((self.preview_frame.as_slice(), size.0, size.1))
    }

    /// Rebuilds the GPU world from the document if it changed since the
    /// last build (called by [`Self::render`]; call it first when rendering
    /// a camera preview before the main view).
    pub fn sync(&mut self, renderer: &mut Renderer, doc: &Document) {
        let counter = doc.change_counter();
        if self.built_counter != Some(counter) {
            let file = doc.to_scene_file();
            file.apply_to_world(self.scene.world_mut());
            automata_scene::materialize_visuals(self.scene.world_mut(), renderer);
            self.built_counter = Some(counter);
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
                // Rebuild the live world ONLY when the document changed.
                // Rebuilding every frame respawned the ECS world and
                // re-uploaded every mesh/material each frame.
                self.sync(renderer, doc);
                self.meshes = Self::draw_world(
                    renderer,
                    &mut self.target,
                    &mut self.frame,
                    self.scene.world(),
                    Some(cam.to_camera3d()),
                    (width, height),
                );
            }
            None => {
                self.meshes = 0;
                render_wireframe::render_into(doc, cam, width, height, &mut self.frame);
            }
        }
        (self.frame.as_slice(), width, height)
    }

    /// Renders a running play-mode simulation (its own live ECS world).
    ///
    /// When the scene has a camera, the frame is rendered through it (what
    /// the game will show); otherwise through the editor camera `cam`.
    /// Without a GPU renderer, the session's display document is drawn as a
    /// wireframe instead.
    pub fn render_play(
        &mut self,
        renderer: Option<&mut Renderer>,
        session: &mut PlaySession,
        cam: ViewCamera,
        width: u32,
        height: u32,
    ) -> (&[u8], u32, u32) {
        let width = width.max(1);
        let height = height.max(1);
        match renderer {
            Some(renderer) => {
                let editor_cam = session.scene_camera().is_none().then(|| cam.to_camera3d());
                let world = session.scene_mut().world_mut();
                // Uploads only visuals not yet materialized (first frame).
                automata_scene::materialize_visuals(world, renderer);
                self.meshes = Self::draw_world(
                    renderer,
                    &mut self.target,
                    &mut self.frame,
                    world,
                    editor_cam,
                    (width, height),
                );
            }
            None => {
                render_wireframe::render_into(&session.doc, cam, width, height, &mut self.frame);
            }
        }
        (self.frame.as_slice(), width, height)
    }

    /// Records `world`, renders it offscreen and reads the pixels back.
    /// `camera` overrides the scene's own camera when given. Returns the
    /// number of meshes drawn.
    fn draw_world(
        renderer: &mut Renderer,
        target: &mut Option<(RenderTarget, (u32, u32))>,
        frame: &mut Vec<u8>,
        world: &SceneWorld,
        camera: Option<automata_renderer::Camera3D>,
        size: (u32, u32),
    ) -> usize {
        // Reuse the render target across frames (recreate on resize).
        if target.as_ref().map(|(_, s)| *s) != Some(size) {
            *target = Some((renderer.create_render_target(size), size));
        }
        let Some((texture, _)) = target.as_ref() else {
            return 0;
        };
        let mut draw = DrawList::new();
        draw.set_clear_color(wgpu_color(0.012, 0.012, 0.012, 1.0));
        automata_scene::record_draw_list(world, &mut draw);
        // The editor's orbit camera overrides any scene camera so the
        // viewport is freely navigable regardless of scene contents.
        if let Some(camera) = camera {
            draw.set_camera_3d(camera);
        }
        renderer.render_offscreen(&draw, texture);
        renderer.read_texture_rgba_into(texture, size, frame);
        draw.mesh_count()
    }
}

fn wgpu_color(r: f64, g: f64, b: f64, a: f64) -> automata_renderer::Color {
    automata_renderer::Color { r, g, b, a }
}

/// A CPU wireframe fallback used when no GPU adapter is present.
///
/// It projects through the same [`crate::view::Projector`] the editor's
/// overlays and picking use, so the fallback lines up with selection boxes
/// and gizmos exactly.
pub mod render_wireframe {
    use glam::Vec3;

    use crate::doc::Document;
    use crate::ui::Rect;
    use crate::view::{self, Projector, ViewCamera};

    type Rgba = (u8, u8, u8, u8);

    /// Renders the document's meshes as wireframes onto an RGBA buffer,
    /// viewed from the editor orbit camera `view`.
    #[must_use]
    pub fn render(doc: &Document, view: ViewCamera, width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::new();
        render_into(doc, view, width, height, &mut px);
        px
    }

    /// Like [`render`], but reuses `px` (resized to `width*height*4`).
    pub fn render_into(doc: &Document, cam: ViewCamera, width: u32, height: u32, px: &mut Vec<u8>) {
        let (w, h) = (width as usize, height as usize);
        px.clear();
        px.resize(w * h * 4, 0);
        for chunk in px.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[3, 3, 3, 255]);
        }
        let proj = Projector::new(&cam, Rect::new(0.0, 0.0, width as f32, height as f32));
        let mut canvas = Lines {
            px: px.as_mut_slice(),
            w,
            h,
            proj,
        };
        canvas.grid();
        for entity in &doc.entities {
            let corners = entity
                .data
                .mesh_visual
                .as_ref()
                .and_then(|_| view::box_corners(&entity.data));
            if let Some(corners) = corners {
                let col = entity
                    .data
                    .mesh_visual
                    .as_ref()
                    .map_or((232, 234, 240, 255), |m| {
                        let c = |i: usize| (m.color[i].clamp(0.0, 1.0) * 255.0) as u8;
                        (c(0), c(1), c(2), 255)
                    });
                for i in 0..8 {
                    for bit in [1usize, 2, 4] {
                        if i & bit == 0 {
                            canvas.edge(corners[i], corners[i | bit], col);
                        }
                    }
                }
            } else if let Some(t) = entity.data.transform {
                let c = Vec3::from(t.position);
                let col = (150, 156, 170, 255);
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    canvas.edge(c - axis * 0.3, c + axis * 0.3, col);
                }
            }
        }
    }

    struct Lines<'a> {
        px: &'a mut [u8],
        w: usize,
        h: usize,
        proj: Projector,
    }

    impl Lines<'_> {
        fn set(&mut self, x: i32, y: i32, c: Rgba) {
            if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
                let i = ((y as usize) * self.w + x as usize) * 4;
                self.px[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, c.3]);
            }
        }

        fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Rgba) {
            // Reject absurdly long segments (points far behind the camera).
            let limit = 4.0 * (self.w + self.h) as f32;
            if a.0.abs().max(a.1.abs()).max(b.0.abs()).max(b.1.abs()) > limit {
                return;
            }
            let (mut x0, mut y0) = (a.0 as i32, a.1 as i32);
            let (x1, y1) = (b.0 as i32, b.1 as i32);
            let dx = (x1 - x0).abs();
            let dy = -(y1 - y0).abs();
            let sx = if x0 < x1 { 1 } else { -1 };
            let sy = if y0 < y1 { 1 } else { -1 };
            let mut err = dx + dy;
            loop {
                self.set(x0, y0, c);
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

        fn edge(&mut self, a: Vec3, b: Vec3, c: Rgba) {
            if let (Some(pa), Some(pb)) = (self.proj.project(a), self.proj.project(b)) {
                self.line(pa, pb, c);
            }
        }

        fn grid(&mut self) {
            let half = 10;
            let hf = half as f32;
            for i in -half..=half {
                let f = i as f32;
                let (ca, cb) = if i == 0 {
                    ((96, 148, 224, 255), (224, 96, 100, 255))
                } else {
                    ((44, 48, 58, 255), (44, 48, 58, 255))
                };
                self.edge(Vec3::new(f, 0.0, -hf), Vec3::new(f, 0.0, hf), ca);
                self.edge(Vec3::new(-hf, 0.0, f), Vec3::new(hf, 0.0, f), cb);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_viewport_follows_the_orbit_camera() {
        let doc = Document::starter();
        let mut vp = ViewportRenderer::new();
        let a = vp
            .render(None, &doc, ViewCamera::default(), 320, 200)
            .0
            .to_vec();
        let turned = ViewCamera {
            yaw: 2.0,
            ..ViewCamera::default()
        };
        let b = vp.render(None, &doc, turned, 320, 200).0.to_vec();
        assert_eq!(a.len(), 320 * 200 * 4);
        assert_ne!(a, b, "orbiting must change the rendered view");
    }
}
