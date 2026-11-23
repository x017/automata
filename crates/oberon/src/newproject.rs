//! `oberon new`: project templates.
//!
//! A new project works in any IDE from the first minute: a scene, a typed
//! starter script, a test, a `pyproject.toml` pinning the engine to this
//! Oberon's version, and settings that point VS Code, `PyCharm` and
//! basedpyright at the project's own `.venv` (made by
//! [`crate::pyenv::setup_venv`]).

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use automata_core::scene_file::Script;

use automata_core::TransformData;

use crate::doc::{CreateKind, Document};
use crate::pyenv::ENGINE_VERSION;

/// What a new project starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// A ground, a player you can walk and jump around, crates to knock
    /// over and a camera that follows.
    ThreeD,
    /// A side-on platformer: flat platforms, a player that runs and jumps
    /// left and right, and a side camera.
    TwoD,
    /// A sun, a sky and a camera; nothing else.
    Empty,
}

impl Template {
    /// Every template, in menu order.
    pub const ALL: [Self; 3] = [Self::ThreeD, Self::TwoD, Self::Empty];

    /// The command-line name (`3d`, `2d`, `empty`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::ThreeD => "3d",
            Self::TwoD => "2d",
            Self::Empty => "empty",
        }
    }

    /// Reads a command-line name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// The name a folder gets in `pyproject.toml`: lower case, with runs of
/// anything but letters and digits turned into `-`.
#[must_use]
pub fn package_name(folder: &str) -> String {
    let mut out = String::new();
    for c in folder.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "game".into()
    } else {
        out
    }
}

/// Writes a new project into `dir` (which must not exist, or be empty).
/// Returns the files written, relative to `dir`. The venv is made
/// separately ([`crate::pyenv::setup_venv`]).
///
/// # Errors
///
/// Returns why the folder can't be used or a file can't be written.
pub fn create(dir: &Path, template: Template) -> Result<Vec<PathBuf>, String> {
    if dir.exists() {
        let mut entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} already exists and isn't empty; pick a new name",
                dir.display()
            ));
        }
    }
    let folder = dir
        .file_name()
        .map_or_else(|| "game".to_string(), |n| n.to_string_lossy().into_owned());
    let title = folder.clone();
    let package = package_name(&folder);

    let mut files: Vec<(PathBuf, String)> = vec![
        ("pyproject.toml".into(), pyproject(&package)),
        ("pyrightconfig.json".into(), PYRIGHTCONFIG.into()),
        (".vscode/settings.json".into(), vscode_settings()),
        (".vscode/extensions.json".into(), VSCODE_EXTENSIONS.into()),
        (".gitignore".into(), GITIGNORE.into()),
        ("README.md".into(), readme(&title, template)),
        ("tests/test_level.py".into(), test_level(template)),
        ("models/.gitkeep".into(), String::new()),
        ("sounds/.gitkeep".into(), String::new()),
    ];
    match template {
        Template::ThreeD => files.push(("scripts/player.py".into(), PLAYER_3D.into())),
        Template::TwoD => files.push(("scripts/player.py".into(), PLAYER_2D.into())),
        Template::Empty => files.push(("scripts/.gitkeep".into(), String::new())),
    }

    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written = Vec::new();
    for (rel, text) in files {
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(rel);
    }

    let level = dir.join("level.ron");
    let mut doc = scene(template);
    doc.save(Some(&level))
        .map_err(|e| format!("{}: {e}", level.display()))?;
    written.push("level.ron".into());

    let mut cfg = automata_export::BuildConfig::for_project(dir, Some(&level));
    cfg.name = title;
    cfg.save(dir)?;
    written.push(automata_export::CONFIG_FILE.into());
    written.sort();
    Ok(written)
}

