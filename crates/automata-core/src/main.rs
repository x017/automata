//! Minimal AUTOMATA application entry point.
//!
//! Opens a window and draws a spinning 3D cube with a floating sprite
//! overlay, driven by the scene system (the Phase 4 deliverable). Press
//! Escape or close the window to quit. Pass `--headless` to run the paced
//! loop without a window or GPU — for machines without a display.

use automata_core::{Application, EngineConfig, Result, WindowMode};
use automata_input::KeyCode;
use automata_renderer::Camera3D;
use automata_scene::{Entity, MeshRenderer, Name, Scene, SpriteRenderer, Transform};
use glam::{Quat, Vec2, Vec3};
use tracing::info;

/// Number of frames the demo runs in headless mode before quitting on its
/// own (two seconds at 60 fps). Windowed mode runs until the user quits.
const HEADLESS_DEMO_FRAMES: u64 = 120;

/// Marks entities that spin around their Y axis.
struct Spin;

/// Marks entities that bob up and down.
struct Bob {
    origin: Vec3,
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

    let mut spawned = false;

    app.run(|ctx| {
        let frame = ctx.time().frame_count();
        if frame % 60 == 0 {
            info!(
                frame,
                elapsed_seconds = ctx.time().elapsed_seconds(),
                entities = ctx.scene().entity_count(),
                "heartbeat"
            );
        }

        if ctx.input().just_pressed(KeyCode::Escape) {
            info!("escape pressed");
            ctx.request_quit();
        }

        // Spawn the demo scene once. GPU-backed render components are only
        // attached when a renderer exists (windowed mode); the scene graph
        // itself works headless.
        if !spawned {
            spawned = true;
            let scene = ctx.scene();
            scene.spawn((
                Name::new("cube"),
                Transform::from_position(Vec3::ZERO),
                Spin,
            ));
            scene.spawn((
                Name::new("camera"),
                Transform::from_position(Vec3::new(0.0, 0.5, 4.0)),
            ));
            scene.spawn((
                Name::new("sprite"),
                Transform::from_position(Vec3::new(80.0, 80.0, 0.0)),
                Bob {
                    origin: Vec3::new(80.0, 80.0, 0.0),
                },
            ));
        }

        // Attach GPU-backed components once a renderer is available.
        let gpu_parts = ctx.renderer().map(|renderer| {
            (
                renderer.create_mesh(&automata_renderer::MeshData::cube(1.5)),
                renderer.create_material([0.9, 0.6, 0.2, 1.0], None),
                renderer.white_texture(),
            )
        });
        if let Some((mesh, material, texture)) = gpu_parts {
            attach_render_components(ctx.scene(), mesh, material, texture);
        }

        // Demo behaviors: spin the cube, bob the sprite.
        let t = ctx.time().elapsed_seconds();
        spin_system(ctx.scene(), t);
        bob_system(ctx.scene(), t);

        if headless && frame >= HEADLESS_DEMO_FRAMES {
            ctx.request_quit();
        }
    })
}

/// Attaches MeshRenderer/SpriteRenderer/Camera3D components to the named
/// demo entities, once, using GPU handles created from the renderer.
fn attach_render_components(
    scene: &mut Scene,
    mesh: automata_renderer::Mesh,
    material: automata_renderer::Material,
    texture: automata_renderer::Texture,
) {
    if let Some(cube) = find_by_name(scene, "cube") {
        if scene.world().get::<&MeshRenderer>(cube).is_err() {
            let _ = scene
                .world_mut()
                .insert_one(cube, MeshRenderer::new(mesh, material));
        }
    }

    if let Some(camera) = find_by_name(scene, "camera") {
        if scene.world().get::<&Camera3D>(camera).is_err() {
            let _ = scene
                .world_mut()
                .insert_one(camera, Camera3D::new(Vec3::new(0.0, 0.5, 4.0), Vec3::ZERO));
        }
    }

    if let Some(sprite) = find_by_name(scene, "sprite") {
        if scene.world().get::<&SpriteRenderer>(sprite).is_err() {
            let _ = scene.world_mut().insert_one(
                sprite,
                SpriteRenderer::new(texture)
                    .size(Vec2::new(120.0, 120.0))
                    .color([0.2, 0.8, 1.0, 0.8]),
            );
        }
    }
}

/// Finds the first entity with the given name, if any.
fn find_by_name(scene: &mut Scene, name: &str) -> Option<Entity> {
    scene
        .world_mut()
        .query::<(Entity, &Name)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
}

/// Rotates every `Spin` entity around Y and X.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn spin_system(scene: &mut Scene, elapsed_seconds: f64) {
    let t = elapsed_seconds as f32;
    for (_spin, transform) in scene.world_mut().query_mut::<(&Spin, &mut Transform)>() {
        transform.rotation = Quat::from_rotation_y(t) * Quat::from_rotation_x(t * 0.7);
    }
}

/// Bobs every `Bob` entity up and down around its origin.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn bob_system(scene: &mut Scene, elapsed_seconds: f64) {
    let t = elapsed_seconds as f32;
    for (bob, transform) in scene.world_mut().query_mut::<(&Bob, &mut Transform)>() {
        transform.position = bob.origin + Vec3::new(0.0, (t * 2.0).sin() * 20.0, 0.0);
    }
}
