//! Renders the Oberon editor UI to a PNG for visual review, headless.
//!
//! Run from the repo root:
//!
//! ```sh
//! cargo run --offline -p oberon --example screenshot -- /tmp/oberon.png
//! ```

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use oberon::canvas::Canvas;
use oberon::doc::Document;
use oberon::ui::{Layout, Pointer, Ui};

#[allow(clippy::too_many_lines)]
fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/oberon.png".to_string());
    let (w, h) = (1600u32, 950u32);
    let mut canvas = Canvas::new(w, h);
    // `OBERON_SCENE=level.ron` screenshots a scene file instead of the starter.
    let mut doc = std::env::var("OBERON_SCENE")
        .ok()
        .and_then(|p| Document::load(std::path::Path::new(&p)).ok())
        .unwrap_or_else(Document::starter);
    // Pre-select the cube (or `OBERON_SELECT=<name>`) so the inspector
    // shows content.
    let pick = std::env::var("OBERON_SELECT").unwrap_or_else(|_| "Cube".to_string());
    if let Some(e) = doc.entities.iter().find(|e| e.name() == pick) {
        doc.select(Some(e.id));
    }
    if let Some(dir) = doc.path.as_ref().map(|_| doc.base_dir()) {
        automata_assets::set_root(Some(&dir));
    }
    let mut ui = Ui::new();
    if doc.path.is_some() {
        ui.project_root = doc.base_dir();
    }

    // Render the viewport frame on an offscreen GPU renderer when one is
    // available; otherwise use the software wireframe fallback.
    let view = Layout::compute(w as f32, h as f32).viewport;
    let (vw, vh) = (view.w as u32, view.h as u32);
    let mut gpu = pollster::block_on(automata_renderer::Renderer::new_offscreen((vw, vh))).ok();
    let mut viewport = oberon::render::ViewportRenderer::new();
    let (frame, fw, fh) = viewport.render(gpu.as_mut(), &doc, ui.camera, vw, vh);
    let frame = frame.to_vec();

    // Optional scene: `picker`, `menu`, `dialog`, `project`, `code`,
    // `code-complete`, `code-api`, `settings`, `build`, `build-menu`, `palette`,
    // `assets`, `assets-sounds`, `assets-fonts`, `bone-picker`, `clip-picker`.
    let scene = std::env::args().nth(2).unwrap_or_default();
    let selected = doc.selected;
    let pointer = Pointer {
        x: 1180.0,
        y: 330.0,
        ..Pointer::default()
    };
    let _ = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));
    // `OBERON_SCROLL=<pixels>` scrolls the Inspector down first.
    if let Some(px) = std::env::var("OBERON_SCROLL")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    {
        let insp = Layout::compute(w as f32, h as f32).inspector;
        let wheel = Pointer {
            x: insp.x + insp.w / 2.0,
            y: insp.y + insp.h / 2.0,
            scroll: -px / 48.0,
            ..Pointer::default()
        };
        let _ = ui.frame(&mut canvas, &doc, wheel, Some((&frame, fw, fh)));
        let _ = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));
    }
    match scene.as_str() {
        "picker" => {
            if let Some(id) = selected {
                ui.open_color_picker(&doc, id, oberon::ui::ColorTarget::Mesh);
            }
        }
        "menu" => ui.open_bar_menu(0, &doc),
        "dialog" => ui.open_dialog_kind(oberon::ui::DialogKind::SaveSceneAs, &doc),
        "browser" | "browser-filter" => {
            if scene == "browser-filter" {
                ui.open_dialog_kind(oberon::ui::DialogKind::OpenScript, &doc);
                ui.type_text("c");
            } else {
                ui.open_dialog_kind(oberon::ui::DialogKind::OpenScene, &doc);
            }
        }
        "files-menu" => {
            let root = ui.project_root.clone();
            if let Some(script) = std::fs::read_dir(root.join("scripts"))
                .ok()
                .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.is_file()))
            {
                ui.reveal_in_files(&script);
                ui.begin_file_rename(&script);
                ui.type_text("hero");
            }
        }
        "project" | "files" => {
            // Reveal a script so the Files tree shows a couple of open folders.
            let root = ui.project_root.clone();
            if let Some(script) = ["scripts/coin.py", "scripts/player.py"]
                .iter()
                .map(|s| root.join(s))
                .find(|p| p.is_file())
            {
                ui.reveal_in_files(&script);
            }
        }
        "code" | "code-complete" => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/python/oop_platformer.py");
            ui.open_code(&path, Some(40));
            ui.code.focused = true;
            if let Some(tab) = ui.code.current_mut() {
                tab.diagnostics.push(oberon::code::Diagnostic {
                    start: oberon::code::Pos::new(44, 8),
                    end: oberon::code::Pos::new(44, 20),
                    severity: oberon::code::Severity::Error,
                    message: "\"velocty\" is not defined".to_string(),
                });
                tab.diagnostics.push(oberon::code::Diagnostic {
                    start: oberon::code::Pos::new(30, 4),
                    end: oberon::code::Pos::new(30, 10),
                    severity: oberon::code::Severity::Warning,
                    message: "Import \"math\" is not accessed".to_string(),
                });
            }
            if scene == "code-complete" {
                use oberon::code::CompletionKind as K;
                ui.code_text("\n        self.");
                let item = |label: &str, detail: &str, kind| oberon::code::CompletionItem {
                    label: label.to_string(),
                    detail: detail.to_string(),
                    insert: label.to_string(),
                    kind,
                };
                ui.code.show_completions(vec![
                    item("entity", "Entity", K::Class),
                    item("on_update", "(self, ctx) -> None", K::Method),
                    item("speed", "float", K::Variable),
                    item("jump", "(self) -> None", K::Function),
                    item("transform", "Transform", K::Property),
                    item("MAX_SPEED", "int", K::Constant),
                    item("math", "module", K::Module),
                    item("return", "", K::Keyword),
                ]);
            } else {
                ui.code_key(
                    oberon::code::Key::Char('f'),
                    oberon::code::Mods {
                        ctrl: true,
                        ..Default::default()
                    },
                );
                ui.code_text("ctx");
            }
        }
        "ui-editor" | "ui-editor-hud" => {
            // The UI editor on the Shape Signal demo's title screen (or HUD),
            // with a widget selected so the properties show.
            let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/oberon/shape_signal/ui");
            let hud = scene == "ui-editor-hud";
            let file = if hud { "hud.ui.ron" } else { "title.ui.ron" };
            ui.open_code(&ui_dir.join(file), None);
            ui.code.focused = true;
            let _ = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));
            ui.select_ui_node(Some(if hud { vec![3] } else { vec![0, 3] }));
            if hud {
                ui.set_ui_preview(1);
            }
        }
        "code-tabs" | "code-switcher" => {
            // Many tabs (the strip scrolls), multi-caret editing, and the
            // Ctrl+Tab switcher.
            let dir =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/python");
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            if let Ok(d) = std::fs::read_dir(dir.join("zero_to_hero")) {
                files.extend(d.flatten().map(|e| e.path()));
            }
            files.retain(|p| p.extension().is_some_and(|e| e == "py"));
            files.sort();
            for f in files.iter().take(12) {
                ui.open_code(f, None);
            }
            let platformer = dir.join("oop_platformer.py");
            ui.open_code(&platformer, Some(60));
            ui.code.focused = true;
            if let Some(tab) = ui.code.current_mut() {
                // On "self" of `self.add(at.Transform(...))`.
                tab.buf.move_to(oberon::code::Pos::new(62, 9), false);
            }
            let ctrl = oberon::code::Mods {
                ctrl: true,
                ..Default::default()
            };
            if scene == "code-tabs" {
                for _ in 0..3 {
                    ui.code_key(oberon::code::Key::Char('d'), ctrl);
                }
            } else {
                ui.code_key(oberon::code::Key::Tab, ctrl);
                ui.code_key(oberon::code::Key::Tab, ctrl);
            }
        }
        "graphics" | "camera" => {
            let existing = doc
                .entities
                .iter()
                .find(|e| e.data.camera_3d.is_some())
                .map(|e| e.id);
            let cam = existing.unwrap_or_else(|| doc.create(oberon::doc::CreateKind::Camera));
            if let (None, Some(i)) = (existing, doc.index_of(cam)) {
                let d = &mut doc.entities[i].data;
                d.name = Some("Main Camera".into());
                d.camera_3d = Some(automata_renderer::Camera3D::new(
                    glam::Vec3::new(6.0, 4.0, 8.0),
                    glam::Vec3::new(0.0, 0.5, 0.0),
                ));
                if let Some(t) = &mut d.transform {
                    t.position = [6.0, 4.0, 8.0];
                }
            }
            doc.select(Some(cam));
            if scene == "graphics" {
                if !doc.entities.iter().any(|e| e.data.environment.is_some()) {
                    doc.create(oberon::doc::CreateKind::Environment);
                }
                ui.right_tab = oberon::ui::RightTab::Graphics;
                ui.stats = oberon::ui::FrameStats {
                    frame_ms: 4.2,
                    meshes: 6,
                };
                doc.select(selected);
            } else {
                let c = doc.entities[doc.index_of(cam).unwrap()]
                    .data
                    .camera_3d
                    .unwrap();
                let (pw, ph) = (416u32, 234u32);
                let px = oberon::render::render_wireframe::render(
                    &doc,
                    oberon::ui::ViewCamera::from_camera3d(&c),
                    pw,
                    ph,
                );
                ui.set_camera_preview(Some((&px, pw, ph)));
            }
        }
        "settings" => {
            ui.open_settings_page(oberon::ui::settings::SettingsPage::Editor);
        }
        "build" => {
            ui.open_settings_page(oberon::ui::settings::SettingsPage::Build);
            ui.build_last = Some((true, "Build finished in 1.4 s → build".to_string()));
        }
        "build-menu" => ui.open_bar_menu(5, &doc),
        "assets" | "assets-sounds" | "assets-fonts" => {
            ui.toggle_assets();
            if scene == "assets-sounds" {
                ui.assets.filter = oberon::ui::assets_panel::AssetFilter::Sounds;
            } else if scene == "assets-fonts" {
                ui.assets.filter = oberon::ui::assets_panel::AssetFilter::Fonts;
            }
            // Thumbnails fill in a few per frame.
            for _ in 0..6 {
                let _ = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));
            }
        }
        "bone-picker" => {
            // The spring chain bone picker for the selected model, as if it
            // had no chains yet (so the suggestions show).
            let entity = selected.and_then(|id| doc.entities.iter().find(|e| e.id == id));
            if let Some(e) = entity {
                let rig = oberon::doc::model_rig(&e.data);
                let path = oberon::doc::model_path(&e.data).map(|p| doc.base_dir().join(p));
                if let (Some(rig), Some(path)) = (rig, path) {
                    let mut bare = (*rig).clone();
                    bare.springs = automata_assets::springs::Springs::default();
                    ui.open_bone_picker(&path, &bare, oberon::ui::springs::SpringPick::Chain);
                }
            }
        }
        "clip-picker" => {
            // The "+ Clips from file…" picker for the selected model.
            let entity = selected.and_then(|id| doc.entities.iter().find(|e| e.id == id));
            if let Some(path) = entity
                .and_then(|e| oberon::doc::model_path(&e.data))
                .map(|p| doc.base_dir().join(p))
            {
                ui.open_clip_file_picker(&path);
            }
        }
        "palette" => {
            ui.open_palette(&doc);
            ui.palette_text("sa");
        }
        "code-api" => {
            let path = std::env::temp_dir().join("oberon_screenshot_player.py");
            let src = "SPEED = 6.0\n\n\ndef start(entity, ctx):\n    ctx.log(\"ready\", entity.name)\n\n\ndef update(entity, ctx):\n    move = ctx.input.axis(\"a\", \"d\") * SPEED\n    entity.transform.translate((move * ctx.time.delta, 0.0, 0.0))\n    \n";
            std::fs::write(&path, src).expect("write script");
            ui.open_code(&path, Some(11));
            ui.code.focused = true;
            if let Some(tab) = ui.code.current_mut() {
                tab.buf.move_to(oberon::code::Pos::new(10, 4), false);
            }
            ui.code_text("if ctx.input.just_pressed(\"");
        }
        _ => {}
    }
    let _intents = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));

    // tiny-skia pixels are premultiplied RGBA; the UI is opaque, so write
    // them straight out as PNG.
    image::save_buffer(&out, canvas.pixels(), w, h, image::ExtendedColorType::Rgba8)
        .expect("write png");
    println!("wrote {out} ({w}x{h})");
}
