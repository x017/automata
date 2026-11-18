//! Minimal AUTOMATA application entry point.
//!
//! Opens a window and draws a spinning 3D cube with a colored sprite
//! overlay (the Phase 3 deliverable). Press Escape or close the window to
//! quit. Pass `--headless` to run the paced loop without a window or GPU —
//! for machines without a display.

use automata_core::{Application, EngineConfig, Result, WindowMode};
use automata_input::KeyCode;
use automata_renderer::{Camera3D, Material, Mesh, MeshData, Sprite};
use glam::{Mat4, Quat, Vec2, Vec3};
use tracing::info;

/// Number of frames the demo runs in headless mode before quitting on its
/// own (two seconds at 60 fps). Windowed mode runs until the user quits.
const HEADLESS_DEMO_FRAMES: u64 = 120;

/// Demo spin angle in radians from elapsed seconds.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn spin_angle(elapsed_seconds: f64) -> f32 {
    elapsed_seconds as f32
}

fn main() -> Result<()> {
    let headless = std::env::args().any(|arg| arg == "--headless");
    let window_mode = if headless {
        WindowMode::Headless
    } else {
        WindowMode::default()
    };

    let mut app = Application::new(EngineConfig {
        window_mode,
        ..EngineConfig::new("AUTOMATA")
    });

    // GPU resources created lazily on the first frame that has a renderer.
    let mut cube: Option<Mesh> = None;
    let mut material: Option<Material> = None;
    let mut sprite_tex = None;

    app.run(|ctx| {
        let frame = ctx.time().frame_count();
        if frame % 60 == 0 {
            info!(
                frame,
                elapsed_seconds = ctx.time().elapsed_seconds(),
                "heartbeat"
            );
        }

        if ctx.input().just_pressed(KeyCode::Escape) {
            info!("escape pressed");
            ctx.request_quit();
        }

        // Create GPU resources once, lazily.
        if let Some(renderer) = ctx.renderer() {
            if cube.is_none() {
                cube = Some(renderer.create_mesh(&MeshData::cube(1.5)));
                material = Some(renderer.create_material([0.9, 0.6, 0.2, 1.0], None));
                sprite_tex = Some(renderer.white_texture());
                info!("created demo meshes, materials, and textures");
            }

            // Draw a spinning cube.
            let model = Mat4::from_scale_rotation_translation(
                Vec3::ONE,
                Quat::from_rotation_y(spin_angle(ctx.time().elapsed_seconds()))
                    * Quat::from_rotation_x(spin_angle(ctx.time().elapsed_seconds()) * 0.7),
                Vec3::ZERO,
            );
            ctx.draw().set_camera_3d(Camera3D::new(
                Vec3::new(0.0, 0.5, 4.0),
                Vec3::new(0.0, 0.0, 0.0),
            ));
            if let (Some(mesh), Some(mat)) = (&cube, &material) {
                ctx.draw().mesh(mesh, model, mat);
            }

            // Draw a small sprite in the top-left corner.
            if let Some(tex) = &sprite_tex {
                ctx.draw().sprite(
                    Sprite::new(tex.clone())
                        .position(Vec2::new(80.0, 80.0))
                        .size(Vec2::new(120.0, 120.0))
                        .color([0.2, 0.8, 1.0, 0.8]),
                );
            }
        }

        if headless && frame >= HEADLESS_DEMO_FRAMES {
            ctx.request_quit();
        }
    })
}