/// The template's scene.
#[must_use]
pub fn scene(template: Template) -> Document {
    match template {
        Template::ThreeD => {
            let mut doc = Document::starter();
            if let Some(i) = doc
                .entities
                .iter()
                .position(|e| e.data.name.as_deref() == Some("Player"))
            {
                doc.entities[i].data.script = Some(Script {
                    path: "scripts/player.py".into(),
                });
            }
            add_camera(&mut doc, [-3.0, 4.0, 10.0], [-3.0, 1.0, 2.0]);
            doc.selected = None;
            doc
        }
        Template::TwoD => {
            let mut doc = Document::new();
            add_sun(&mut doc);
            let platforms: [([f32; 3], [f32; 3]); 4] = [
                ([0.0, -0.5, 0.0], [30.0, 1.0, 2.0]),
                ([-4.0, 1.5, 0.0], [3.0, 0.4, 2.0]),
                ([1.0, 3.0, 0.0], [3.0, 0.4, 2.0]),
                ([6.0, 4.5, 0.0], [3.0, 0.4, 2.0]),
            ];
            for (i, (position, scale)) in platforms.into_iter().enumerate() {
                let id = doc.create(CreateKind::Cube);
                let Some(idx) = doc.index_of(id) else {
                    continue;
                };
                let data = &mut doc.entities[idx].data;
                data.name = Some(if i == 0 {
                    "Ground".into()
                } else {
                    format!("Platform {i}")
                });
                data.transform = Some(TransformData {
                    position,
                    scale,
                    ..Default::default()
                });
                if let Some(mesh) = &mut data.mesh_visual {
                    mesh.color = if i == 0 {
                        [0.30, 0.55, 0.32, 1.0]
                    } else {
                        [0.62, 0.45, 0.30, 1.0]
                    };
                }
                data.rigid_body = Some(automata_physics::RigidBody::fixed());
                data.collider_3d = Some(automata_physics::Collider3D::cuboid(
                    scale[0], scale[1], scale[2],
                ));
            }
            let player = doc.create(CreateKind::Player);
            if let Some(idx) = doc.index_of(player) {
                let data = &mut doc.entities[idx].data;
                data.name = Some("Player".into());
                if let Some(t) = &mut data.transform {
                    t.position = [-6.0, 1.0, 0.0];
                }
                data.script = Some(Script {
                    path: "scripts/player.py".into(),
                });
            }
            add_camera(&mut doc, [-6.0, 2.5, 14.0], [-6.0, 2.0, 0.0]);
            doc.selected = None;
            doc
        }
        Template::Empty => {
            let mut doc = Document::new();
            add_sun(&mut doc);
            add_camera(&mut doc, [0.0, 2.0, 6.0], [0.0, 0.5, 0.0]);
            doc.selected = None;
            doc
        }
    }
}

fn add_sun(doc: &mut Document) {
    let starter = Document::starter();
    if let Some(sun) = starter
        .entities
        .iter()
        .find(|e| e.data.name.as_deref() == Some("Sun"))
    {
        let id = doc.create(CreateKind::Empty);
        if let Some(idx) = doc.index_of(id) {
            doc.entities[idx].data = sun.data.clone();
        }
    }
}

fn add_camera(doc: &mut Document, eye: [f32; 3], target: [f32; 3]) {
    let id = doc.create(CreateKind::Camera);
    if let Some(idx) = doc.index_of(id) {
        let data = &mut doc.entities[idx].data;
        data.name = Some("Camera".into());
        if let Some(t) = &mut data.transform {
            t.position = eye;
        }
        if let Some(camera) = &mut data.camera_3d {
            camera.position = eye.into();
            camera.target = target.into();
        }
    }
}

fn pyproject(package: &str) -> String {
    format!(
        r#"[project]
name = "{package}"
version = "0.1.0"
description = "A game made with AUTOMATA."
requires-python = ">=3.10"
# The engine this project was made with. `oberon setup --upgrade` moves it
# to your Oberon's version.
dependencies = ["automata-engine=={ENGINE_VERSION}"]

[dependency-groups]
# Installed into .venv by `oberon new` / `oberon setup` when Oberon has
# them: the language server behind completions, and the test runner.
dev = ["basedpyright", "pytest"]

[tool.oberon]
# The scene `oberon run` and `oberon open` start with.
scene = "level.ron"

[tool.pytest.ini_options]
testpaths = ["tests"]
"#
    )
}

const PYRIGHTCONFIG: &str = r#"{
  "venvPath": ".",
  "venv": ".venv",
  "include": ["scripts", "tests"],
  "typeCheckingMode": "standard",
  "reportOptionalMemberAccess": "warning"
}
"#;

fn vscode_settings() -> String {
    let python = if cfg!(windows) {
        "${workspaceFolder}/.venv/Scripts/python.exe"
    } else {
        "${workspaceFolder}/.venv/bin/python"
    };
    format!(
        r#"{{
  "python.defaultInterpreterPath": "{python}",
  "python.terminal.activateEnvironment": true,
  "python.testing.pytestEnabled": true,
  "python.testing.pytestArgs": ["tests"],
  "files.exclude": {{
    "**/__pycache__": true,
    ".oberon": true
  }},
  "files.associations": {{
    "*.ron": "rust"
  }}
}}
"#
    )
}

const VSCODE_EXTENSIONS: &str = r#"{
  "recommendations": ["ms-python.python", "detachhead.basedpyright"]
}
"#;

