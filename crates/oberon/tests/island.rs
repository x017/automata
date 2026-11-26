//! The Terrain sample (`examples/oberon/island/level.ron`), made the way a
//! person makes it in Oberon: Create ▸ Terrain, then strokes with the
//! Inspector's brushes (Raise, Lower, Smooth, Flatten, Noise, the layer
//! paints), Auto Paint, and Grass painted with the foliage brush. Every
//! edit below goes through the same `Document` calls the editor's
//! Inspector, brush and menus make — nothing is set on the terrain
//! directly.
//!
//! The test rebuilds the island and checks the committed scene still
//! matches. To rewrite it (after changing the strokes or the brushes):
//!
//! ```sh
//! OBERON_WRITE_ISLAND=1 cargo test -p oberon --no-default-features --test island
//! ```

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines // `build` is the island, stroke by stroke
)]

use std::path::PathBuf;

use glam::Vec2;
use oberon::doc::{BrushTool, CreateKind, Document};
use oberon::fields::{self, Field, Flag};

/// The water's height (the island's shore is where the ground meets it).
const SEA: f32 = 1.5;
const BODY_PASSES: u32 = 1;

fn level_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/oberon/island/level.ron")
}

fn set(doc: &mut Document, id: u64, field: Field, value: f32) {
    doc.modify(id, false, |d| {
        fields::set(d, field, value);
    });
}

/// The ground's world height at `(x, z)` (the terrain sits at the origin).
fn ground(doc: &Document, terrain: u64, x: f32, z: f32) -> f32 {
    doc.entities[doc.index_of(terrain).unwrap()]
        .data
        .terrain
        .as_ref()
        .unwrap()
        .height_at(x, z)
}

/// One drag along `path` with `tool`, laying dabs as the brush does: every
/// `0.2 × radius` metres for terrain tools, `0.3 ×` for foliage, each on
/// the ground under the cursor. One undo step.
fn stroke(
    doc: &mut Document,
    (target, terrain): (u64, u64),
    tool: BrushTool,
    path: &[(f32, f32)],
    radius: f32,
    strength: f32,
) {
    let spacing = radius * if tool.is_terrain() { 0.2 } else { 0.3 };
    let at = |doc: &Document, p: Vec2| [p.x, ground(doc, terrain, p.x, p.y), p.y];
    let first = Vec2::from(path[0]);
    let anchor = at(doc, first);
    let mut dabs = vec![first];
    for pair in path.windows(2) {
        let (a, b) = (Vec2::from(pair[0]), Vec2::from(pair[1]));
        let steps = (a.distance(b) / spacing).ceil().max(1.0) as u32;
        dabs.extend((1..=steps).map(|i| a.lerp(b, i as f32 / steps as f32)));
    }
    for p in dabs {
        let point = at(doc, p);
        doc.brush(
            target,
            tool,
            point,
            [0.0, 1.0, 0.0],
            radius,
            strength,
            anchor,
        );
    }
    doc.end_gesture();
}

/// A closed loop of `n` points round `(cx, cz)`.
fn ring(cx: f32, cz: f32, rx: f32, rz: f32, n: u32) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + rx * t.cos(), cz + rz * t.sin())
        })
        .collect()
}

