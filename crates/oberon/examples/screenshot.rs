//! Renders the Oberon editor UI to a PNG for visual review, headless.
//!
//! Run from the repo root:
//!
//! ```sh
//! cargo run --offline -p oberon --example screenshot -- /tmp/oberon.png
//! ```

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

    let pointer = Pointer::default();
    let _intents = ui.frame(&mut canvas, &doc, pointer, Some((&frame, fw, fh)));

    // tiny-skia pixels are premultiplied RGBA; the UI is opaque, so write
    // them straight out as PNG.
    image::save_buffer(&out, canvas.pixels(), w, h, image::ExtendedColorType::Rgba8)
        .expect("write png");
    println!("wrote {out} ({w}x{h})");
}
