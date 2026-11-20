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
    let mut doc = Document::starter();
    // Pre-select the cube so the inspector shows content.
    if let Some(cube) = doc.entities.iter().find(|e| e.name() == "Cube") {
        doc.select(Some(cube.id));
    }
    let mut ui = Ui::new();

    // Render the viewport frame on an offscreen GPU renderer when one is
    // available; otherwise use the software wireframe fallback.
    let view = Layout::compute(w as f32, h as f32).viewport;
    let (vw, vh) = (view.w as u32, view.h as u32);
    let mut gpu = pollster::block_on(automata_renderer::Renderer::new_offscreen((vw, vh))).ok();
    let mut viewport = oberon::render::ViewportRenderer::new();
    let (frame, fw, fh) = viewport.render(gpu.as_mut(), &doc, ui.camera, vw, vh);
    let frame = frame.to_vec();

    // Optional scene: `picker`, `menu`, `dialog`, `project`, `code`,
    // `code-complete`, `code-api`, `settings`, `palette`.
    let scene = std::env::args().nth(2).unwrap_or_default();
    let selected = doc.selected;
    let pointer = Pointer {
        x: 1180.0,
        y: 330.0,
        ..Pointer::default()
    };
    let _ = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));
    match scene.as_str() {
        "picker" => {
            if let Some(id) = selected {
                ui.open_color_picker(&doc, id, oberon::ui::ColorTarget::Mesh);
            }
        }
        "menu" => ui.open_bar_menu(0, &doc),
        "dialog" => ui.open_dialog_kind(oberon::ui::DialogKind::SaveSceneAs, &doc),
        "project" => ui.bottom_tab = oberon::ui::BottomTab::Project,
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
                ui.code_text("\n        self.");
                let item = |label: &str, detail: &str, kind| oberon::code::CompletionItem {
                    label: label.to_string(),
                    detail: detail.to_string(),
                    insert: label.to_string(),
                    kind,
                };
                ui.code.show_completions(vec![
                    item("entity", "Entity", 'v'),
                    item("on_update", "(self, ctx) -> None", 'm'),
                    item("speed", "float", 'v'),
                    item("jump", "(self) -> None", 'm'),
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
        "settings" => {
            ui.open_settings_page(oberon::ui::settings::SettingsPage::Editor);
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