const GITIGNORE: &str = "\
# The project's Python (made by `oberon new` / `oberon setup`)
.venv/
__pycache__/
.pytest_cache/
# Oberon's autosaves and play-mode files
.oberon/
# Exported games
build/
";

fn readme(title: &str, template: Template) -> String {
    let play = match template {
        Template::ThreeD => {
            "Walk with **WASD** or the arrow keys and jump with **Space**. \
             The player's code is in `scripts/player.py`.\n"
        }
        Template::TwoD => {
            "Run with **A**/**D** or the arrow keys and jump with **Space**. \
             The player's code is in `scripts/player.py`.\n"
        }
        Template::Empty => {
            "The scene has a sun, a sky and a camera. Add entities in Oberon \
             and scripts in `scripts/`.\n"
        }
    };
    format!(
        "# {title}\n\n\
         A game made with [AUTOMATA](https://github.com/x017/automata).\n\n\
         {play}\n\
         ## Commands\n\n\
         ```sh\n\
         oberon open            # edit level.ron in Oberon\n\
         oberon run             # play it in a window\n\
         oberon test            # run tests/ headless\n\
         oberon export --target linux   # or windows, all\n\
         oberon pip install …   # add a package to this project's Python\n\
         oberon doctor          # check the GPU, Python and editor support\n\
         ```\n\n\
         ## Editing code\n\n\
         The project has its own Python in `.venv`, with the engine and a \
         language server. Open the folder in VS Code (`code .`) or PyCharm \
         and pick `.venv` as the interpreter if it doesn't do so itself: \
         `ctx.` and `entity.` complete with docs, and typos are underlined. \
         Oberon's own code editor uses the same `.venv`.\n\n\
         After cloning the project somewhere else, run `oberon setup` to make \
         `.venv` again.\n"
    )
}

fn test_level(template: Template) -> String {
    let find = match template {
        Template::ThreeD | Template::TwoD => {
            "    assert scene.find(\"Player\") is not None\n    assert scene.find(\"Camera\") is not None\n"
        }
        Template::Empty => "    assert scene.find(\"Camera\") is not None\n",
    };
    format!(
        r#""""Tests for the level. Run them with `oberon test` (or `pytest`)."""

import os

import automata as at

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(ROOT, "level.ron")


def test_the_level_loads():
    scene = at.Scene.load(LEVEL)
{find}

def test_the_level_runs_headless():
    # Scripts run as in Play; no window opens.
    frames = at.run_scene(LEVEL, max_frames=60, headless=True, fixed_delta=1 / 60)
    assert frames == 60
"#
    )
}

const PLAYER_3D: &str = r#""""player: walks and jumps (WASD or arrows, Space).

Oberon calls `start` once and `update` every frame for each entity this
script is attached to; `oberon run` and exported games do the same. Type
`ctx.` or `entity.` for everything you can use, with docs.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # types for your editor; nothing is imported when it runs
    from automata.script_api import Context, Entity

SPEED = 5.0
JUMP = 7.0
GRAVITY = 20.0


def start(entity: Entity, ctx: Context) -> None:
    entity.state["vy"] = 0.0
    print(f"{entity.name} ready")


def update(entity: Entity, ctx: Context) -> None:
    keys = ctx.input
    x = keys.axis("a", "d") + keys.axis("arrow_left", "arrow_right")
    z = keys.axis("w", "s") + keys.axis("arrow_up", "arrow_down")
    x, z = max(-1.0, min(1.0, x)), max(-1.0, min(1.0, z))

    vy = entity.state["vy"]
    if entity.grounded:
        vy = JUMP if keys.just_pressed("space") else -2.0
    else:
        vy -= GRAVITY * ctx.time.delta
    entity.state["vy"] = vy
    entity.character_velocity = (x * SPEED, vy, z * SPEED)

    camera = ctx.camera
    if camera is not None:
        camera.follow(entity, offset=(0.0, 3.0, 8.0))
"#;

const PLAYER_2D: &str = r#""""player: runs and jumps side-on (A/D or arrows, Space).

Oberon calls `start` once and `update` every frame for each entity this
script is attached to; `oberon run` and exported games do the same. Type
`ctx.` or `entity.` for everything you can use, with docs.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # types for your editor; nothing is imported when it runs
    from automata.script_api import Context, Entity

SPEED = 6.0
JUMP = 9.0
GRAVITY = 25.0


def start(entity: Entity, ctx: Context) -> None:
    entity.state["vy"] = 0.0


