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

    // Optional scene: `picker`, `menu`, `dialog`, `project`, `code`.
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
        "code" => {
            let path = std::path::Path::new("examples/python/oop_platformer.py");
            ui.open_code(path, Some(150));
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
