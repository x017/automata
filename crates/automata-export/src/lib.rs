//! Exports AUTOMATA games to standalone builds for Windows and Linux.
//!
//! An export copies an **export template** (the `automata-player` launcher
//! plus a portable Python with the engine installed) and adds the project:
//!
//! ```text
//! build/linux/Coin Run/
//!     Coin Run            the launcher, renamed
//!     game/               scenes, scripts, assets, game.json
//!     runtime/            Python + the engine
//! build/Coin Run-0.1.0-linux-x86_64.tar.gz
//! ```
//!
//! Templates are built per platform by CI (`.github/workflows/export.yml`)
//! and installed into [`templates_dir`]. Because a template already holds
//! the compiled engine, exporting needs no compiler: any machine can export
//! for any supported platform.
//!
//! Project settings live in `build.ron` next to the project's scenes
//! ([`BuildConfig`]).

pub mod archive;
pub mod cli;

use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The engine version templates must match.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The repository that publishes the templates (`owner/name`).
pub const TEMPLATE_REPO: &str = "x017/automata";
/// The release tag holding the templates.
pub const TEMPLATE_TAG: &str = "export-templates";
/// Where CI publishes templates (`<base>/automata-template-<target>.tar.gz`).
pub const TEMPLATE_URL: &str =
    "https://github.com/x017/automata/releases/download/export-templates";

/// The settings file inside a project.
pub const CONFIG_FILE: &str = "build.ron";

/// A platform a game can be exported to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    /// 64-bit Linux (`x86_64`, glibc).
    Linux,
    /// 64-bit Windows 10+ (`x86_64`).
    Windows,
}

impl Target {
    /// Every supported target.
    pub const ALL: [Target; 2] = [Target::Linux, Target::Windows];

    /// Short name (`linux`, `windows`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Target::Linux => "linux",
            Target::Windows => "windows",
        }
    }

    /// Display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Target::Linux => "Linux",
            Target::Windows => "Windows",
        }
    }

    /// Platform with architecture (`linux-x86_64`).
    #[must_use]
    pub fn triple(self) -> &'static str {
        match self {
            Target::Linux => "linux-x86_64",
            Target::Windows => "windows-x86_64",
        }
    }

    /// The launcher's file name inside a template.
    #[must_use]
    pub fn player_file(self) -> &'static str {
        match self {
            Target::Linux => "automata-player",
            Target::Windows => "automata-player.exe",
        }
    }

    /// The executable suffix.
    #[must_use]
    pub fn exe_suffix(self) -> &'static str {
        match self {
            Target::Linux => "",
            Target::Windows => ".exe",
        }
    }

    /// The archive extension for exported builds.
    #[must_use]
    pub fn archive_ext(self) -> &'static str {
        match self {
            Target::Linux => "tar.gz",
            Target::Windows => "zip",
        }
    }

    /// The target this program runs on, if supported.
    #[must_use]
    pub fn host() -> Option<Target> {
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Target::Linux)
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Target::Windows)
        } else {
            None
        }
    }

    /// Parses `linux` / `windows`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Target> {
        Target::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(s.trim()))
    }

    /// The template archive name (`automata-template-linux-x86_64.tar.gz`).
    #[must_use]
    pub fn template_archive(self) -> String {
        format!("automata-template-{}.tar.gz", self.triple())
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A project's export settings (`build.ron`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    /// The game's name: the window title, executable, and folder name.
    pub name: String,
    /// Version, used in archive names.
    pub version: String,
    /// The scene the game starts in (relative to the project), for Oberon
    /// projects.
    pub start_scene: Option<String>,
    /// A Python script to run instead of a scene (code-first games).
    pub main_script: Option<String>,
    /// Window size in pixels.
    pub resolution: (u32, u32),
    /// Platforms to export.
    pub targets: Vec<Target>,
    /// Output folder (relative to the project).
    pub output: String,
    /// Also write a `.tar.gz` / `.zip` of each build.
    pub archive: bool,
    /// Extra paths to leave out (relative to the project; a trailing `/`
    /// means a folder). Build output, `.oberon/`, caches and VCS folders are
    /// always left out.
    pub exclude: Vec<String>,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            name: "My Game".into(),
            version: "0.1.0".into(),
            start_scene: None,
            main_script: None,
            resolution: (1280, 720),
            targets: vec![Target::Linux, Target::Windows],
            output: "build".into(),
            archive: true,
            exclude: vec!["tests/".into()],
        }
    }
}

impl BuildConfig {
    /// Settings for a project: `build.ron` if present, otherwise defaults
    /// named after the folder, starting in `scene` (when given).
    #[must_use]
    pub fn for_project(project: &Path, scene: Option<&Path>) -> Self {
        if let Ok(cfg) = Self::load(project) {
            return cfg;
        }
        let mut cfg = Self::default();
        if let Some(name) = project.file_name().and_then(|n| n.to_str()) {
            cfg.name = title_case(name);
        }
        cfg.start_scene = scene
            .and_then(|s| {
                s.strip_prefix(project)
                    .ok()
                    .map(Path::to_path_buf)
                    .or(Some(s.to_path_buf()))
            })
            .map(|p| archive::slash_path(&p));
        cfg
    }