/// Builds the island scene from scratch.
fn build() -> Document {
    let mut doc = Document::new();

    // --- The ground: a 128 m terrain, one height sample a metre. --------
    let island = doc.create(CreateKind::Terrain);
    doc.rename_entity(island, "Island");
    set(&mut doc, island, Field::TerrainSize, 128.0);
    let t = (island, island);

    // The body of the island: big soft Raise strokes back and forth,
    // then a Smooth over the lot.
    let scribble: Vec<(f32, f32)> = [-14.0f32, -4.0, 6.0, 14.0]
        .iter()
        .enumerate()
        .flat_map(|(k, &z)| {
            let (a, b) = if k % 2 == 0 {
                (-24.0, 22.0)
            } else {
                (22.0, -24.0)
            };
            [(a, z), (b, z)]
        })
        .collect();
    for _ in 0..BODY_PASSES {
        stroke(&mut doc, t, BrushTool::Raise, &scribble, 20.0, 0.6);
    }
    for _ in 0..2 {
        stroke(&mut doc, t, BrushTool::Smooth, &scribble, 20.0, 1.0);
    }

    // Grassy hills.
    for &(x, z, passes) in &[(-14.0, -8.0, 12), (8.0, -12.0, 9), (-4.0, 8.0, 5)] {
        for _ in 0..passes {
            stroke(
                &mut doc,
                t,
                BrushTool::Raise,
                &[(x, z), (x + 4.0, z + 2.0)],
                16.0,
                0.6,
            );
        }
    }
    stroke(
        &mut doc,
        t,
        BrushTool::Smooth,
        &[(-16.0, -10.0), (10.0, -12.0)],
        20.0,
        0.5,
    );

    // Cliffs: a headland on the north-east, raised hard with a small brush
    // right where the land drops into the sea.
    let headland = [(18.0, -22.0), (25.0, -16.0), (30.0, -8.0), (31.0, 0.0)];
    for _ in 0..6 {
        stroke(&mut doc, t, BrushTool::Raise, &headland, 5.0, 1.0);
    }
    // Lower the sea side away so the face stands sheer.
    stroke(
        &mut doc,
        t,
        BrushTool::Lower,
        &[(24.0, -28.0), (33.0, -19.0), (39.0, -8.0), (40.0, 2.0)],
        5.0,
        1.0,
    );
    stroke(&mut doc, t, BrushTool::Noise, &headland, 5.0, 1.0);

    // A cove and a wide beach on the south: lowered, then flattened just
    // above the water.
    stroke(
        &mut doc,
        t,
        BrushTool::Lower,
        &[(-6.0, 26.0), (6.0, 27.0)],
        10.0,
        1.0,
    );
    let beach = [(-22.0, 22.0), (-10.0, 27.0), (4.0, 28.0), (16.0, 22.0)];
    // Start the stroke on ground 0.6 m above the sea: what Flatten levels to.
    let start = (0..160)
        .map(|k| (2.0, k as f32 * 0.25))
        .find(|&(x, z)| ground(&doc, island, x, z) < SEA + 0.6)
        .unwrap();
    let mut path = vec![start];
    path.extend_from_slice(&beach);
    for _ in 0..3 {
        stroke(&mut doc, t, BrushTool::Flatten, &path, 10.0, 1.0);
    }
    stroke(&mut doc, t, BrushTool::Smooth, &beach, 10.0, 1.0);

    // Some roughness over the land so it doesn't look moulded.
    stroke(
        &mut doc,
        t,
        BrushTool::Noise,
        &ring(-3.0, -2.0, 18.0, 14.0, 8),
        10.0,
        0.5,
    );

    // --- The sea, the sky and the sun. -----------------------------------
    let sea = doc.create(CreateKind::Water);
    doc.rename_entity(sea, "Sea");
    doc.modify(sea, false, |d| {
        d.transform.as_mut().unwrap().position[1] = SEA;
    });
    set(&mut doc, sea, Field::WaterSize(0), 400.0);
    set(&mut doc, sea, Field::WaterSize(1), 400.0);
    set(&mut doc, sea, Field::WaterWaveHeight, 0.12);
    set(&mut doc, sea, Field::WaterWaveLength, 6.0);
    doc.modify(sea, false, |d| {
        fields::toggle(d, Flag::WaterEndless);
    });
    let env = doc.create(CreateKind::Environment);
    set(&mut doc, env, Field::EnvWind, 0.4);
    doc.create(CreateKind::Light);

    // --- Paint: by shape first (Auto Paint reads the Sea's height), then
    // a dirt path up from the beach by hand. ------------------------------
    assert!(doc.auto_paint_terrain(island, 1.2, 36.0));
    stroke(
        &mut doc,
        t,
        BrushTool::Layer(3),
        &[
            (0.0, 22.0),
            (-3.0, 14.0),
            (-8.0, 6.0),
            (-12.0, -2.0),
            (-14.0, -7.0),
        ],
        2.0,
        1.0,
    );

    // --- Grass: a field painted over the hills with the foliage brush; it
    // stands on the terrain wherever it's painted. -----------------------
    let meadow = doc.create(CreateKind::Grass);
    doc.rename_entity(meadow, "Meadow");
    set(&mut doc, meadow, Field::GrassArea(0), 0.0);
    set(&mut doc, meadow, Field::GrassArea(1), 0.0);
    set(&mut doc, meadow, Field::GrassDensity, 24.0);
    set(&mut doc, meadow, Field::GrassDrawDistance, 70.0);
    let g = (meadow, island);
    for row in [-14.0f32, -8.0, -2.0, 4.0, 10.0] {
        let path: Vec<(f32, f32)> = (-5..=4).map(|i| (i as f32 * 4.0, row)).collect();
        stroke(&mut doc, g, BrushTool::Paint, &path, 6.0, 1.0);
    }
    // Keep it off the beach, the cliffs and the path.
    stroke(
        &mut doc,
        g,
        BrushTool::Erase,
        &[(-16.0, 16.0), (16.0, 16.0)],
        6.0,
        1.0,
    );
    stroke(&mut doc, g, BrushTool::Erase, &headland, 6.0, 1.0);
    stroke(
        &mut doc,
        g,
        BrushTool::Erase,
        &[(-3.0, 14.0), (-8.0, 6.0), (-12.0, -2.0)],
        1.5,
        1.0,
    );

    // --- Someone to walk it, and a view. ---------------------------------
    let player = doc.create(CreateKind::Player);
    let (px, pz) = (0.0, 18.0);
    let py = ground(&doc, island, px, pz) + 1.0;
    doc.modify(player, false, |d| {
        d.transform.as_mut().unwrap().position = [px, py, pz];
    });
    doc.create_camera_at(automata_renderer::Camera3D::new(
        glam::Vec3::new(46.0, 30.0, 62.0),
        glam::Vec3::new(0.0, 3.0, 0.0),
    ));
    doc.select(None);
    doc
}

