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
//!
//! With `PLAY=seconds` in the environment it plays the scene that long
//! first (physics and scripts, through the real script runner) and renders
//! that moment of Play: through the scene's camera, or the given view when
//! the camera arguments are present. `SHOOT=seconds` also holds the left
//! mouse button (with the cursor captured) from that moment of Play on, for
//! scenes whose scripts fire on a click (the FPS example); `AIM=dx,dy`
//! moves the mouse that many pixels on the first frame. `KEYS` holds keys
//! down over moments of Play: `KEYS=tab@0-0.1,w@0.5-1.5,space@1.2-1.3`
//! (key names as scripts see them; a bare `name@t` is one step).
//! `POST=bleached` (or `neutral`, `vivid`, `film`, `noir`) swaps the
//! scene Environment's post effects for that look.

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

    let mut doc = Document::load(Path::new(scene)).unwrap_or_else(|e| {
        eprintln!("cannot open {scene}: {e}");
        std::process::exit(1);
    });
    if let Ok(name) = std::env::var("POST") {
        let Some(post) = automata_renderer::PostEffects::preset(&name) else {
            eprintln!("unknown POST look {name:?}");
            std::process::exit(1);
        };
        let ids: Vec<u64> = doc
            .entities
            .iter()
            .filter(|e| e.data.environment.is_some())
            .map(|e| e.id)
            .collect();
        for id in ids {
            doc.modify(id, false, |d| {
                if let Some(env) = &mut d.environment {
                    env.post = post;
                }
            });
        }
    }
    automata_assets::set_root(Some(&doc.base_dir()));
    let mut gpu = pollster::block_on(Renderer::new_offscreen_with_quality((w, h), quality))
        .unwrap_or_else(|e| {
            eprintln!("no GPU: {e}");
            std::process::exit(1);
        });
    let mut viewport = ViewportRenderer::new();
    if let Some(seconds) = std::env::var("PLAY")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    {
        let view = (nums.len() == 6).then_some(cam);
        let (rgba, fw, fh) = render_play(&mut gpu, &mut viewport, &doc, seconds, view, (w, h));
        image::save_buffer(out, &rgba, fw, fh, image::ColorType::Rgba8).expect("write png");
        println!("{out}: {fw}x{fh} {quality:?}, after {seconds} s of Play");
        return;
    }
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

/// Plays `doc` for `seconds` (scripts run in lockstep with physics) and
/// renders the moment: through `view` if given, else the scene's camera.
fn render_play(
    gpu: &mut Renderer,
    viewport: &mut ViewportRenderer,
    doc: &Document,
    seconds: f32,
    view: Option<ViewCamera>,
    (w, h): (u32, u32),
) -> (Vec<u8>, u32, u32) {
    use oberon::bridge::{BridgeMsg, ScriptBridge};
    use oberon::play::{PlaySession, TIMESTEP};

    let mut play = PlaySession::start(doc);
    let root = doc.base_dir();
    let has_scripts = doc.entities.iter().any(|e| e.data.script.is_some());
    let mut bridge = has_scripts
        .then(|| {
            ScriptBridge::start(
                oberon::bridge::default_python(),
                &root.join(".oberon"),
                &root,
            )
        })
        .and_then(Result::ok);
    let steps = (seconds / TIMESTEP).round() as usize;
    let shoot_at = std::env::var("SHOOT")
        .ok()
        .and_then(|s| s.parse::<f32>().ok());
    let aim = std::env::var("AIM").ok().and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some([x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?])
    });
    let held = held_keys();
    for step in 0..steps {
        if let Some(b) = bridge.as_mut() {
            // One script frame per physics step: send, then wait for it.
            let mut input = play.frame_input();
            if let Some(at) = shoot_at {
                input.screen = [w, h];
                input.mouse_locked = true;
                if play.elapsed >= at {
                    input.mouse_buttons = vec!["left"];
                }
            }
            if let (0, Some(delta)) = (step, aim) {
                input.mouse_delta = delta;
            }
            let states = play.script_states(|rel| doc.resolve(rel));
            let answered = b.answered;
            let now = play.elapsed;
            let keys: Vec<&str> = held
                .iter()
                .filter(|(_, from, to)| now >= *from && now <= *to)
                .map(|(k, _, _)| k.as_str())
                .collect();
            if b.send_frame_with(TIMESTEP, play.elapsed, &keys, &states, &input) {
                let deadline = Instant::now() + std::time::Duration::from_secs(10);
                while b.answered == answered && Instant::now() < deadline {
                    for msg in b.poll() {
                        match msg {
                            BridgeMsg::Frame(reply) => play.apply_frame(&reply),
                            BridgeMsg::Error { msg, traceback, .. } => {
                                eprintln!("script error: {msg}\n{traceback}");
                            }
                            _ => {}
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
            }
        }
        play.update(TIMESTEP);
    }
    if let Some(b) = bridge {
        drop(b.stop());
    }
    // The orbit view applies only when the scene has no camera of its own.
    let cam = view.unwrap_or_default();
    if view.is_some() {
        let world = play.scene_mut().world_mut();
        let cameras: Vec<_> = world
            .query::<(automata_scene::Entity, &automata_renderer::Camera3D)>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        for e in cameras {
            let _ = world.remove_one::<automata_renderer::Camera3D>(e);
        }
    }
    let (rgba, fw, fh) = viewport.render_play(Some(gpu), &mut play, cam, w, h);
    (rgba.to_vec(), fw, fh)
}

/// `KEYS` from the environment: each key and the Play seconds it's held
/// from and to.
fn held_keys() -> Vec<(String, f32, f32)> {
    let Ok(spec) = std::env::var("KEYS") else {
        return Vec::new();
    };
    spec.split(',')
        .filter_map(|item| {
            let (key, when) = item.trim().split_once('@')?;
            let (from, to) = if let Some((a, b)) = when.split_once('-') {
                (a.trim().parse().ok()?, b.trim().parse().ok()?)
            } else {
                let t: f32 = when.trim().parse().ok()?;
                (t, t + oberon::play::TIMESTEP * 0.5)
            };
            Some((key.trim().to_ascii_lowercase(), from, to))
        })
        .collect()
}