    /// Reads `build.ron` from `project`.
    ///
    /// # Errors
    ///
    /// When the file is missing or invalid.
    pub fn load(project: &Path) -> Result<Self, String> {
        let path = project.join(CONFIG_FILE);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Writes `build.ron` into `project`.
    ///
    /// # Errors
    ///
    /// When the file can't be written.
    pub fn save(&self, project: &Path) -> Result<(), String> {
        let pretty = ron::ser::PrettyConfig::new().struct_names(true);
        let text = ron::ser::to_string_pretty(self, pretty).map_err(|e| e.to_string())?;
        let path = project.join(CONFIG_FILE);
        std::fs::write(&path, format!("// Export settings for automata-export and Oberon (Build → Build Settings).\n{text}\n"))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The executable / folder name: the game name without characters
    /// file systems reject.
    #[must_use]
    pub fn file_name(&self) -> String {
        let clean: String = self
            .name
            .chars()
            .filter(|c| {
                !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                    && !c.is_control()
            })
            .collect();
        let clean = clean.trim().trim_end_matches('.').to_string();
        if clean.is_empty() {
            "Game".into()
        } else {
            clean
        }
    }
}

fn title_case(s: &str) -> String {
    s.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_uppercase().collect::<String>() + c.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How serious a check result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Progress.
    Info,
    /// Suspicious but exportable.
    Warning,
    /// The export can't continue.
    Error,
}

/// A progress or problem message from an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// How serious.
    pub level: Level,
    /// The text.
    pub text: String,
}

impl Message {
    fn info(text: impl Into<String>) -> Self {
        Self {
            level: Level::Info,
            text: text.into(),
        }
    }
    fn warn(text: impl Into<String>) -> Self {
        Self {
            level: Level::Warning,
            text: text.into(),
        }
    }
    fn error(text: impl Into<String>) -> Self {
        Self {
            level: Level::Error,
            text: text.into(),
        }
    }
}

/// Checks a project before exporting: the start scene loads, every script,
/// model, material, sound and font it names exists inside the project
/// (only the project folder ships), and a start point is set.
#[must_use]
pub fn validate(project: &Path, cfg: &BuildConfig) -> Vec<Message> {
    let mut out = Vec::new();
    if cfg.name.trim().is_empty() {
        out.push(Message::error("The game has no name (build.ron: name)."));
    }
    match (&cfg.main_script, &cfg.start_scene) {
        (Some(main), _) => {
            let path = project.join(main);
            if !path.is_file() {
                out.push(Message::error(format!("Main script not found: {main}")));
            }
        }
        (None, Some(scene)) => {
            let path = project.join(scene);
            match std::fs::read_to_string(&path) {
                Err(e) => out.push(Message::error(format!("Start scene {scene}: {e}"))),
                Ok(text) => match automata_core::scene_file::SceneFile::from_ron(&text) {
                    Err(e) => out.push(Message::error(format!(
                        "Start scene {scene} doesn't load: {e}"
                    ))),
                    Ok(file) => {
                        let base = path.parent().unwrap_or(project);
                        for entity in &file.entities {
                            if let Some(script) = &entity.script {
                                if !base.join(&script.path).is_file() {
                                    out.push(Message::error(format!(
                                        "{}: script {} not found",
                                        entity.name.as_deref().unwrap_or("an entity"),
                                        script.path
                                    )));
                                }
                            }
                            if let Some(automata_core::scene_file::PrimitiveData::Model { path }) =
                                entity.mesh_visual.as_ref().map(|m| &m.primitive)
                            {
                                let who = entity.name.as_deref().unwrap_or("an entity");
                                check_model(project, base, who, path, &mut out);
                            }
                            if let Some(material) = entity
                                .mesh_visual
                                .as_ref()
                                .map(|m| m.material.trim())
                                .filter(|m| !m.is_empty())
                            {
                                let who = entity.name.as_deref().unwrap_or("an entity");
                                check_material(project, base, who, material, &mut out);
                            }
                            if let Some(source) = &entity.audio_source {
                                let who = entity.name.as_deref().unwrap_or("an entity");
                                check_sound(project, base, who, &source.clip, &mut out);
                            }
                            if let Some(collider) = &entity.collider_3d {
                                let who = entity.name.as_deref().unwrap_or("an entity");
                                let body = entity.rigid_body.as_ref().map(|b| b.body_type);
                                if let Some(problem) =
                                    automata_physics::collider_problem(&collider.shape, body)
                                {
                                    out.push(Message::error(format!("{who}: {problem}")));
                                } else if let Some(model) = collider.shape.model() {
                                    check_model(project, base, who, model, &mut out);
                                }
                            }
                            if let Some(joint) = &entity.joint {
                                check_joint(&file, entity, joint, &mut out);
                            }
                        }
                    }
                },
            }
        }
        (None, None) => out.push(Message::error(
            "Nothing to start: set a start scene or a main script (Build Settings).",
        )),
    }
    // Scripts name fonts relative to the scene's folder.
    let base = match (&cfg.main_script, &cfg.start_scene) {
        (None, Some(scene)) => project
            .join(scene)
            .parent()
            .map_or_else(|| project.to_path_buf(), Path::to_path_buf),
        _ => project.to_path_buf(),
    };
    check_fonts(project, &base, &Skip::new(cfg), &mut out);
    check_strings(project, &base, &Skip::new(cfg), &mut out);
    if cfg.targets.is_empty() {
        out.push(Message::error("No target platforms selected."));
    }
    out
}

/// Checks the fonts scripts name: every string literal ending in `.ttf` or
/// `.otf` in a shipped `.py` file must be a font file inside the project
/// that ships. (A game with a missing font still runs, drawing with the
/// built-in one, which is easy to miss.)
fn check_fonts(project: &Path, base: &Path, skip: &Skip, out: &mut Vec<Message>) {
    let mut seen = std::collections::HashSet::new();
    for (script, line, font) in font_literals(project, skip) {
        if seen.insert(font.clone()) {
            check_font(project, base, skip, &format!("{script}:{line}"), &font, out);
        }
    }
}

/// Checks one font a game names (`who` says where): it must be a font file
/// inside the project that ships.
fn check_font(
    project: &Path,
    base: &Path,
    skip: &Skip,
    who: &str,
    font: &str,
    out: &mut Vec<Message>,
) {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let file = if Path::new(font).is_absolute() {
        PathBuf::from(font)
    } else {
        automata_assets::join_relative(base, font)
    };
    if !file.is_file() {
        out.push(Message::error(format!(
            "{who}: font {font} not found (fonts are named relative to the scene's \
             folder, e.g. fonts/Pixel.ttf)"
        )));
        return;
    }
    let shipped = canon(&file)
        .strip_prefix(canon(project))
        .ok()
        .map(archive::slash_path);
    match shipped {
        None => out.push(Message::error(format!(
            "{who}: font {font} is outside the project folder and won't ship \
             (drop it into the editor to import it into fonts/)"
        ))),
        Some(rel) if skip.excluded(&rel) => out.push(Message::error(format!(
            "{who}: font {font} is excluded from the build (build.ron: exclude)"
        ))),
        Some(_) if !is_font_file(&file) => out.push(Message::error(format!(
            "{who}: {font} isn't a TrueType or OpenType font"
        ))),
        Some(_) => {}
    }
}

/// Checks the string tables (`lang/*.csv`): they ship, their language
/// fonts are fonts that ship, and mistakes in them are warned about, with
/// the keys a language lacks (those show in the first language).
fn check_strings(project: &Path, base: &Path, skip: &Skip, out: &mut Vec<Message>) {
    let table = automata_core::StringTable::load(base);
    if table.files.is_empty() {
        return;
    }
    for file in &table.files {
        let rel = std::fs::canonicalize(base.join(file))
            .ok()
            .and_then(|p| {
                let root = std::fs::canonicalize(project).ok()?;
                p.strip_prefix(root).ok().map(archive::slash_path)
            })
            .unwrap_or_else(|| file.clone());
        if skip.excluded(&rel) {
            out.push(Message::error(format!(
                "{file} is excluded from the build (build.ron: exclude), so ctx.tr would \
                 show keys instead of text"
            )));
        }
    }
    for problem in &table.problems {
        out.push(Message::warn(problem.clone()));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, font) in table.fonts.iter().enumerate() {
        if !font.is_empty() && seen.insert(font.clone()) {
            let who = format!("lang/*.csv @font ({})", table.languages[i]);
            check_font(project, base, skip, &who, font, out);
        }
    }
    for (language, keys) in table.untranslated() {
        let shown: Vec<&str> = keys.iter().take(4).map(String::as_str).collect();
        let more = if keys.len() > shown.len() {
            ", …"
        } else {
            ""
        };
        out.push(Message::warn(format!(
            "lang/*.csv: {} key{} without {language} text ({}{more}); they show in {}",
            keys.len(),
            if keys.len() == 1 { "" } else { "s" },
            shown.join(", "),
            table.languages[0]
        )));
    }
}

/// Whether `path` starts like a TrueType / OpenType font (or collection).
fn is_font_file(path: &Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok()
        && matches!(&head, [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf")
}

/// Every `.ttf` / `.otf` string literal in the project's shipped `.py`
/// files: (script, line, literal), in a stable order.
fn font_literals(project: &Path, skip: &Skip) -> Vec<(String, usize, String)> {
    let mut scripts = Vec::new();
    let mut stack = vec![project.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(items) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in items.flatten() {
            let path = item.path();
            let rel = archive::slash_path(path.strip_prefix(project).unwrap_or(&path));
            if skip.excluded(&rel) || scripts.len() > 5000 {
                continue;
            }
            match item.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(_)
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("py")) =>
                {
                    scripts.push((rel, path));
                }
                _ => {}
            }
        }
    }
    scripts.sort();
    let mut found = Vec::new();
    for (rel, path) in scripts {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for literal in string_literals(line) {
                let font = Path::new(&literal).extension().is_some_and(|e| {
                    e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf")
                });
                // f-strings and format templates can't be checked.
                if font && !literal.contains('{') {
                    found.push((rel.clone(), i + 1, literal.replace('\\', "/")));
                }
            }
        }
    }
    found
}