def update(entity: Entity, ctx: Context) -> None:
    keys = ctx.input
    x = keys.axis("a", "d") + keys.axis("arrow_left", "arrow_right")
    x = max(-1.0, min(1.0, x))

    vy = entity.state["vy"]
    if entity.grounded:
        vy = JUMP if keys.just_pressed("space") else -2.0
    else:
        vy -= GRAVITY * ctx.time.delta
    entity.state["vy"] = vy
    # No sideways (Z) movement: the level is a flat strip.
    entity.character_velocity = (x * SPEED, vy, 0.0)

    camera = ctx.camera
    if camera is not None:
        camera.follow(entity, offset=(0.0, 1.5, 14.0), look_height=1.0)
"#;

/// The scene `oberon run` / `oberon open` start with in `root`: the
/// `[tool.oberon] scene` of `pyproject.toml`, else `level.ron`, else the
/// only `.ron` scene in the folder.
#[must_use]
pub fn main_scene(root: &Path) -> Option<PathBuf> {
    if let Some(scene) = std::fs::read_to_string(root.join("pyproject.toml"))
        .ok()
        .and_then(|text| tool_oberon_scene(&text))
    {
        let path = root.join(scene);
        if path.is_file() {
            return Some(path);
        }
    }
    let level = root.join("level.ron");
    if level.is_file() {
        return Some(level);
    }
    let scenes: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "ron")
                && !p.to_string_lossy().ends_with(".import.ron")
                && p.file_name()
                    .is_some_and(|n| n != automata_export::CONFIG_FILE)
        })
        .collect();
    match scenes.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// `scene = "…"` under `[tool.oberon]` in a `pyproject.toml`.
fn tool_oberon_scene(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[tool.oberon]";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "scene" {
                let value = value.split('#').next().unwrap_or("").trim();
                return Some(value.trim_matches(['"', '\'']).to_string()).filter(|v| !v.is_empty());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon-new-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn package_names_are_lower_case_and_dashed() {
        assert_eq!(package_name("My Game"), "my-game");
        assert_eq!(package_name("rocket__league!!2"), "rocket-league-2");
        assert_eq!(package_name("  "), "game");
    }

    #[test]
    fn every_template_writes_a_playable_project() {
        for template in Template::ALL {
            let dir = temp(template.name()).join("My Game");
            let files = create(&dir, template).unwrap();
            for f in [
                "level.ron",
                "build.ron",
                "pyproject.toml",
                "pyrightconfig.json",
                ".vscode/settings.json",
                ".gitignore",
                "README.md",
                "tests/test_level.py",
            ] {
                assert!(files.contains(&PathBuf::from(f)), "{template:?}: {f}");
                assert!(dir.join(f).is_file());
            }
            let pyproject = std::fs::read_to_string(dir.join("pyproject.toml")).unwrap();
            assert!(pyproject.contains("name = \"my-game\""));
            assert!(pyproject.contains(&format!("automata-engine=={ENGINE_VERSION}")));
            assert_eq!(main_scene(&dir), Some(dir.join("level.ron")));
            let doc = Document::load(&dir.join("level.ron")).unwrap();
            let names: Vec<&str> = doc
                .entities
                .iter()
                .filter_map(|e| e.data.name.as_deref())
                .collect();
            assert!(names.contains(&"Camera"), "{template:?}: {names:?}");
            assert!(names.contains(&"Sun"), "{template:?}: {names:?}");
            let player = doc
                .entities
                .iter()
                .find(|e| e.data.name.as_deref() == Some("Player"));
            if template == Template::Empty {
                assert!(player.is_none());
            } else {
                let script = player.unwrap().data.script.as_ref().unwrap();
                assert_eq!(script.path, "scripts/player.py");
                let code = std::fs::read_to_string(dir.join(&script.path)).unwrap();
                assert!(code.contains("def update(entity: Entity, ctx: Context) -> None:"));
            }
            // A second time is refused: the folder isn't empty any more.
            assert!(create(&dir, template).unwrap_err().contains("isn't empty"));
        }
    }

    #[test]
    fn the_main_scene_follows_pyproject_then_level_ron_then_the_only_scene() {
        let dir = temp("main");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(main_scene(&dir), None);
        std::fs::write(dir.join("arena.ron"), "").unwrap();
        std::fs::write(dir.join("build.ron"), "").unwrap();
        assert_eq!(main_scene(&dir), Some(dir.join("arena.ron")));
        std::fs::write(dir.join("level.ron"), "").unwrap();
        assert_eq!(main_scene(&dir), Some(dir.join("level.ron")));
        std::fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"x\"\n\n[tool.oberon]\nscene = \"arena.ron\"  # start here\n",
        )
        .unwrap();
        assert_eq!(main_scene(&dir), Some(dir.join("arena.ron")));
    }
}
