//! The Graphics panel (right dock, next to the Inspector): renderer
//! quality, the scene's sun, environment and camera in one place, plus
//! one-click test arrangements and live frame statistics for checking the
//! renderer.
//!
//! Everything it edits lives on ordinary entities (the first Directional
//! Light, Environment, and Camera in the scene), through the same rows and
//! intents the Inspector uses, so undo, saving, and play mode just work.

#![forbid(unsafe_code)]

use automata_renderer::{Environment, GraphicsQuality, PostEffects};

use super::{ButtonStyle, CursorHint, Intent, PlayState, Rect, Ui};
use crate::canvas::{Canvas, Face};
use crate::doc::{CreateKind, Document, EntityEntry, TestScene};
use crate::fields::{Choice, Field, Flag};
use crate::theme::{color, layout};
use crate::ui::ColorTarget;

/// Tabs of the right dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RightTab {
    /// The selected entity's components.
    #[default]
    Inspector,
    /// Scene-wide rendering: quality, sun, environment, camera, tests.
    Graphics,
}

/// Frame statistics the app reports for the Graphics panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameStats {
    /// Smoothed frame time in milliseconds (whole editor frame).
    pub frame_ms: f32,
    /// Meshes drawn in the scene view last frame.
    pub meshes: usize,
}