/// The single-line string literals of one line of Python (comments
/// skipped; escapes kept as written, which is fine for paths).
fn string_literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '#' => break,
            '"' | '\'' => {
                let mut text = String::new();
                let mut closed = false;
                while let Some(d) = chars.next() {
                    if d == '\\' {
                        text.push(d);
                        if let Some(e) = chars.next() {
                            text.push(e);
                        }
                    } else if d == c {
                        closed = true;
                        break;
                    } else {
                        text.push(d);
                    }
                }
                if closed {
                    out.push(text);
                }
            }
            _ => {}
        }
    }
    out
}

/// Warns about a joint that can't hold anything: no body of its own, or a
/// partner name no entity has (a script might spawn it, so not an error).
fn check_joint(
    file: &automata_core::scene_file::SceneFile,
    entity: &automata_core::scene_file::EntityData,
    joint: &automata_physics::Joint,
    out: &mut Vec<Message>,
) {
    let who = entity.name.as_deref().unwrap_or("an entity");
    if entity.rigid_body.is_none() {
        out.push(Message::warn(format!(
            "{who}: its Joint needs a Rigidbody on the same entity"
        )));
    }
    if let Some(partner) = &joint.connected {
        let found = file
            .entities
            .iter()
            .any(|e| e.name.as_deref() == Some(partner.as_str()) && !std::ptr::eq(e, entity));
        if !found {
            out.push(Message::warn(format!(
                "{who}: its Joint is connected to {partner:?}, but no entity has that name"
            )));
        }
    }
}

/// Checks one model reference: the file and the files it uses (buffers,
/// textures, `.mtl`) exist and sit inside the project, so they ship.
fn check_model(project: &Path, base: &Path, who: &str, rel: &str, out: &mut Vec<Message>) {
    let file = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        automata_assets::join_relative(base, rel)
    };
    if !file.is_file() {
        out.push(Message::error(format!("{who}: model {rel} not found")));
        return;
    }
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root = canon(project);
    if !canon(&file).starts_with(&root) {
        out.push(Message::error(format!(
            "{who}: model {rel} is outside the project folder and won't ship \
             (drop it into the editor to import it into models/)"
        )));
        return;
    }
    for dep in automata_assets::dependencies(&file) {
        let shown = dep
            .strip_prefix(&root)
            .unwrap_or(&dep)
            .display()
            .to_string();
        if !dep.is_file() {
            out.push(Message::warn(format!(
                "{who}: model {rel} uses {shown}, which is missing"
            )));
        } else if !canon(&dep).starts_with(&root) {
            out.push(Message::warn(format!(
                "{who}: model {rel} uses {shown}, outside the project; it won't ship"
            )));
        }
    }
}

/// Checks a material reference: the file exists inside the project and
/// reads, and the textures it names exist inside the project too.
fn check_material(project: &Path, base: &Path, who: &str, rel: &str, out: &mut Vec<Message>) {
    let file = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        automata_assets::join_relative(base, rel)
    };
    let Ok(text) = std::fs::read_to_string(&file) else {
        out.push(Message::error(format!("{who}: material {rel} not found")));
        return;
    };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root = canon(project);
    if !canon(&file).starts_with(&root) {
        out.push(Message::error(format!(
            "{who}: material {rel} is outside the project folder and won't ship"
        )));
        return;
    }
    if let Err(e) = automata_assets::MaterialFile::parse(&text) {
        out.push(Message::error(format!(
            "{who}: material {rel} doesn't read: {e}"
        )));
        return;
    }
    for dep in automata_assets::material::material_dependencies(&file) {
        let shown = canon(&dep);
        let shown = shown
            .strip_prefix(&root)
            .unwrap_or(&dep)
            .display()
            .to_string()
            .replace('\\', "/");
        if !dep.is_file() {
            out.push(Message::warn(format!(
                "{who}: material {rel} uses {shown}, which is missing"
            )));
        } else if !canon(&dep).starts_with(&root) {
            out.push(Message::warn(format!(
                "{who}: material {rel} uses {shown}, outside the project; it won't ship"
            )));
        }
    }
}

/// Checks an Audio Source's clip: set, present, a sound file, and inside
/// the project so it ships.
fn check_sound(project: &Path, base: &Path, who: &str, rel: &str, out: &mut Vec<Message>) {
    if rel.trim().is_empty() {
        out.push(Message::warn(format!(
            "{who}: the Audio Source has no clip and will stay silent"
        )));
        return;
    }
    let file = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        automata_assets::join_relative(base, rel)
    };
    if !file.is_file() {
        out.push(Message::error(format!("{who}: sound {rel} not found")));
        return;
    }
    if !automata_core::audio_source::is_audio_path(&file) {
        out.push(Message::warn(format!(
            "{who}: {rel} isn't a sound file the engine plays (.ogg, .wav, .mp3, .flac)"
        )));
    }
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if !canon(&file).starts_with(canon(project)) {
        out.push(Message::error(format!(
            "{who}: sound {rel} is outside the project folder and won't ship \
             (drop it into the editor to import it into sounds/)"
        )));
    }
}

/// Where templates are installed: `$AUTOMATA_TEMPLATES`, else
/// `~/.local/share/automata/templates/<version>` (Linux) or
/// `%APPDATA%\automata\templates\<version>` (Windows).
#[must_use]
pub fn templates_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AUTOMATA_TEMPLATES") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
    }
    .unwrap_or_else(std::env::temp_dir);
    base.join("automata").join("templates").join(VERSION)
}

/// The installed template folder for `target`, if there is one.
#[must_use]
pub fn find_template(target: Target) -> Option<PathBuf> {
    let dir = templates_dir().join(target.triple());
    dir.join(target.player_file()).is_file().then_some(dir)
}

/// Whether a template folder bundles its own Python (a development
/// template doesn't, and needs Python with `automata` installed).
#[must_use]
pub fn template_has_runtime(dir: &Path) -> bool {
    dir.join("runtime").is_dir()
}