#[test]
fn the_island_is_sculpted_and_painted_with_the_brushes() {
    let mut doc = build();
    let path = level_path();
    if std::env::var_os("OBERON_WRITE_ISLAND").is_some() {
        doc.save(Some(&path)).expect("write the island");
    }
    let terrain = |doc: &Document| {
        let i = doc
            .entities
            .iter()
            .position(|e| e.data.terrain.is_some())
            .unwrap();
        doc.entities[i].data.terrain.clone().unwrap()
    };
    let built = terrain(&doc);

    let h = |x, z| built.height_at(x, z);
    // It is an island: land in the middle, sea all round.
    assert!(
        h(0.0, 0.0) > SEA + 2.0,
        "land in the middle: {}",
        h(0.0, 0.0)
    );
    for (x, z) in [(-60.0, 0.0), (60.0, 0.0), (0.0, -60.0), (0.0, 60.0)] {
        assert!(h(x, z) < SEA, "sea at ({x}, {z})");
    }
    // Hills above the plateau, cliffs on the headland, a beach in the south.
    assert!(
        h(-12.0, -7.0) > h(0.0, 0.0) + 3.0,
        "a hill: {}",
        h(-12.0, -7.0)
    );
    let steepest = (0..40)
        .map(|i| {
            let p = Vec2::new(18.0, -22.0).lerp(Vec2::new(40.0, 2.0), i as f32 / 40.0);
            built.normal_at(p.x, p.y).y.acos().to_degrees()
        })
        .fold(0.0f32, f32::max);
    assert!(steepest > 45.0, "sheer cliffs: {steepest:.0}°");
    let beach = built.weights_at(0.0, 25.0);
    assert!(beach[2] > 0.6, "sand on the beach: {beach:?}");
    let hill = built.weights_at(-12.0, -7.0);
    assert!(hill[0] > 0.6, "grass on the hills: {hill:?}");
    let cliff = (0..40)
        .map(|i| {
            let p = Vec2::new(18.0, -22.0).lerp(Vec2::new(40.0, 2.0), i as f32 / 40.0);
            built.weights_at(p.x, p.y)[1]
        })
        .fold(0.0f32, f32::max);
    assert!(cliff > 0.6, "rock on the cliffs: {cliff}");

    // The committed scene is this island.
    let saved = Document::load(&path).expect("examples/oberon/island/level.ron");
    let committed = terrain(&saved);
    assert_eq!(committed.resolution(), built.resolution());
    let (lo, hi) = built.height_range();
    let step = (hi - lo) / 65535.0 + 1e-4;
    let drift = committed
        .heights()
        .iter()
        .zip(built.heights())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        drift <= step * 2.0,
        "level.ron is stale (run with OBERON_WRITE_ISLAND=1): {drift}"
    );
    let paint = committed
        .splat()
        .iter()
        .zip(built.splat())
        .flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)))
        .max()
        .unwrap_or(0);
    assert!(paint <= 2, "level.ron's paint is stale: {paint}");
    assert_eq!(saved.entities.len(), doc.entities.len());
}