/// Named environment looks offered as one-click presets.
type Preset = (&'static str, fn() -> Environment);

/// The FPS Limit choices: label and cap (0 = none).
const FPS_LIMITS: [(&str, u32); 5] = [
    ("Off", 0),
    ("30", 30),
    ("60", 60),
    ("120", 120),
    ("144", 144),
];

const PRESETS: [Preset; 4] = [
    ("Neutral", Environment::default),
    ("Daylight", Environment::daylight),
    ("Dusk", Environment::dusk),
    ("Night", Environment::night),
];

impl Ui {
    /// The right dock's tab strip (replaces the panel title).
    pub(super) fn right_tabs(&mut self, canvas: &mut Canvas, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, layout::TITLEBAR_H, color::surface());
        canvas.fill_rect(
            rect.x,
            rect.y + layout::TITLEBAR_H - 1.0,
            rect.w,
            1.0,
            color::border(),
        );
        let mut x = rect.x + 2.0;
        for (tab, label) in [
            (RightTab::Inspector, "Inspector"),
            (RightTab::Graphics, "Graphics"),
        ] {
            let tw = canvas.measure_text(layout::TITLE_SIZE, label) + 22.0;
            let r = Rect::new(x, rect.y, tw, layout::TITLEBAR_H);
            let active = self.right_tab == tab;
            if active {
                canvas.fill_rect(r.x + 8.0, r.y + r.h - 2.0, r.w - 16.0, 2.0, color::accent());
            }
            canvas.text(
                r.x + 11.0,
                r.y + (r.h - layout::TITLE_SIZE) / 2.0 - 1.0,
                layout::TITLE_SIZE,
                label,
                if active {
                    color::text()
                } else {
                    color::text_dim()
                },
            );
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed && !active {
                    self.right_tab = tab;
                    self.inspector_scroll = 0.0;
                }
            }
            x += tw + 2.0;
        }
    }

    /// The Graphics tab's content. Returns the y after the last row.
    pub(super) fn graphics_body(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let mut y = self.renderer_group(canvas, x, y, w);
        y = self.sun_group(canvas, doc, x, y, w);
        y = self.environment_group(canvas, doc, x, y, w);
        y = self.style_group(canvas, doc, x, y, w);
        y = self.post_group(canvas, doc, x, y, w);
        y = self.camera_group(canvas, doc, x, y, w);
        y = self.tests_group(canvas, doc, x, y, w);
        y + 12.0
    }

    /// A group heading: medium-weight title with generous space above.
    fn gfx_heading(&self, canvas: &mut Canvas, x: f32, y: f32, w: f32, title: &str) -> f32 {
        let r = Rect::new(x, y, w, 30.0);
        if self.row_visible(r) {
            canvas.fill_rect(x - 4.0, y, w + 8.0, 1.0, color::border());
            canvas.text_in(
                Face::Medium,
                x,
                y + 12.0,
                layout::FONT_SIZE,
                title,
                color::text(),
            );
        }
        y + 36.0
    }

    /// A line of secondary text (wrapped by the caller).
    fn gfx_note(&self, canvas: &mut Canvas, x: f32, y: f32, w: f32, text: &str) -> f32 {
        if self.row_visible(Rect::new(x, y, w, 16.0)) {
            canvas.text_clipped(x, y, layout::SMALL_SIZE, text, color::text_dim(), w);
        }
        y + 17.0
    }

    fn editable_style(&self, style: ButtonStyle) -> ButtonStyle {
        if self.play == PlayState::Editing {
            style
        } else {
            ButtonStyle::Disabled
        }
    }

    /// Quality preset, frame pacing and live statistics.
    fn renderer_group(&mut self, canvas: &mut Canvas, x: f32, y: f32, w: f32) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Renderer");
        // Quality: a segmented control (one option is always active).
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "Quality");
            let area = w - layout::LABEL_W;
            let n = GraphicsQuality::ALL.len() as f32;
            let seg = area / n;
            canvas.fill_rounded(
                x + layout::LABEL_W,
                y,
                area,
                layout::FIELD_H,
                layout::RADIUS,
                color::field(),
            );
            for (i, q) in GraphicsQuality::ALL.into_iter().enumerate() {
                let b = Rect::new(
                    x + layout::LABEL_W + seg * i as f32 + 1.0,
                    y + 1.0,
                    seg - 2.0,
                    layout::FIELD_H - 2.0,
                );
                let active = self.prefs.graphics_quality == q;
                let style = if active {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Ghost
                };
                if self.button(canvas, b, q.label(), style) && !active {
                    self.prefs.graphics_quality = q;
                    self.prefs_dirty = true;
                }
            }
        }
        y += layout::ROW_STEP;
        // V-Sync and the FPS limit (editor preferences).
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "V-Sync");
            let b = Rect::new(x + layout::LABEL_W, y + 1.0, 18.0, 18.0);
            if self.checkbox(canvas, b, self.prefs.vsync) {
                self.prefs.vsync = !self.prefs.vsync;
                self.prefs_dirty = true;
            }
            canvas.text(
                b.x + 26.0,
                y + 4.0,
                layout::SMALL_SIZE,
                "Match the display's refresh rate",
                color::text_dim(),
            );
        }
        y += layout::ROW_STEP;
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "FPS Limit");
            let area = w - layout::LABEL_W;
            let seg = area / FPS_LIMITS.len() as f32;
            canvas.fill_rounded(
                x + layout::LABEL_W,
                y,
                area,
                layout::FIELD_H,
                layout::RADIUS,
                color::field(),
            );
            for (i, (label, limit)) in FPS_LIMITS.iter().enumerate() {
                let b = Rect::new(
                    x + layout::LABEL_W + seg * i as f32 + 1.0,
                    y + 1.0,
                    seg - 2.0,
                    layout::FIELD_H - 2.0,
                );
                let active = self.prefs.fps_limit == *limit;
                let style = if active {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Ghost
                };
                if self.button(canvas, b, label, style) && !active {
                    self.prefs.fps_limit = *limit;
                    self.prefs_dirty = true;
                }
            }
        }
        y += layout::ROW_STEP;
        let shadow = match self.prefs.graphics_quality {
            GraphicsQuality::Low => "Shadows off".to_string(),
            q if q.is_beautiful() => format!(
                "{}×{} px soft shadows · HDR bloom · FXAA",
                q.shadow_cascades(),
                q.shadow_map_size()
            ),
            q if q.shadow_cascades() > 1 => format!(
                "{}×{} px shadow cascades",
                q.shadow_cascades(),
                q.shadow_map_size()
            ),
            q => format!("{} px shadow map", q.shadow_map_size()),
        };
        let s = self.stats;
        let fps = if s.frame_ms > 0.01 {
            1000.0 / s.frame_ms
        } else {
            0.0
        };
        let line = format!(
            "{:.1} ms  ·  {fps:.0} fps  ·  {} meshes  ·  {shadow}",
            s.frame_ms, s.meshes
        );
        y = self.gfx_note(canvas, x, y + 2.0, w, &line);
        y + 10.0
    }

    /// The first directional light: angles, color, ambient, shadows.
    fn sun_group(&mut self, canvas: &mut Canvas, doc: &Document, x: f32, y: f32, w: f32) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Sun");
        let Some(sun) = doc
            .entities
            .iter()
            .find(|e| e.data.directional_light.is_some())
        else {
            y = self.gfx_note(canvas, x, y, w, "No sun: meshes get a flat default light.");
            let b = Rect::new(x, y + 4.0, w, 28.0);
            if self.row_visible(b) {
                let style = self.editable_style(ButtonStyle::Normal);
                if self.button(canvas, b, "Add Directional Light", style) {
                    self.out.push(Intent::Create(CreateKind::Light));
                }
            }
            return y + 44.0;
        };
        y = self.float_row(canvas, x, y, w, sun, "Time of Day", Field::LightTimeOfDay);
        y = self.float_row(canvas, x, y, w, sun, "Azimuth", Field::LightAzimuth);
        y = self.float_row(canvas, x, y, w, sun, "Elevation", Field::LightElevation);
        y = self.color_row(canvas, x, y, w, sun, "Color", ColorTarget::Light);
        y = self.float_row(canvas, x, y, w, sun, "Ambient", Field::LightAmbient);
        y = self.flag_row(canvas, x, y, w, sun, "Cast Shadows", Flag::LightShadows);
        y = self.float_row(
            canvas,
            x,
            y,
            w,
            sun,
            "Shadow Distance",
            Field::LightShadowDistance,
        );
        y = self.float_row(
            canvas,
            x,
            y,
            w,
            sun,
            "Shadow Softness",
            Field::LightShadowSoftness,
        );
        y = self.source_note(canvas, x, y, w, sun);
        y + 8.0
    }

    /// "From Sun · Select" — which entity these rows edit.
    fn source_note(&mut self, canvas: &mut Canvas, x: f32, y: f32, w: f32, e: &EntityEntry) -> f32 {
        let r = Rect::new(x, y, w, 18.0);
        if self.row_visible(r) {
            let text = format!("On entity “{}”", e.name());
            canvas.text_clipped(
                x,
                y + 2.0,
                layout::SMALL_SIZE,
                &text,
                color::text_faint(),
                w - 60.0,
            );
            let b = Rect::new(x + w - 56.0, y - 2.0, 56.0, 20.0);
            if self.button(canvas, b, "Select", ButtonStyle::Ghost) {
                self.out.push(Intent::Select(Some(e.id)));
                self.right_tab = RightTab::Inspector;
            }
        }
        y + 22.0
    }

    /// The first Environment: presets, ambient, exposure, fog.
    fn environment_group(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Environment");
        let Some(env) = doc.entities.iter().find(|e| e.data.environment.is_some()) else {
            y = self.gfx_note(canvas, x, y, w, "This scene uses the neutral default look:");
            y = self.gfx_note(canvas, x, y, w, "white ambient, no fog, no tone mapping.");
            let b = Rect::new(x, y + 4.0, w, 28.0);
            if self.row_visible(b) {
                let style = self.editable_style(ButtonStyle::Normal);
                if self.button(canvas, b, "Customize Environment", style) {
                    self.out.push(Intent::Create(CreateKind::Environment));
                }
            }
            return y + 44.0;
        };
        // Presets: one row of quiet buttons.
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "Preset");
            let area = w - layout::LABEL_W;
            let seg = area / PRESETS.len() as f32;
            let current = env.data.environment;
            for (i, (label, make)) in PRESETS.iter().enumerate() {
                let b = Rect::new(
                    x + layout::LABEL_W + seg * i as f32,
                    y,
                    seg - 2.0,
                    layout::FIELD_H,
                );
                // The look's sky, light and fog; the post effects stay.
                let value = Environment {
                    post: current.map_or_else(PostEffects::default, |e| e.post),
                    ..make()
                };
                let style = if current == Some(value) {
                    ButtonStyle::Active
                } else {
                    self.editable_style(ButtonStyle::Ghost)
                };
                if self.button(canvas, b, label, style) {
                    self.out
                        .push(Intent::SetEnvironment(env.id, Box::new(value)));
                }
            }
        }
        y += layout::ROW_STEP;
        y = self.color_row(canvas, x, y, w, env, "Sky", ColorTarget::EnvSky);
        y = self.color_row(canvas, x, y, w, env, "Ground", ColorTarget::EnvGround);
        y = self.sky_rows(canvas, x, y, w, env);
        y = self.choice_row(canvas, x, y, w, env, "Tone Mapping", Choice::Tonemapping);
        y = self.float_row(canvas, x, y, w, env, "Exposure", Field::EnvExposure);
        y = self.float_row(
            canvas,
            x,
            y,
            w,
            env,
            "Occlusion",
            Field::EnvAmbientOcclusion,
        );
        y = self.flag_row(canvas, x, y, w, env, "Fog", Flag::EnvFog);
        if env.data.environment.is_some_and(|e| e.fog) {
            y = self.color_row(canvas, x, y, w, env, "Fog Color", ColorTarget::EnvFog);
            y = self.float_row(canvas, x, y, w, env, "Fog Density", Field::EnvFogDensity);
            y = self.float_row(canvas, x, y, w, env, "Fog Start", Field::EnvFogStart);
        }
        y = self.atmosphere_rows(canvas, x, y, w, env);
        y = self.source_note(canvas, x, y, w, env);
        y + 8.0
    }

    /// The first Environment's style: a pick of whole looks ("Custom" once
    /// a knob changes), then its shading, outline and pixel settings.
    fn style_group(&mut self, canvas: &mut Canvas, doc: &Document, x: f32, y: f32, w: f32) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Style");
        let Some(env) = doc.entities.iter().find(|e| e.data.environment.is_some()) else {
            y = self.gfx_note(canvas, x, y, w, "Styles live on the Environment:");
            y = self.gfx_note(canvas, x, y, w, "customize it above.");
            return y + 8.0;
        };
        y = self.gfx_note(
            canvas,
            x,
            y,
            w,
            "Sets shading, lens and post; colours and sky stay.",
        );
        y = self.choice_row(canvas, x, y, w, env, "Style", Choice::EnvStyle);
        y = self.style_rows(canvas, x, y, w, env);
        y = self.gfx_note(
            canvas,
            x,
            y,
            w,
            "Outlines and pixels: Medium graphics and up.",
        );
        y + 8.0
    }

    /// An Environment's stylized shading, outlines and pixels (the
    /// Graphics tab and the Inspector).
    pub(super) fn style_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        env: &EntityEntry,
    ) -> f32 {
        let e = env.data.environment;
        let mut y = self.float_row(canvas, x, y, w, env, "Toon Bands", Field::EnvToonBands);
        if e.is_some_and(|e| e.toon_bands > 0) {
            y = self.float_row(
                canvas,
                x,
                y,
                w,
                env,
                "Band Softness",
                Field::EnvToonSoftness,
            );
        }
        y = self.color_row(
            canvas,
            x,
            y,
            w,
            env,
            "Shade Tint",
            ColorTarget::EnvShadowTint,
        );
        y = self.float_row(canvas, x, y, w, env, "Rim Light", Field::EnvRimLight);
        if e.is_some_and(|e| e.sky) {
            y = self.flag_row(canvas, x, y, w, env, "Painted Sky", Flag::EnvPaintedSky);
        }
        y = self.float_row(canvas, x, y, w, env, "Outline", Field::EnvOutline);
        if e.is_some_and(|e| e.outline > 0.0) {
            y = self.color_row(
                canvas,
                x,
                y,
                w,
                env,
                "Outline Color",
                ColorTarget::EnvOutline,
            );
        }
        y = self.float_row(canvas, x, y, w, env, "Pixelate", Field::EnvPixelate);
        self.float_row(canvas, x, y, w, env, "Palette", Field::EnvPalette)
    }

    /// The first Environment's post effects: a row of looks, then every
    /// setting.
    fn post_group(&mut self, canvas: &mut Canvas, doc: &Document, x: f32, y: f32, w: f32) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Post");
        let Some(env) = doc.entities.iter().find(|e| e.data.environment.is_some()) else {
            y = self.gfx_note(
                canvas,
                x,
                y,
                w,
                "Grading, bloom, vignette and grain live on",
            );
            y = self.gfx_note(canvas, x, y, w, "the Environment: customize it above.");
            return y + 8.0;
        };
        y = self.gfx_note(
            canvas,
            x,
            y,
            w,
            "Medium graphics and up; bloom on High and up.",
        );
        // Looks: one row of quiet buttons across the panel.
        let r = Rect::new(x, y + 4.0, w, layout::FIELD_H);
        if self.row_visible(r) {
            let seg = w / PostEffects::PRESETS.len() as f32;
            let current = env.data.environment;
            for (i, (label, make)) in PostEffects::PRESETS.iter().enumerate() {
                let b = Rect::new(x + seg * i as f32, r.y, seg - 2.0, layout::FIELD_H);
                let post = make();
                let style = if current.is_some_and(|e| e.post == post) {
                    ButtonStyle::Active
                } else {
                    self.editable_style(ButtonStyle::Ghost)
                };
                if self.button(canvas, b, label, style) {
                    if let Some(e) = current {
                        self.out.push(Intent::SetEnvironment(
                            env.id,
                            Box::new(Environment { post, ..e }),
                        ));
                    }
                }
            }
        }
        y += layout::ROW_STEP + 4.0;
        y = self.post_rows(canvas, x, y, w, env);
        y = self.source_note(canvas, x, y, w, env);
        y + 8.0
    }

    /// Height fog (with fog on), god rays and depth of field (the Graphics
    /// tab and the Inspector).
    pub(super) fn atmosphere_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        env: &EntityEntry,
    ) -> f32 {
        let mut y = y;
        let e = env.data.environment;
        if e.is_some_and(|e| e.fog) {
            y = self.float_row(canvas, x, y, w, env, "Fog Height", Field::EnvFogHeight);
            y = self.float_row(canvas, x, y, w, env, "Fog Falloff", Field::EnvFogFalloff);
            y = self.float_row(canvas, x, y, w, env, "Fog Sun Glow", Field::EnvFogSun);
        }
        y = self.float_row(canvas, x, y, w, env, "God Rays", Field::EnvGodRays);
        y = self.float_row(
            canvas,
            x,
            y,
            w,
            env,
            "Depth of Field",
            Field::EnvDepthOfField,
        );
        if e.is_some_and(|e| e.depth_of_field > 0.0) {
            y = self.float_row(
                canvas,
                x,
                y,
                w,
                env,
                "Focus Distance",
                Field::EnvFocusDistance,
            );
            y = self.float_row(canvas, x, y, w, env, "Focus Range", Field::EnvFocusRange);
        }
        y
    }

    /// An Environment's post-effect settings (the Graphics tab and the
    /// Inspector).
    pub(super) fn post_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        env: &EntityEntry,
    ) -> f32 {
        let mut y = self.float_row(canvas, x, y, w, env, "Bloom", Field::EnvBloom);
        y = self.float_row(canvas, x, y, w, env, "Contrast", Field::EnvContrast);
        y = self.float_row(canvas, x, y, w, env, "Saturation", Field::EnvSaturation);
        y = self.float_row(canvas, x, y, w, env, "Temperature", Field::EnvTemperature);
        y = self.float_row(canvas, x, y, w, env, "Tint", Field::EnvTint);
        y = self.vec3_row(canvas, x, y, w, env, "Lift", Field::EnvLift);
        y = self.vec3_row(canvas, x, y, w, env, "Gamma", Field::EnvGamma);
        y = self.vec3_row(canvas, x, y, w, env, "Gain", Field::EnvGain);
        y = self.float_row(canvas, x, y, w, env, "Vignette", Field::EnvVignette);
        y = self.float_row(canvas, x, y, w, env, "Grain", Field::EnvGrain);
        self.float_row(canvas, x, y, w, env, "Aberration", Field::EnvChromatic)
    }

    /// The background, or with the sky dome on, the sky: horizon and
    /// zenith colors, the sun's size and glow, clouds and lens flare.
    pub(super) fn sky_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        env: &EntityEntry,
    ) -> f32 {
        let mut y = self.flag_row(canvas, x, y, w, env, "Sky & Sun", Flag::EnvSkyDome);
        if !env.data.environment.is_some_and(|e| e.sky) {
            return self.color_row(
                canvas,
                x,
                y,
                w,
                env,
                "Background",
                ColorTarget::EnvBackground,
            );
        }
        y = self.color_row(canvas, x, y, w, env, "Horizon", ColorTarget::EnvBackground);
        y = self.color_row(canvas, x, y, w, env, "Zenith", ColorTarget::EnvZenith);
        y = self.float_row(canvas, x, y, w, env, "Sun Size", Field::EnvSunSize);
        y = self.float_row(canvas, x, y, w, env, "Sun Glow", Field::EnvSunGlow);
        y = self.float_row(canvas, x, y, w, env, "Clouds", Field::EnvClouds);
        self.float_row(canvas, x, y, w, env, "Lens Flare", Field::EnvLensFlare)
    }

    /// The scene camera: what Play renders through, plus view tools.
    fn camera_group(&mut self, canvas: &mut Canvas, doc: &Document, x: f32, y: f32, w: f32) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Camera");
        let Some(cam) = doc.entities.iter().find(|e| e.data.camera_3d.is_some()) else {
            y = self.gfx_note(canvas, x, y, w, "No camera: Play uses the editor view and");
            y = self.gfx_note(canvas, x, y, w, "follows the player.");
            let b = Rect::new(x, y + 4.0, w, 28.0);
            if self.row_visible(b) {
                let style = self.editable_style(ButtonStyle::Normal);
                if self.button(canvas, b, "Add Camera", style) {
                    self.out.push(Intent::Create(CreateKind::Camera));
                }
            }
            return y + 44.0;
        };
        y = self.float_row(canvas, x, y, w, cam, "Field of View", Field::CameraFov);
        y = self.camera_tools(canvas, x, y, w, cam);
        // Preview toggle (an editor preference, not scene data).
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "Preview");
            let b = Rect::new(x + layout::LABEL_W, y + 1.0, 18.0, 18.0);
            if self.checkbox(canvas, b, self.prefs.camera_preview) {
                self.prefs.camera_preview = !self.prefs.camera_preview;
                self.prefs_dirty = true;
            }
            canvas.text(
                b.x + 26.0,
                y + 4.0,
                layout::SMALL_SIZE,
                "Inset in the Scene view while selected",
                color::text_dim(),
            );
        }
        y += layout::ROW_STEP;
        y = self.gfx_note(canvas, x, y, w, "Play renders through this camera.");
        y = self.source_note(canvas, x, y, w, cam);
        y + 8.0
    }

    /// "Look Through" / "Align to View" buttons for a camera entity.
    pub(super) fn camera_tools(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        cam: &EntityEntry,
    ) -> f32 {
        let r = Rect::new(x, y, w, 26.0);
        if self.row_visible(r) {
            let half = (w - 6.0) / 2.0;
            let look = Rect::new(x, y, half, 26.0);
            if self.button(canvas, look, "Look Through", ButtonStyle::Normal) {
                if let Some(c) = cam.data.camera_3d {
                    self.camera = super::ViewCamera::from_camera3d(&c);
                }
            }
            self.tip(look, "Move the editor view to this camera");
            let align = Rect::new(x + half + 6.0, y, half, 26.0);
            let style = self.editable_style(ButtonStyle::Normal);
            if self.button(canvas, align, "Align to View", style) {
                self.out.push(Intent::AlignCameraToView(cam.id));
            }
            self.tip(align, "Place this camera where the editor view is");
        }
        y + 34.0
    }

    /// One-click arrangements for checking the renderer.
    fn tests_group(&mut self, canvas: &mut Canvas, doc: &Document, x: f32, y: f32, w: f32) -> f32 {
        let mut y = self.gfx_heading(canvas, x, y, w, "Test Scenes");
        y = self.gfx_note(
            canvas,
            x,
            y,
            w,
            "Adds objects named “Test …” (undo removes them).",
        );
        y += 4.0;
        let half = (w - 6.0) / 2.0;
        for (i, t) in TestScene::ALL.into_iter().enumerate() {
            let col = (i % 2) as f32;
            let b = Rect::new(x + col * (half + 6.0), y, half, 28.0);
            if self.row_visible(b) {
                let style = self.editable_style(ButtonStyle::Normal);
                if self.button(canvas, b, t.label(), style) {
                    self.out.push(Intent::SpawnTest(t));
                }
                self.tip(b, t.hint());
            }
            if i % 2 == 1 || i + 1 == TestScene::ALL.len() {
                y += 34.0;
            }
        }
        let tests = doc
            .entities
            .iter()
            .filter(|e| e.name().starts_with("Test "))
            .count();
        if tests > 0 {
            let b = Rect::new(x, y, w, 28.0);
            if self.row_visible(b) {
                let label = format!("Remove {tests} test objects");
                let style = self.editable_style(ButtonStyle::Danger);
                if self.button(canvas, b, &label, style) {
                    self.out.push(Intent::ClearTests);
                }
            }
            y += 34.0;
        }
        y
    }

    /// A checkbox; returns true when clicked.
    pub(super) fn checkbox(&mut self, canvas: &mut Canvas, b: Rect, on: bool) -> bool {
        let hot = self.hot(b);
        let fill = if on {
            color::accent()
        } else if hot {
            color::hover()
        } else {
            color::field()
        };
        canvas.fill_rounded(b.x, b.y, b.w, b.h, 3.0, fill);
        if on {
            canvas.polyline(
                &[
                    (b.x + 4.0, b.y + 9.5),
                    (b.x + 7.5, b.y + 13.0),
                    (b.x + 14.0, b.y + 5.0),
                ],
                color::on_accent(),
                2.0,
            );
        } else {
            canvas.stroke_rounded(b.x, b.y, b.w, b.h, 3.0, color::border(), 1.0);
        }
        if hot {
            self.cursor = CursorHint::Pointer;
            return self.p.pressed;
        }
        false
    }
}

/// The camera preview inset: where it sits inside the scene view `rect`
/// for a preview image of `w`×`h` logical pixels.
#[must_use]
pub fn preview_rect(rect: Rect, w: f32, h: f32) -> Rect {
    Rect::new(rect.x + rect.w - w - 16.0, rect.y + rect.h - h - 16.0, w, h)
}

/// The logical size of the camera preview for a scene view of `rect`:
/// a 16:9 inset a quarter of the view wide (at least 200 px).
#[must_use]
pub fn preview_size(rect: Rect) -> (f32, f32) {
    let w = (rect.w * 0.26)
        .clamp(200.0, 420.0)
        .min(rect.w - 32.0)
        .max(1.0);
    (w, (w * 9.0 / 16.0).max(1.0))
}