/// Installs a template archive (`automata-template-<target>.tar.gz`) for
/// `target`, replacing any installed one.
///
/// # Errors
///
/// When the archive can't be extracted or holds no player.
pub fn install_template(archive_path: &Path, target: Target) -> Result<PathBuf, String> {
    let dest = templates_dir().join(target.triple());
    let staging = dest.with_extension("installing");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    archive::extract_tar_gz(archive_path, &staging)
        .map_err(|e| format!("{}: {e}", archive_path.display()))?;
    if !staging.join(target.player_file()).is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!(
            "{} is not a {} template (no {})",
            archive_path.display(),
            target.label(),
            target.player_file()
        ));
    }
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Downloads the published template for `target` (with `curl`, which ships
/// with Linux distributions and Windows 10+) and installs it.
///
/// # Errors
///
/// When the download or install fails.
pub fn download_template(target: Target) -> Result<PathBuf, String> {
    let name = target.template_archive();
    let tmp = std::env::temp_dir().join(&name);
    let _ = std::fs::remove_file(&tmp);
    let mut tried = Vec::new();
    // 1. The public release URL.
    let url = format!("{TEMPLATE_URL}/{name}");
    if curl(&["-fsSL", "--retry", "2", "-o"], &tmp, &url) {
        return install_downloaded(&tmp, target);
    }
    tried.push(format!(
        "{url} (not reachable without signing in: the repository may be private)"
    ));
    // 2. The GitHub CLI, signed in (`gh auth login`): works for private repos.
    let gh = std::process::Command::new("gh")
        .args([
            "release",
            "download",
            TEMPLATE_TAG,
            "--repo",
            TEMPLATE_REPO,
            "--clobber",
        ])
        .args(["--pattern", &name, "--output"])
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();
    match gh {
        Ok(out) if out.status.success() && tmp.is_file() => {
            return install_downloaded(&tmp, target);
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            let first = err.lines().next().unwrap_or("failed").trim();
            let short: String = first.chars().take(160).collect();
            tried.push(format!("gh release download: {short}"));
        }
        Err(_) => tried.push("gh (GitHub CLI) is not installed".into()),
    }
    // 3. A token in GH_TOKEN / GITHUB_TOKEN, through the GitHub API.
    let token = std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .ok()
        .filter(|t| !t.trim().is_empty());
    if let Some(token) = token {
        match download_with_token(&token, &name, &tmp) {
            Ok(()) => return install_downloaded(&tmp, target),
            Err(e) => tried.push(format!("GitHub API with GH_TOKEN: {e}")),
        }
    } else {
        tried.push("no GH_TOKEN / GITHUB_TOKEN set".into());
    }
    Err(format!(
        "could not download {name}. Tried:\n  - {}\nSign in with `gh auth login` (or set GH_TOKEN), or install a template file with `automata-export templates install <file>`.",
        tried.join("\n  - ")
    ))
}

fn install_downloaded(tmp: &Path, target: Target) -> Result<PathBuf, String> {
    let dir = install_template(tmp, target);
    let _ = std::fs::remove_file(tmp);
    dir
}

/// Runs curl with `args`, `-o <out>` style output and `url`; true on success.
fn curl(args: &[&str], out: &Path, url: &str) -> bool {
    std::process::Command::new("curl")
        .args(args)
        .arg(out)
        .arg(url)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Fetches a release asset through the API (private repositories).
fn download_with_token(token: &str, name: &str, out: &Path) -> Result<(), String> {
    let api = format!("https://api.github.com/repos/{TEMPLATE_REPO}/releases/tags/{TEMPLATE_TAG}");
    let auth = format!("Authorization: Bearer {token}");
    let json = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            &auth,
            "-H",
            "Accept: application/vnd.github+json",
            &api,
        ])
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !json.status.success() {
        return Err("the release isn't visible with this token".into());
    }
    let asset = asset_api_url(&String::from_utf8_lossy(&json.stdout), name)
        .ok_or_else(|| format!("the release has no {name}"))?;
    let ok = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--retry",
            "2",
            "-H",
            &auth,
            "-H",
            "Accept: application/octet-stream",
            "-o",
        ])
        .arg(out)
        .arg(&asset)
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        Err(format!("downloading {asset} failed"))
    }
}

/// The API URL of the asset called `name` in a release's JSON. Each asset
/// object starts with its `"url"`; the uploader's own `"url"` comes after
/// the asset's `"name"`, so splitting on `"url"` keeps them apart.
fn asset_api_url(json: &str, name: &str) -> Option<String> {
    let quoted = format!("\"{name}\"");
    json.split("\"url\"").find_map(|chunk| {
        let rest = chunk.trim_start().strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let url = &rest[..rest.find('"')?];
        (url.contains("/releases/assets/") && chunk.contains(&quoted)).then(|| url.to_string())
    })
}

/// Packs a template folder (player + optional `runtime/`) into an archive.
/// CI uses this to publish templates.
///
/// # Errors
///
/// When the folder holds no player or the archive can't be written.
pub fn pack_template(dir: &Path, target: Target, dest: &Path) -> Result<(), String> {
    if !dir.join(target.player_file()).is_file() {
        return Err(format!("{} has no {}", dir.display(), target.player_file()));
    }
    let entries = archive::entries(dir, "", &|rel| {
        executable_in_build(target, rel, target.player_file())
    })
    .map_err(|e| e.to_string())?;
    archive::write_tar_gz(dest, &entries).map_err(|e| e.to_string())
}

/// Makes a development template from a built `automata-player`: no
/// bundled Python, so the exported game uses the Python on `PATH` (with
/// `automata` installed). Handy for testing exports without downloads.
///
/// # Errors
///
/// When the player can't be copied.
pub fn install_dev_template(player: &Path, target: Target) -> Result<PathBuf, String> {
    let dest = templates_dir().join(target.triple());
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let to = dest.join(target.player_file());
    std::fs::copy(player, &to).map_err(|e| format!("{}: {e}", player.display()))?;
    make_executable(&to);
    Ok(dest)
}

fn executable_in_build(target: Target, rel: &str, exe: &str) -> bool {
    target == Target::Linux
        && (rel == exe
            || rel.starts_with("runtime/bin/")
            || std::path::Path::new(rel)
                .extension()
                .is_some_and(|e| e == "so"))
}

/// The result of exporting one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The platform.
    pub target: Target,
    /// The game folder (run the executable inside it).
    pub folder: PathBuf,
    /// The executable.
    pub executable: PathBuf,
    /// The archive, when one was written.
    pub archive: Option<PathBuf>,
    /// Whether the build bundles Python (false for development templates).
    pub standalone: bool,
}

