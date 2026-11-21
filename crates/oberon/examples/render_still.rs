//! Renders a scene (no editor UI) to a PNG, headless — for comparing
//! graphics presets and reviewing lighting changes.
//!
//! ```sh
//! cargo run --release -p oberon --example render_still -- \
//!     examples/oberon/fps/level.ron out.png [quality] [WxH] [yaw pitch distance tx ty tz]
//! ```
//!
//! `quality` is `low`, `medium`, `high` (default), `ultra` or `beautiful`.
//! Without the camera arguments the editor's default orbit view is used.

#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::path::Path;
use std::time::Instant;

use automata_renderer::{GraphicsQuality, Renderer};
use oberon::doc::Document;
use oberon::render::ViewportRenderer;
use oberon::view::ViewCamera;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(scene), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!(
            "usage: render_still scene.ron out.png [quality] [WxH] [yaw pitch dist tx ty tz]"
        );
        std::process::exit(2);
    };
    let quality = match args.get(2).map(String::as_str) {
        None | Some("high") => GraphicsQuality::High,
        Some("low") => GraphicsQuality::Low,
        Some("medium") => GraphicsQuality::Medium,
        Some("ultra") => GraphicsQuality::Ultra,
        Some("beautiful") => GraphicsQuality::Beautiful,
        Some(other) => {
            eprintln!("unknown quality {other}");
            std::process::exit(2);
        }
    };
    let (w, h) = args
        .get(3)
        .and_then(|s| s.split_once('x'))
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .unwrap_or((1280_u32, 720_u32));
    let mut cam = ViewCamera::default();
    let nums: Vec<f32> = args.iter().skip(4).filter_map(|s| s.parse().ok()).collect();
    if let [yaw, pitch, distance, tx, ty, tz] = nums[..] {
        cam = ViewCamera {
            yaw,
            pitch,
            distance,
            target: [tx, ty, tz],
        };
    }

    let doc = Document::load(Path::new(scene)).unwrap_or_else(|e| {
        eprintln!("cannot open {scene}: {e}");
        std::process::exit(1);
    });
    automata_assets::set_root(Some(&doc.base_dir()));
    let mut gpu = pollster::block_on(Renderer::new_offscreen_with_quality((w, h), quality))
        .unwrap_or_else(|e| {
            eprintln!("no GPU: {e}");
            std::process::exit(1);
        });
    let mut viewport = ViewportRenderer::new();
    // Warm up (pipelines, uploads), then time a few frames.
    let _ = viewport.render(Some(&mut gpu), &doc, cam, w, h);
    let start = Instant::now();
    let frames = 5;
    for _ in 0..frames {
        let _ = viewport.render(Some(&mut gpu), &doc, cam, w, h);
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    let (rgba, fw, fh) = viewport.render(Some(&mut gpu), &doc, cam, w, h);
    image::save_buffer(out, rgba, fw, fh, image::ColorType::Rgba8).expect("write png");
    println!("{out}: {fw}x{fh} {quality:?}, {ms:.1} ms/frame (incl. readback)");
}