/// Exports `project` for `target` using the template in `template`.
/// `progress` receives messages as the export runs.
///
/// # Errors
///
/// When validation fails or a file can't be copied / written.
pub fn export(
    project: &Path,
    cfg: &BuildConfig,
    target: Target,
    template: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<Exported, String> {
    let problems = validate(project, cfg);
    let mut failed = false;
    for p in problems {
        failed |= p.level == Level::Error;
        progress(p);
    }
    if failed {
        return Err("the project has errors; fix them and build again".into());
    }
    let player = template.join(target.player_file());
    if !player.is_file() {
        return Err(format!(
            "no {} template at {} (Build Settings → Download templates)",
            target.label(),
            template.display()
        ));
    }
    let standalone = template_has_runtime(template);
    if !standalone {
        progress(Message::warn(format!(
            "{} template has no bundled Python: the build runs with the Python on PATH (development template).",
            target.label()
        )));
    }

    let name = cfg.file_name();
    let out_root = project.join(&cfg.output);
    let folder = out_root.join(target.name()).join(&name);
    progress(Message::info(format!(
        "{}: exporting to {}",
        target.label(),
        folder.display()
    )));
    if folder.exists() {
        std::fs::remove_dir_all(&folder)
            .map_err(|e| format!("could not clear {}: {e}", folder.display()))?;
    }
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    // The launcher, renamed after the game.
    let exe = folder.join(format!("{name}{}", target.exe_suffix()));
    std::fs::copy(&player, &exe).map_err(|e| format!("{}: {e}", player.display()))?;
    make_executable(&exe);

    // The runtime.
    if standalone {
        let n = copy_tree(&template.join("runtime"), &folder.join("runtime"), &|_| {
            true
        })?;
        progress(Message::info(format!(
            "{}: runtime ({n} files)",
            target.label()
        )));
    }

    // The game files.
    let n = copy_game(project, cfg, &folder.join("game"))?;
    progress(Message::info(format!(
        "{}: game ({n} files)",
        target.label()
    )));

    let archive_path = if cfg.archive {
        let file = format!(
            "{name}-{}-{}.{}",
            cfg.version.trim(),
            target.triple(),
            target.archive_ext()
        );
        let path = out_root.join(file);
        let entries = archive::entries(&folder, &name, &|rel| {
            executable_in_build(target, rel, &format!("{name}{}", target.exe_suffix()))
        })
        .map_err(|e| e.to_string())?;
        match target {
            Target::Linux => archive::write_tar_gz(&path, &entries),
            Target::Windows => archive::write_zip(&path, &entries),
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        progress(Message::info(format!(
            "{}: archive {}",
            target.label(),
            path.display()
        )));
        Some(path)
    } else {
        None
    };
    progress(Message::info(format!(
        "{}: done → {}",
        target.label(),
        exe.display()
    )));
    Ok(Exported {
        target,
        folder,
        executable: exe,
        archive: archive_path,
        standalone,
    })
}

/// Writes the game folder an export ships (`game/` in a build) to `dest`:
/// the project's files minus the left-out ones, plus `game.json`. Builds
/// that don't come from a desktop template use this; the Android build
/// packs the folder into its APK. Returns how many project files were
/// copied. A previous staged folder at `dest` is replaced.
///
/// # Errors
///
/// When the project has errors (each reported through `progress`), when
/// `dest` is the project, holds it, or sits inside it without being left
/// out (the copy would include itself), when `dest` is a folder with other
/// things in it, or when files can't be copied.
pub fn stage_game(
    project: &Path,
    cfg: &BuildConfig,
    dest: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<usize, String> {
    let mut failed = false;
    for p in validate(project, cfg) {
        failed |= p.level == Level::Error;
        progress(p);
    }
    if failed {
        return Err("the project has errors; fix them and build again".into());
    }
    let skip = Skip::new(cfg);
    if project.starts_with(dest) {
        return Err(format!(
            "{} holds the project; stage the game somewhere else",
            dest.display()
        ));
    }
    if let Ok(rel) = dest.strip_prefix(project) {
        if !skip.excluded(&archive::slash_path(rel)) {
            return Err(format!(
                "{} is inside the project and would be copied into itself; use the output folder ({}) or somewhere outside",
                dest.display(),
                cfg.output
            ));
        }
    }
    let foreign = std::fs::read_dir(dest).is_ok_and(|mut entries| entries.next().is_some())
        && !dest.join("game.json").is_file();
    if foreign {
        return Err(format!(
            "{} isn't empty and isn't a staged game; choose an empty or new folder",
            dest.display()
        ));
    }
    copy_game(project, cfg, dest)
}

/// Replaces `dest` with the project's shipped files and `game.json`.
fn copy_game(project: &Path, cfg: &BuildConfig, dest: &Path) -> Result<usize, String> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .map_err(|e| format!("could not clear {}: {e}", dest.display()))?;
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let skip = Skip::new(cfg);
    let n = copy_tree(project, dest, &|rel| !skip.excluded(rel))?;
    write_manifest(&dest.join("game.json"), cfg).map_err(|e| e.to_string())?;
    Ok(n)
}

/// Paths an export leaves out.
struct Skip {
    output: String,
    extra: Vec<String>,
}

impl Skip {
    fn new(cfg: &BuildConfig) -> Self {
        let output = archive::slash_path(Path::new(&cfg.output));
        Self {
            output,
            extra: cfg
                .exclude
                .iter()
                .map(|e| e.trim().trim_start_matches("./").to_string())
                .collect(),
        }
    }

    fn excluded(&self, rel: &str) -> bool {
        let parts: Vec<&str> = rel.split('/').collect();
        let always = parts.iter().any(|p| {
            matches!(
                *p,
                ".oberon"
                    | "__pycache__"
                    | ".git"
                    | ".hg"
                    | ".svn"
                    | ".venv"
                    | "venv"
                    | ".pytest_cache"
                    | ".mypy_cache"
                    | ".ruff_cache"
                    | "node_modules"
            )
        }) || Path::new(rel)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pyc"))
            || rel == CONFIG_FILE
            || rel == ".DS_Store";
        let output = !self.output.is_empty()
            && (rel == self.output || rel.starts_with(&format!("{}/", self.output)));
        let extra = self.extra.iter().any(|e| {
            if let Some(dir) = e.strip_suffix('/') {
                rel == dir || rel.starts_with(&format!("{dir}/"))
            } else {
                rel == e
            }
        });
        always || output || extra
    }
}

/// Drops Windows' verbatim prefix (`\\?\C:\…` → `C:\…`,
/// `\\?\UNC\host\…` → `\\host\…`) that `canonicalize` adds. In verbatim
/// form `/` is not a separator, so Python opening `game\scripts/x.py` fails.
/// Other paths are returned unchanged.
#[must_use]
pub fn plain_path(p: PathBuf) -> PathBuf {
    let plain = p.to_str().and_then(|s| {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            Some(format!(r"\\{rest}"))
        } else {
            s.strip_prefix(r"\\?\")
                .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
                .map(str::to_string)
        }
    });
    plain.map_or(p, PathBuf::from)
}

fn copy_tree(from: &Path, to: &Path, keep: &dyn Fn(&str) -> bool) -> Result<usize, String> {
    let mut count = 0;
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let items = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for item in items {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            let rel = archive::slash_path(path.strip_prefix(from).unwrap_or(&path));
            if !keep(&rel) {
                continue;
            }
            let kind = item.file_type().map_err(|e| e.to_string())?;
            let dest = to.join(&rel);
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() || archive::is_file_link(&path) {
                // Links are copied as the file they point at.
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::copy(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
                count += 1;
            }
        }
    }
    Ok(count)
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_manifest(path: &Path, cfg: &BuildConfig) -> std::io::Result<()> {
    let opt = |v: &Option<String>| v.as_deref().map_or_else(|| "null".to_string(), json_string);
    let text = format!(
        "{{\n  \"name\": {},\n  \"version\": {},\n  \"start_scene\": {},\n  \"main\": {},\n  \"resolution\": [{}, {}],\n  \"engine\": {}\n}}\n",
        json_string(&cfg.name),
        json_string(&cfg.version),
        opt(&cfg.start_scene),
        opt(&cfg.main_script),
        cfg.resolution.0,
        cfg.resolution.1,
        json_string(VERSION),
    );
    std::fs::write(path, text)
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_asset_api_url_in_release_json() {
        let json = r#"{"url":"https://api.github.com/repos/x/y/releases/1","assets":[
          {"url": "https://api.github.com/repos/x/y/releases/assets/11", "id":11,
           "name": "automata-template-windows-x86_64.tar.gz",
           "uploader": {"url": "https://api.github.com/users/bot"}},
          {"url":"https://api.github.com/repos/x/y/releases/assets/22","id":22,
           "name":"automata-template-linux-x86_64.tar.gz",
           "uploader":{"url":"https://api.github.com/users/bot"}}]}"#;
        assert_eq!(
            asset_api_url(json, "automata-template-linux-x86_64.tar.gz").as_deref(),
            Some("https://api.github.com/repos/x/y/releases/assets/22")
        );
        assert_eq!(
            asset_api_url(json, "automata-template-windows-x86_64.tar.gz").as_deref(),
            Some("https://api.github.com/repos/x/y/releases/assets/11")
        );
        assert_eq!(asset_api_url(json, "missing.tar.gz"), None);
    }

    #[test]
    fn verbatim_windows_paths_are_made_plain() {
        let plain = |s: &str| plain_path(PathBuf::from(s)).to_string_lossy().into_owned();
        assert_eq!(plain(r"\\?\C:\Games\Coin Run"), r"C:\Games\Coin Run");
        assert_eq!(plain(r"\\?\UNC\server\share\game"), r"\\server\share\game");
        // Device paths and ordinary paths are left alone.
        assert_eq!(plain(r"\\?\Volume{1234}\game"), r"\\?\Volume{1234}\game");
        assert_eq!(plain("/home/me/game"), "/home/me/game");
    }

    fn project(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("automata-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::create_dir_all(dir.join(".oberon")).unwrap();
        std::fs::create_dir_all(dir.join("scripts/__pycache__")).unwrap();
        std::fs::write(
            dir.join("scripts/coin.py"),
            "def update(entity, ctx):\n    pass\n",
        )
        .unwrap();
        std::fs::write(dir.join("scripts/__pycache__/coin.pyc"), "x").unwrap();
        std::fs::write(dir.join(".oberon/autosave.ron"), "x").unwrap();
        let scene = automata_core::scene_file::SceneFile {
            version: 1,
            entities: Vec::new(),
        };
        std::fs::write(dir.join("level.ron"), scene.to_ron().unwrap()).unwrap();
        dir
    }

    fn template(tag: &str, target: Target, runtime: bool) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("automata-template-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(target.player_file()), b"player").unwrap();
        if runtime {
            std::fs::create_dir_all(dir.join("runtime/bin")).unwrap();
            std::fs::write(dir.join("runtime/bin/python3"), b"py").unwrap();
        }
        dir
    }

    #[test]
    fn stages_the_game_folder_without_a_template() {
        let dir = project("stage");
        let mut cfg = BuildConfig::for_project(&dir, Some(&dir.join("level.ron")));
        cfg.name = "Coin Run".into();
        let cfg = cfg;
        let out = std::env::temp_dir().join(format!("automata-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let n = stage_game(&dir, &cfg, &out, &mut |_| {}).unwrap();
        assert_eq!(n, 2, "the scene and the script");
        assert!(out.join("scripts/coin.py").is_file());
        assert!(out.join("level.ron").is_file());
        assert!(!out.join(".oberon").exists());
        assert!(!out.join("scripts/__pycache__").exists());
        let manifest = std::fs::read_to_string(out.join("game.json")).unwrap();
        assert!(manifest.contains("\"name\": \"Coin Run\""));
        assert!(manifest.contains("\"start_scene\": \"level.ron\""));

        // Staging again replaces the previous folder.
        std::fs::write(out.join("stale.txt"), "x").unwrap();
        stage_game(&dir, &cfg, &out, &mut |_| {}).unwrap();
        assert!(!out.join("stale.txt").exists());

        // The output folder inside the project is fine; other places inside
        // it, the project itself, and folders with other things in them aren't.
        let inside = dir.join(&cfg.output).join("game");
        stage_game(&dir, &cfg, &inside, &mut |_| {}).unwrap();
        assert!(inside.join("game.json").is_file());
        let no = |dest: &Path| stage_game(&dir, &cfg, dest, &mut |_| {}).unwrap_err();
        assert!(no(&dir.join("scripts/game")).contains("copied into itself"));
        assert!(no(&dir).contains("holds the project"));
        let busy = std::env::temp_dir().join(format!("automata-busy-{}", std::process::id()));
        std::fs::create_dir_all(&busy).unwrap();
        std::fs::write(busy.join("notes.txt"), "mine").unwrap();
        assert!(no(&busy).contains("isn't a staged game"));
        assert!(busy.join("notes.txt").is_file(), "left alone");

        // A project with errors isn't staged.
        let mut broken = cfg.clone();
        broken.start_scene = Some("missing.ron".into());
        let err = stage_game(&dir, &broken, &out, &mut |_| {}).unwrap_err();
        assert!(err.contains("errors"));

        for d in [dir, out, busy] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn exports_a_scene_project_for_both_platforms() {
        let dir = project("both");
        let mut cfg = BuildConfig::for_project(&dir, Some(&dir.join("level.ron")));
        cfg.name = "Coin Run".into();
        assert_eq!(cfg.start_scene.as_deref(), Some("level.ron"));
        for target in Target::ALL {
            let tpl = template(target.name(), target, true);
            let mut log = Vec::new();
            let out = export(&dir, &cfg, target, &tpl, &mut |m| log.push(m)).unwrap();
            assert!(out
                .executable
                .ends_with(format!("Coin Run{}", target.exe_suffix())));
            assert!(out.folder.join("runtime/bin/python3").is_file());
            assert!(out.folder.join("game/scripts/coin.py").is_file());
            assert!(
                !out.folder.join("game/.oberon").exists(),
                "editor state stays out"
            );
            assert!(!out.folder.join("game/scripts/__pycache__").exists());
            assert!(
                !out.folder.join("game/build").exists(),
                "no build inside the build"
            );
            let manifest = std::fs::read_to_string(out.folder.join("game/game.json")).unwrap();
            assert!(manifest.contains("\"start_scene\": \"level.ron\""));
            assert!(manifest.contains("\"main\": null"));
            let archive = out.archive.unwrap();
            assert!(archive.to_string_lossy().ends_with(target.archive_ext()));
            assert!(log.iter().all(|m| m.level != Level::Error));
        }
    }

    #[test]
    fn a_linux_archive_extracts_to_a_runnable_folder() {
        let dir = project("tgz");
        let cfg = BuildConfig {
            name: "Run".into(),
            start_scene: Some("level.ron".into()),
            ..BuildConfig::default()
        };
        let tpl = template("tgz", Target::Linux, true);
        let out = export(&dir, &cfg, Target::Linux, &tpl, &mut |_| {}).unwrap();
        let unpacked = dir.join("unpacked");
        archive::extract_tar_gz(out.archive.as_ref().unwrap(), &unpacked).unwrap();
        assert!(unpacked.join("Run/Run").is_file());
        assert!(unpacked.join("Run/game/game.json").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(unpacked.join("Run/Run"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "the launcher stays executable");
        }
    }

    #[test]
    fn validation_catches_missing_scenes_and_scripts() {
        let dir = project("bad");
        let none = BuildConfig::default();
        assert!(validate(&dir, &none)
            .iter()
            .any(|m| m.level == Level::Error));
        let missing = BuildConfig {
            start_scene: Some("nope.ron".into()),
            ..BuildConfig::default()
        };
        assert!(validate(&dir, &missing)[0].text.contains("nope.ron"));
        // A scene whose entity points at a script that isn't there.
        let mut scene = automata_core::scene_file::SceneFile {
            version: 1,
            entities: Vec::new(),
        };
        scene.entities.push(automata_core::scene_file::EntityData {
            name: Some("Coin".into()),
            script: Some(automata_core::scene_file::Script {
                path: "scripts/gone.py".into(),
            }),
            ..Default::default()
        });
        std::fs::write(dir.join("broken.ron"), scene.to_ron().unwrap()).unwrap();
        let broken = BuildConfig {
            start_scene: Some("broken.ron".into()),
            ..BuildConfig::default()
        };
        let problems = validate(&dir, &broken);
        assert!(problems
            .iter()
            .any(|m| m.text.contains("Coin: script scripts/gone.py not found")));
    }

    #[test]
    fn validation_checks_materials_and_their_textures() {
        use automata_core::scene_file::{EntityData, MeshVisualData, PrimitiveData, SceneFile};
        let dir = project("materials");
        std::fs::create_dir_all(dir.join("materials")).unwrap();
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        std::fs::write(dir.join("textures/brick.png"), b"not checked here").unwrap();
        std::fs::write(
            dir.join("materials/brick.mat.ron"),
            "(base_map: \"../textures/brick.png\", normal_map: \"../textures/gone.png\")",
        )
        .unwrap();
        std::fs::write(dir.join("materials/typo.mat.ron"), "(colour: (1, 1, 1, 1))").unwrap();
        let cube = |name: &str, material: &str| EntityData {
            name: Some(name.into()),
            mesh_visual: Some(MeshVisualData {
                primitive: PrimitiveData::Cube { size: 1.0 },
                color: [1.0; 4],
                surface: automata_core::scene_file::Surface::default(),
                draw_distance: 0.0,
                material: material.into(),
            }),
            ..Default::default()
        };
        let scene = SceneFile {
            version: 1,
            entities: vec![
                cube("Wall", "materials/brick.mat.ron"),
                cube("Odd", "materials/typo.mat.ron"),
                cube("Ghost", "materials/ghost.mat.ron"),
                cube("Plain", ""),
            ],
        };
        std::fs::write(dir.join("mats.ron"), scene.to_ron().unwrap()).unwrap();
        let cfg = BuildConfig {
            name: "Mats".into(),
            start_scene: Some("mats.ron".into()),
            ..BuildConfig::default()
        };
        let log = validate(&dir, &cfg);
        let texts: Vec<String> = log
            .iter()
            .map(|m| format!("{:?} {}", m.level, m.text))
            .collect();
        let has = |s: &str| texts.iter().any(|t| t.contains(s));
        assert!(has("Warning Wall: material materials/brick.mat.ron uses textures/gone.png, which is missing"), "{texts:#?}");
        assert!(!has("brick.png"), "a present texture is fine: {texts:#?}");
        assert!(
            has("Error Odd: material materials/typo.mat.ron doesn't read"),
            "{texts:#?}"
        );
        assert!(
            has("Error Ghost: material materials/ghost.mat.ron not found"),
            "{texts:#?}"
        );
        assert!(!has("Plain"), "{texts:#?}");
    }

    #[test]
    fn validation_checks_models_ship_with_the_game() {
        use automata_core::scene_file::{EntityData, MeshVisualData, PrimitiveData, SceneFile};
        let dir = project("models");
        let outside = std::env::temp_dir().join(format!("export-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(dir.join("models")).unwrap();
        let tri = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        std::fs::write(dir.join("models/rock.obj"), tri).unwrap();
        std::fs::write(
            dir.join("models/crate.obj"),
            format!("mtllib crate.mtl\n{tri}"),
        )
        .unwrap();
        std::fs::write(outside.join("far.obj"), tri).unwrap();
        let model = |name: &str, path: String| EntityData {
            name: Some(name.into()),
            mesh_visual: Some(MeshVisualData {
                primitive: PrimitiveData::Model { path },
                color: [1.0; 4],
                surface: automata_core::scene_file::Surface::default(),
                draw_distance: 0.0,
                material: String::new(),
            }),
            ..Default::default()
        };
        let scene = SceneFile {
            version: 1,
            entities: vec![
                model("Rock", "models/rock.obj".into()),
                model("Crate", "models/crate.obj".into()),
                model("Ghost", "models/ghost.glb".into()),
                model("Far", outside.join("far.obj").display().to_string()),
            ],
        };
        std::fs::write(dir.join("models.ron"), scene.to_ron().unwrap()).unwrap();
        let cfg = BuildConfig {
            start_scene: Some("models.ron".into()),
            ..BuildConfig::default()
        };
        let problems = validate(&dir, &cfg);
        let has = |level: Level, text: &str| {
            problems
                .iter()
                .any(|m| m.level == level && m.text.contains(text))
        };
        assert!(
            !problems.iter().any(|m| m.text.starts_with("Rock")),
            "{problems:?}"
        );
        assert!(
            has(Level::Warning, "Crate: model models/crate.obj uses"),
            "{problems:?}"
        );
        assert!(has(Level::Error, "Ghost: model models/ghost.glb not found"));
        assert!(has(Level::Error, "is outside the project folder"));
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn validation_warns_about_joints_that_cant_hold() {
        use automata_core::scene_file::{EntityData, SceneFile};
        use automata_physics::{Joint, RigidBody};
        let dir = project("joints");
        let scene = SceneFile {
            version: 1,
            entities: vec![
                EntityData {
                    name: Some("Frame".into()),
                    rigid_body: Some(RigidBody::fixed()),
                    ..Default::default()
                },
                EntityData {
                    name: Some("Door".into()),
                    rigid_body: Some(RigidBody::dynamic()),
                    joint: Some(Joint::default().connected_to("Frame")),
                    ..Default::default()
                },
                EntityData {
                    name: Some("Ghost".into()),
                    joint: Some(Joint::ball().connected_to("Nobody")),
                    ..Default::default()
                },
            ],
        };
        std::fs::write(dir.join("j.ron"), scene.to_ron().unwrap()).unwrap();
        let cfg = BuildConfig {
            start_scene: Some("j.ron".into()),
            ..BuildConfig::default()
        };
        let texts: Vec<String> = validate(&dir, &cfg)
            .into_iter()
            .filter(|m| m.level == Level::Warning)
            .map(|m| m.text)
            .collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(texts.iter().all(|t| t.starts_with("Ghost:")), "{texts:?}");
        assert!(texts.iter().any(|t| t.contains("Rigidbody")), "{texts:?}");
        assert!(texts.iter().any(|t| t.contains("\"Nobody\"")), "{texts:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation_checks_collider_models_and_body_types() {
        use automata_core::scene_file::{EntityData, SceneFile};
        use automata_physics::{Collider3D, RigidBody};
        let dir = project("colliders");
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::write(
            dir.join("models/rock.obj"),
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        )
        .unwrap();
        let entity = |name: &str, collider: Collider3D, body: Option<RigidBody>| EntityData {
            name: Some(name.into()),
            collider_3d: Some(collider),
            rigid_body: body,
            ..Default::default()
        };
        let scene = SceneFile {
            version: 1,
            entities: vec![
                entity("Ground", Collider3D::mesh("models/rock.obj"), None),
                entity(
                    "Boulder",
                    Collider3D::convex_hull("models/rock.obj"),
                    Some(RigidBody::dynamic()),
                ),
                entity(
                    "Heavy",
                    Collider3D::mesh("models/rock.obj"),
                    Some(RigidBody::dynamic()),
                ),
                entity("Lost", Collider3D::mesh("models/lost.glb"), None),
            ],
        };
        std::fs::write(dir.join("c.ron"), scene.to_ron().unwrap()).unwrap();
        let cfg = BuildConfig {
            start_scene: Some("c.ron".into()),
            ..BuildConfig::default()
        };
        let problems = validate(&dir, &cfg);
        let named = |who: &str| problems.iter().filter(|m| m.text.starts_with(who)).count();
        assert_eq!(named("Ground"), 0, "{problems:?}");
        assert_eq!(named("Boulder"), 0, "{problems:?}");
        assert!(problems
            .iter()
            .any(|m| m.level == Level::Error && m.text.starts_with("Heavy: A Mesh collider")));
        assert!(problems
            .iter()
            .any(|m| m.level == Level::Error && m.text == "Lost: model models/lost.glb not found"));
    }

    /// The FPS example exports cleanly: its scripts, models (with import
    /// settings) and sounds are all where they belong.
    #[test]
    fn the_fps_example_validates() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/oberon/fps");
        let cfg = BuildConfig::load(&dir).unwrap();
        assert_eq!(cfg.start_scene.as_deref(), Some("level.ron"));
        let problems: Vec<_> = validate(&dir, &cfg)
            .into_iter()
            .filter(|m| m.level != Level::Info)
            .collect();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn validation_checks_audio_clips() {
        use automata_core::scene_file::{EntityData, SceneFile};
        use automata_core::AudioSource;
        let dir = project("sounds");
        std::fs::create_dir_all(dir.join("sounds")).unwrap();
        std::fs::write(dir.join("sounds/bell.ogg"), b"ogg").unwrap();
        let source = |name: &str, clip: &str| EntityData {
            name: Some(name.into()),
            audio_source: Some(AudioSource::new(clip)),
            ..Default::default()
        };
        let scene = SceneFile {
            version: 1,
            entities: vec![
                source("Bell", "sounds/bell.ogg"),
                source("Gone", "sounds/gone.wav"),
                source("Mute", ""),
            ],
        };
        std::fs::write(dir.join("s.ron"), scene.to_ron().unwrap()).unwrap();
        let cfg = BuildConfig {
            start_scene: Some("s.ron".into()),
            ..BuildConfig::default()
        };
        let problems = validate(&dir, &cfg);
        let has = |level: Level, text: &str| {
            problems
                .iter()
                .any(|m| m.level == level && m.text.contains(text))
        };
        assert!(
            !problems.iter().any(|m| m.text.starts_with("Bell")),
            "{problems:?}"
        );
        assert!(
            has(Level::Error, "Gone: sound sounds/gone.wav not found"),
            "{problems:?}"
        );
        assert!(
            has(Level::Warning, "Mute: the Audio Source has no clip"),
            "{problems:?}"
        );
    }

    #[test]
    fn python_string_literals_are_found_and_comments_skipped() {
        assert_eq!(
            string_literals(r#"ctx.hud.text("Hi", 1, 2, font='fonts/A.ttf')  # "b.otf""#),
            vec!["Hi".to_string(), "fonts/A.ttf".to_string()]
        );
        assert_eq!(
            string_literals(r#"x = "it's" + 'say "hi"'"#),
            vec!["it's".to_string(), "say \"hi\"".to_string()]
        );
        assert_eq!(string_literals("x = 'open"), Vec::<String>::new());
    }

    #[test]
    fn validation_checks_fonts_scripts_name() {
        let dir = project("fonts");
        let ttf = |p: &Path, head: &[u8]| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, [head, &[0u8; 60]].concat()).unwrap();
        };
        ttf(&dir.join("fonts/Pixel.ttf"), &[0, 1, 0, 0]);
        ttf(&dir.join("fonts/Round.OTF"), b"OTTO");
        ttf(&dir.join("fonts/fake.ttf"), b"PK\x03\x04");
        ttf(&dir.join("tests/Only.ttf"), &[0, 1, 0, 0]);
        let outside =
            std::env::temp_dir().join(format!("automata-export-font-{}", std::process::id()));
        ttf(&outside.join("Far.ttf"), &[0, 1, 0, 0]);
        let far = outside
            .join("Far.ttf")
            .display()
            .to_string()
            .replace('\\', "/");
        let scene = automata_core::scene_file::SceneFile {
            version: 1,
            entities: vec![],
        };
        std::fs::write(dir.join("s.ron"), scene.to_ron().unwrap()).unwrap();
        std::fs::write(
            dir.join("scripts/hud.py"),
            format!(
                "def update(ctx, dt):\n\
                 \x20   ctx.hud.text('Hi', 0, 0, font=\"fonts/Pixel.ttf\")\n\
                 \x20   ctx.ui.theme.font = 'fonts/Round.OTF'\n\
                 \x20   ctx.hud.text('x', 0, 0, font='fonts\\\\Pixel.ttf')  # 'fonts/Gone.ttf'\n\
                 \x20   ctx.hud.text('x', 0, 0, font='fonts/Gone.ttf')\n\
                 \x20   ctx.hud.text('x', 0, 0, font='fonts/fake.ttf')\n\
                 \x20   ctx.hud.text('x', 0, 0, font='tests/Only.ttf')\n\
                 \x20   ctx.hud.text('x', 0, 0, font='{far}')\n\
                 \x20   ctx.hud.text('x', 0, 0, font=f'fonts/{{name}}.ttf')\n"
            ),
        )
        .unwrap();
        // Test scripts don't ship, so their fonts aren't checked.
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(dir.join("tests/test_x.py"), "F = 'fonts/Nope.ttf'\n").unwrap();
        let cfg = BuildConfig {
            start_scene: Some("s.ron".into()),
            ..BuildConfig::default()
        };
        let problems = validate(&dir, &cfg);
        let fonts: Vec<&str> = problems
            .iter()
            .filter(|m| m.text.contains("font") || m.text.contains("TrueType"))
            .map(|m| {
                assert_eq!(m.level, Level::Error, "{m:?}");
                m.text.as_str()
            })
            .collect();
        assert_eq!(fonts.len(), 4, "{fonts:#?}");
        assert!(fonts[0].starts_with("scripts/hud.py:5: font fonts/Gone.ttf not found"));
        assert!(fonts[1].starts_with("scripts/hud.py:6: fonts/fake.ttf isn't a TrueType"));
        assert!(fonts[2].contains("tests/Only.ttf is excluded from the build"));
        assert!(
            fonts[3].contains("is outside the project folder"),
            "{}",
            fonts[3]
        );
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn validation_checks_string_tables() {
        let dir = project("strings");
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(
            dir.join("fonts/JP.otf"),
            [&b"OTTO"[..], &[0u8; 60]].concat(),
        )
        .unwrap();
        let scene = automata_core::scene_file::SceneFile {
            version: 1,
            entities: vec![],
        };
        std::fs::write(dir.join("s.ron"), scene.to_ron().unwrap()).unwrap();
        std::fs::create_dir_all(dir.join("lang")).unwrap();
        std::fs::write(
            dir.join("lang/strings.csv"),
            "key,en,ja,ko\n@font,,fonts/JP.otf,fonts/KO.otf\n\
             menu.play,Play,プレイ,\nmenu.quit,Quit,,\nmenu.play,Again,,\n",
        )
        .unwrap();
        let cfg = BuildConfig {
            start_scene: Some("s.ron".into()),
            ..BuildConfig::default()
        };
        let texts = |cfg: &BuildConfig| -> Vec<(Level, String)> {
            validate(&dir, cfg)
                .into_iter()
                .filter(|m| m.text.contains("lang/"))
                .map(|m| (m.level, m.text))
                .collect()
        };
        let found = texts(&cfg);
        assert_eq!(found.len(), 4, "{found:#?}");
        assert_eq!(
            found[0],
            (
                Level::Warning,
                "lang/strings.csv:5: menu.play is already in lang/strings.csv:3; keeping that one"
                    .into()
            )
        );
        assert_eq!(found[1].0, Level::Error);
        assert!(
            found[1]
                .1
                .starts_with("lang/*.csv @font (ko): font fonts/KO.otf not found"),
            "{}",
            found[1].1
        );
        assert_eq!(
            found[2],
            (
                Level::Warning,
                "lang/*.csv: 1 key without ja text (menu.quit); they show in en".into()
            )
        );
        assert!(found[3].1.starts_with("lang/*.csv: 2 keys without ko text"));
        // Excluding the tables is an error: the game would show keys.
        let excluded = BuildConfig {
            exclude: vec!["lang/".into()],
            ..cfg
        };
        assert!(texts(&excluded)
            .iter()
            .any(|(l, t)| *l == Level::Error && t.contains("lang/strings.csv is excluded")));
    }

    #[test]
    fn config_round_trips_and_names_are_file_safe() {
        let dir = project("cfg");
        let cfg = BuildConfig {
            name: "Coin: Run?".into(),
            main_script: Some("main.py".into()),
            targets: vec![Target::Windows],
            ..BuildConfig::default()
        };
        cfg.save(&dir).unwrap();
        assert_eq!(BuildConfig::load(&dir).unwrap(), cfg);
        assert_eq!(cfg.file_name(), "Coin Run");
        assert_eq!(BuildConfig::for_project(&dir, None), cfg, "build.ron wins");
        assert_eq!(title_case("coin_run"), "Coin Run");
        assert_eq!(Target::parse("Windows"), Some(Target::Windows));
    }

    #[test]
    fn templates_pack_and_install() {
        let tpl = template("pack", Target::Linux, true);
        let archive = tpl.with_extension("tar.gz");
        pack_template(&tpl, Target::Linux, &archive).unwrap();
        let home = std::env::temp_dir().join(format!("automata-tpl-home-{}", std::process::id()));
        // This is the only test that touches AUTOMATA_TEMPLATES.
        std::env::set_var("AUTOMATA_TEMPLATES", &home);
        let dir = install_template(&archive, Target::Linux).unwrap();
        assert_eq!(find_template(Target::Linux), Some(dir.clone()));
        assert!(template_has_runtime(&dir));
        assert!(
            install_template(&archive, Target::Windows).is_err(),
            "wrong platform"
        );
        std::env::remove_var("AUTOMATA_TEMPLATES");
    }
}
