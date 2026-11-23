//! The in-app file browser behind Open / Save As / prefabs / scripts /
//! model and sound import, and the Settings "Browse…" buttons.
//!
//! Built to be driven with the mouse or the keyboard, typing as little as
//! possible:
//!
//! * a sidebar of places: the project and its asset folders, Home,
//!   Desktop, Documents, Downloads, and recently used folders
//! * a clickable breadcrumb path with Back / Forward / Up
//! * a listing with kind icons, sizes and ages, sortable by name, size or
//!   date; one click selects a file (or opens a folder), a double click
//!   chooses it
//! * typing filters the listing (in open/pick dialogs); ↑/↓, PgUp/PgDn,
//!   Home/End move the selection, Enter chooses, Tab completes, Backspace
//!   in an empty field goes up a folder, Alt+←/→/↑ walk the history
//! * "New Folder" right in the dialog, files dropped from the desktop jump
//!   to their folder, and each kind of dialog remembers where it was

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::dialogs::Dialog;
use super::menus::DialogKind;
use super::project_panel::{asset_kind, draw_tree_icon, unique_child, AssetKind};
use super::settings::{SettingText, SettingsPage};
use super::{ButtonStyle, CursorHint, EditTarget, Intent, Rect, TextEdit, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::files;
use crate::theme::{color, layout};

/// How many folders the sidebar's "Recent" section remembers.
pub const RECENT_DIRS: usize = 8;

const ROW_H: f32 = 24.0;
const PLACE_H: f32 = 24.0;
const SIDEBAR_W: f32 = 176.0;

/// One row of the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// The listing's sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    /// A → Z (folders first).
    #[default]
    Name,
    /// Largest first (folders first).
    Size,
    /// Newest first (folders first).
    Modified,
}

/// A sidebar shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Place {
    label: String,
    path: PathBuf,
    /// The sidebar section: 0 project, 1 computer, 2 recent.
    section: u8,
}

/// A keyboard command for the file browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    /// Select the previous entry.
    Up,
    /// Select the next entry.
    Down,
    /// Select a page up.
    PageUp,
    /// Select a page down.
    PageDown,
    /// Select the first entry.
    First,
    /// Select the last entry.
    Last,
    /// The previous folder in the history.
    Back,
    /// The next folder in the history.
    Forward,
    /// The parent folder.
    Parent,
    /// Re-read the folder.
    Refresh,
}

/// File browser state.
#[derive(Debug, Clone)]
pub struct FileDialog {
    kind: DialogKind,
    dir: PathBuf,
    entries: Vec<Entry>,
    scroll: f32,
    error: Option<String>,
    /// A path the user must confirm overwriting (second click on Save).
    overwrite: Option<PathBuf>,
    /// The highlighted entry's name.
    selected: Option<String>,
    /// The selection moved last (by click or arrow key), so Enter acts on
    /// it; typing clears this.
    browsing: bool,
    /// What filters the listing (the typed text, open dialogs only).
    filter: String,
    /// The name field's text as last seen; a difference means the user
    /// typed.
    seen: String,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    sort: SortBy,
    places: Vec<Place>,
    /// While naming a new folder: the name field's text to restore.
    new_folder: Option<String>,
    /// Settings page to go back to (the Browse… buttons).
    return_to: Option<SettingsPage>,
    /// Scroll the selection into view on the next frame.
    reveal: bool,
    /// Listing rows that fit (PgUp/PgDn step), from the last frame.
    page: usize,
    /// A click just opened a folder: the second half of a double click
    /// must not choose a file in the new one.
    ignore_double: bool,
}

impl FileDialog {
    fn title(&self) -> &'static str {
        match self.kind {
            DialogKind::OpenScene => "Open Scene",
            DialogKind::SaveSceneAs => "Save Scene As",
            DialogKind::SavePrefab(_) => "Save Prefab",
            DialogKind::InstantiatePrefab => "Instantiate Prefab",
            DialogKind::OpenScript => "Open Script",
            DialogKind::NewScript => "New Script",
            DialogKind::AttachScript(_) => "Attach Script — pick one or name a new .py",
            DialogKind::ImportModel(None) => "Import Model",
            DialogKind::ImportModel(Some(_)) => "Choose Model",
            DialogKind::ImportSound(None) => "Import Sound",
            DialogKind::ImportSound(Some(_)) => "Choose Sound",
            DialogKind::ImportFont => "Import Font",
            DialogKind::BuildScene => "Choose Start Scene",
            DialogKind::BuildOutput => "Choose Output Folder",
            DialogKind::PythonInterpreter => "Choose Python Interpreter",
            DialogKind::Shortcuts | DialogKind::About => "",
        }
    }

    /// The extension typed names get (empty: none is added).
    fn extension(&self) -> &'static str {
        match self.kind {
            DialogKind::SavePrefab(_) | DialogKind::InstantiatePrefab => "prefab",
            DialogKind::OpenScript | DialogKind::NewScript | DialogKind::AttachScript(_) => "py",
            DialogKind::ImportModel(_) => "glb",
            DialogKind::ImportSound(_) => "ogg",
            DialogKind::ImportFont => "ttf",
            DialogKind::BuildOutput | DialogKind::PythonInterpreter => "",
            _ => "ron",
        }
    }

    /// The file types listed, for hints.
    fn types_label(&self) -> &'static str {
        match self.kind {
            DialogKind::ImportModel(_) => ".glb .gltf .obj",
            DialogKind::ImportSound(_) => ".ogg .wav .mp3 .flac",
            DialogKind::ImportFont => ".ttf .otf",
            DialogKind::SavePrefab(_) | DialogKind::InstantiatePrefab => ".prefab",
            DialogKind::OpenScript | DialogKind::NewScript | DialogKind::AttachScript(_) => ".py",
            DialogKind::BuildOutput => "folders",
            DialogKind::PythonInterpreter => "all files",
            _ => ".ron",
        }
    }

    /// Whether the browser lists a file called `name`.
    fn shows(&self, name: &str) -> bool {
        match self.kind {
            DialogKind::ImportModel(_) => automata_assets::is_model_path(Path::new(name)),
            DialogKind::ImportSound(_) => {
                automata_core::audio_source::is_audio_path(Path::new(name))
            }
            DialogKind::ImportFont => crate::fonts::is_font_path(Path::new(name)),
            DialogKind::BuildOutput => false,
            DialogKind::PythonInterpreter => true,
            _ => {
                let lower = name.to_ascii_lowercase();
                Path::new(name)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case(self.extension()))
                    && !lower.ends_with(".import.ron")
                    && lower != "build.ron"
            }
        }
    }

    fn saves(&self) -> bool {
        matches!(
            self.kind,
            DialogKind::SaveSceneAs | DialogKind::SavePrefab(_) | DialogKind::NewScript
        )
    }

    fn picks_folder(&self) -> bool {
        self.kind == DialogKind::BuildOutput
    }

    fn needs_existing(&self) -> bool {
        matches!(
            self.kind,
            DialogKind::OpenScene
                | DialogKind::InstantiatePrefab
                | DialogKind::OpenScript
                | DialogKind::ImportModel(_)
                | DialogKind::ImportSound(_)
                | DialogKind::ImportFont
                | DialogKind::BuildScene
                | DialogKind::PythonInterpreter
        )
    }

    /// Whether typing filters the listing (open/pick dialogs: in a save
    /// dialog the typed text is a new name).
    fn filters(&self) -> bool {
        !self.saves() && !self.picks_folder()
    }

    fn action_label(&self) -> &'static str {
        match self.kind {
            DialogKind::OpenScene | DialogKind::OpenScript => "Open",
            DialogKind::InstantiatePrefab => "Instantiate",
            DialogKind::NewScript => "Create",
            DialogKind::AttachScript(_) => "Attach",
            DialogKind::ImportModel(None)
            | DialogKind::ImportSound(None)
            | DialogKind::ImportFont => "Import",
            DialogKind::ImportModel(Some(_))
            | DialogKind::ImportSound(Some(_))
            | DialogKind::BuildScene
            | DialogKind::PythonInterpreter => "Choose",
            DialogKind::BuildOutput => "Use This Folder",
            _ => "Save",
        }
    }

    /// The memory slot for "where this kind of dialog was last".
    fn memory_key(&self) -> &'static str {
        match self.kind {
            DialogKind::BuildOutput => "folder",
            DialogKind::PythonInterpreter => "python",
            DialogKind::ImportModel(_) => "model",
            DialogKind::ImportSound(_) => "sound",
            DialogKind::ImportFont => "font",
            _ => self.extension(),
        }
    }

    fn refresh(&mut self) {
        let mut entries = Vec::new();
        match std::fs::read_dir(&self.dir) {
            Ok(read) => {
                for e in read.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') || name == "__pycache__" {
                        continue;
                    }
                    // Follow symlinks: a link to a folder is a folder.
                    let meta = std::fs::metadata(e.path()).ok();
                    let dir = meta.as_ref().is_some_and(std::fs::Metadata::is_dir);
                    if dir || self.shows(&name) {
                        entries.push(Entry {
                            name,
                            dir,
                            size: meta.as_ref().map_or(0, std::fs::Metadata::len),
                            modified: meta.and_then(|m| m.modified().ok()),
                        });
                    }
                }
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Can't read {}: {e}", self.dir.display())),
        }
        self.entries = entries;
        self.sort_entries();
        if let Some(sel) = &self.selected {
            if !self.entries.iter().any(|e| &e.name == sel) {
                self.selected = None;
            }
        }
    }

    fn sort_entries(&mut self) {
        let sort = self.sort;
        self.entries.sort_by(|a, b| {
            b.dir.cmp(&a.dir).then_with(|| match sort {
                SortBy::Size if !a.dir => b.size.cmp(&a.size),
                SortBy::Modified => b.modified.cmp(&a.modified),
                SortBy::Name | SortBy::Size => natural_cmp(&a.name, &b.name),
            })
        });
    }

    /// Indices of the entries shown with the current filter.
    fn visible(&self) -> Vec<usize> {
        let filter = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| filter.is_empty() || e.name.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect()
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The highlighted entry, if it's shown.
    fn selected_entry(&self) -> Option<&Entry> {
        let name = self.selected.as_deref()?;
        let filter = self.filter.to_lowercase();
        self.entry(name)
            .filter(|e| filter.is_empty() || e.name.to_lowercase().contains(&filter))
    }

    /// Goes to `dir` (recording history unless walking it).
    fn go(&mut self, dir: &Path, record: bool) {
        let dir = absolute(dir);
        if dir == self.dir {
            return;
        }
        let came_from = std::mem::replace(&mut self.dir, dir);
        // Going up lands on the folder we came from.
        self.selected = (came_from.parent() == Some(self.dir.as_path()))
            .then(|| files::display_name(&came_from));
        self.reveal = true;
        if record {
            self.back.push(came_from);
            self.forward.clear();
        }
        self.browsing = false;
        self.filter.clear();
        self.overwrite = None;
        self.scroll = 0.0;
        self.refresh();
    }

    /// Resolves the typed name to a full path with the right extension.
    fn target(&self, name: &str) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut path = self.dir.join(files::expand_home(name));
        if path.extension().is_none() && !self.extension().is_empty() && !path.is_dir() {
            path.set_extension(self.extension());
        }
        Some(path)
    }

    /// The breadcrumb trail for the current folder: (label, path) from the
    /// outermost shown folder in.
    fn crumbs(&self, project_root: &Path) -> Vec<(String, PathBuf)> {
        let project_root = &absolute(project_root);
        let mut base: Option<(String, PathBuf)> = None;
        if self.dir.starts_with(project_root) {
            base = Some((files::display_name(project_root), project_root.clone()));
        } else if let Some(home) = files::home_dir().filter(|h| self.dir.starts_with(h)) {
            base = Some(("Home".to_string(), home));
        }
        let mut out = Vec::new();
        match base {
            Some((label, root)) => {
                out.push((label, root.clone()));
                let mut acc = root.clone();
                if let Ok(rest) = self.dir.strip_prefix(&root) {
                    for part in rest.components() {
                        acc.push(part);
                        out.push((part.as_os_str().to_string_lossy().into_owned(), acc.clone()));
                    }
                }
            }
            None => {
                let mut chain: Vec<&Path> = self.dir.ancestors().collect();
                chain.reverse();
                for p in chain {
                    let label = p.file_name().map_or_else(
                        || p.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    out.push((label, p.to_path_buf()));
                }
            }
        }
        out
    }
}

/// `path` made absolute and free of `..` and symlinks where it exists, so
/// folders compare equal however they were reached.
fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        }
    })
}

/// Compares names the way people count: `level2` before `level10`.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    b.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// What a click in the dialog asks for (applied after drawing).
enum Act {
    Go(PathBuf),
    Select(String),
    Choose(String),
    Submit,
    Back,
    Forward,
    Parent,
    NewFolder,
    Sort(SortBy),
    Cancel,
}

impl Ui {
    fn file_dialog(&self) -> Option<&FileDialog> {
        match &self.dialog {
            Some(Dialog::File(fd)) => Some(fd),
            _ => None,
        }
    }

    fn file_dialog_mut(&mut self) -> Option<&mut FileDialog> {
        match &mut self.dialog {
            Some(Dialog::File(fd)) => Some(fd),
            _ => None,
        }
    }

    /// Whether the file browser is open.
    #[must_use]
    pub fn file_dialog_open(&self) -> bool {
        self.file_dialog().is_some()
    }

    /// The file browser's current folder (tests, screenshots).
    #[must_use]
    pub fn file_dialog_dir(&self) -> Option<&Path> {
        self.file_dialog().map(|fd| fd.dir.as_path())
    }

    /// The highlighted entry in the file browser.
    #[must_use]
    pub fn file_dialog_selection(&self) -> Option<&str> {
        self.file_dialog()?.selected.as_deref()
    }

    /// The text in the file browser's name field.
    #[must_use]
    pub fn file_dialog_name(&self) -> Option<&str> {
        self.edit
            .as_ref()
            .filter(|e| e.target == EditTarget::DialogName)
            .map(|e| e.buffer.as_str())
    }

    /// Puts `text` in the name field (not counted as typing).
    fn set_dialog_name(&mut self, text: String, fresh: bool) {
        if let Some(fd) = self.file_dialog_mut() {
            fd.seen.clone_from(&text);
        }
        self.edit = Some(TextEdit {
            id: 0,
            target: EditTarget::DialogName,
            buffer: text,
            rect: Rect::default(),
            fresh,
            attr: None,
        });
    }

    /// Opens the file browser for `kind`. `doc` suggests names and the
    /// file to start on.
    pub(super) fn open_file_dialog(&mut self, kind: DialogKind, doc: Option<&Document>) {
        let return_to = match kind {
            DialogKind::BuildScene | DialogKind::BuildOutput => Some(SettingsPage::Build),
            DialogKind::PythonInterpreter => Some(SettingsPage::Python),
            _ => None,
        };
        let mut fd = FileDialog {
            kind,
            dir: self.project_root.clone(),
            entries: Vec::new(),
            scroll: 0.0,
            error: None,
            overwrite: None,
            selected: None,
            browsing: false,
            filter: String::new(),
            seen: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            sort: SortBy::Name,
            places: Vec::new(),
            new_folder: None,
            return_to,
            reveal: true,
            page: 10,
            ignore_double: false,
        };
        fd.dir = self.start_dir(&fd);
        let (suggestion, current) = self.dialog_suggestion(kind, doc);
        // Start on the current file: its folder, highlighted, named.
        let mut name = suggestion;
        if let Some(cur) = current.filter(|p| p.exists()) {
            if let (Some(parent), Some(file)) = (cur.parent(), cur.file_name()) {
                if parent.is_dir() {
                    fd.dir = parent.to_path_buf();
                    let file = file.to_string_lossy().into_owned();
                    if kind != DialogKind::BuildOutput {
                        name.clone_from(&file);
                    }
                    fd.selected = Some(file);
                }
            }
        }
        if kind == DialogKind::BuildOutput {
            name.clear();
        }
        fd.dir = absolute(&fd.dir);
        fd.places = self.dialog_places();
        fd.refresh();
        self.dialog = Some(Dialog::File(Box::new(fd)));
        self.set_dialog_name(name, false);
    }

    /// Where a dialog of this kind starts: the Files panel's folder for
    /// scripts, else where this kind was last used, else the project's
    /// matching asset folder.
    fn start_dir(&self, fd: &FileDialog) -> PathBuf {
        let root = self.project_root.clone();
        let kind = fd.kind;
        if matches!(kind, DialogKind::NewScript | DialogKind::OpenScript)
            && self.project.selected.is_some()
        {
            return self.project_dir();
        }
        if let Some(dir) = self.dialog_dirs.get(fd.memory_key()).filter(|d| d.is_dir()) {
            return dir.clone();
        }
        let sub = match kind {
            DialogKind::SavePrefab(_) | DialogKind::InstantiatePrefab => Some("prefabs"),
            DialogKind::NewScript | DialogKind::OpenScript | DialogKind::AttachScript(_) => {
                Some("scripts")
            }
            DialogKind::ImportModel(_) => Some(crate::models::MODELS_DIR),
            DialogKind::ImportSound(_) => Some(crate::sounds::SOUNDS_DIR),
            DialogKind::ImportFont => Some(crate::fonts::FONTS_DIR),
            DialogKind::PythonInterpreter => {
                if Path::new("/usr/bin").is_dir() {
                    return PathBuf::from("/usr/bin");
                }
                return files::home_dir().unwrap_or(root);
            }
            _ => None,
        };
        match sub.map(|s| root.join(s)) {
            Some(dir) if dir.is_dir() => dir,
            _ => root,
        }
    }

    /// The name a dialog suggests, and the file it starts on.
    fn dialog_suggestion(
        &mut self,
        kind: DialogKind,
        doc: Option<&Document>,
    ) -> (String, Option<PathBuf>) {
        let entity = |id: u64| doc.and_then(|d| d.index_of(id).map(|i| &d.entities[i]));
        let base = doc.map_or_else(|| self.project_root.clone(), Document::base_dir);
        match kind {
            DialogKind::OpenScene => (String::new(), doc.and_then(|d| d.path.clone())),
            DialogKind::SaveSceneAs => {
                let name = doc
                    .and_then(|d| d.path.as_ref())
                    .and_then(|p| p.file_name())
                    .map_or_else(
                        || "scene.ron".to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                (name, None)
            }
            DialogKind::SavePrefab(id) => (
                entity(id).map_or_else(
                    || "object.prefab".to_string(),
                    |e| format!("{}.prefab", super::dialogs::sanitize(e.name())),
                ),
                None,
            ),
            DialogKind::NewScript => (
                unique_child(&self.start_dir_for(kind), "new_script.py"),
                None,
            ),
            DialogKind::OpenScript => (
                String::new(),
                self.code
                    .active
                    .and_then(|i| self.code.tabs.get(i))
                    .map(|t| t.path.clone()),
            ),
            DialogKind::AttachScript(id) => {
                let e = entity(id);
                let name = e.map_or_else(
                    || "script.py".to_string(),
                    |e| format!("{}.py", super::dialogs::sanitize(e.name()).to_lowercase()),
                );
                let current = e
                    .and_then(|e| e.data.script.as_ref())
                    .map(|s| base.join(&s.path));
                (name, current)
            }
            DialogKind::ImportModel(Some(id)) => {
                let current = entity(id)
                    .and_then(|e| e.data.mesh_visual.as_ref())
                    .and_then(|m| match &m.primitive {
                        automata_core::PrimitiveData::Model { path } => Some(base.join(path)),
                        _ => None,
                    });
                (String::new(), current)
            }
            DialogKind::ImportSound(Some(id)) => {
                let current = entity(id)
                    .and_then(|e| e.data.audio_source.as_ref())
                    .filter(|a| !a.clip.is_empty())
                    .map(|a| base.join(&a.clip));
                (String::new(), current)
            }
            DialogKind::BuildScene => {
                let current = self
                    .build_settings()
                    .start_scene
                    .clone()
                    .map(|s| self.project_root.join(s));
                (String::new(), current)
            }
            DialogKind::BuildOutput => {
                let out = self.build_settings().output.clone();
                let dir = self.project_root.join(out);
                (String::new(), dir.is_dir().then_some(dir))
            }
            DialogKind::PythonInterpreter => {
                (String::new(), self.prefs.python.clone().map(PathBuf::from))
            }
            _ => (String::new(), None),
        }
    }

    fn start_dir_for(&self, kind: DialogKind) -> PathBuf {
        let probe = FileDialog {
            kind,
            dir: PathBuf::new(),
            entries: Vec::new(),
            scroll: 0.0,
            error: None,
            overwrite: None,
            selected: None,
            browsing: false,
            filter: String::new(),
            seen: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            sort: SortBy::Name,
            places: Vec::new(),
            new_folder: None,
            return_to: None,
            reveal: false,
            page: 0,
            ignore_double: false,
        };
        self.start_dir(&probe)
    }

    /// The sidebar: the project and its asset folders, the usual places
    /// on this computer, and recently used folders.
    fn dialog_places(&self) -> Vec<Place> {
        let mut places = Vec::new();
        let root = &absolute(&self.project_root);
        places.push(Place {
            label: files::display_name(root),
            path: root.clone(),
            section: 0,
        });
        for (sub, label) in [
            ("scenes", "Scenes"),
            ("levels", "Levels"),
            ("scripts", "Scripts"),
            ("prefabs", "Prefabs"),
            (crate::models::MODELS_DIR, "Models"),
            (crate::sounds::SOUNDS_DIR, "Sounds"),
            ("textures", "Textures"),
        ] {
            let path = root.join(sub);
            if path.is_dir() {
                places.push(Place {
                    label: label.to_string(),
                    path,
                    section: 0,
                });
            }
        }
        if let Some(home) = files::home_dir() {
            places.push(Place {
                label: "Home".into(),
                path: home.clone(),
                section: 1,
            });
            for name in ["Desktop", "Documents", "Downloads"] {
                let path = home.join(name);
                if path.is_dir() {
                    places.push(Place {
                        label: name.to_string(),
                        path,
                        section: 1,
                    });
                }
            }
        }
        let mut recent = 0;
        for dir in &self.prefs.recent_dirs {
            if recent >= 5 || !dir.is_dir() || places.iter().any(|p| &p.path == dir) {
                continue;
            }
            places.push(Place {
                label: files::display_name(dir),
                path: dir.clone(),
                section: 2,
            });
            recent += 1;
        }
        places
    }

    /// Remembers where a dialog of this kind finished.
    fn remember_dialog_dir(&mut self, key: &'static str, dir: &Path) {
        self.dialog_dirs.insert(key, dir.to_path_buf());
        let dir = dir.to_path_buf();
        let recent = &mut self.prefs.recent_dirs;
        if recent.first() != Some(&dir) {
            recent.retain(|d| d != &dir);
            recent.insert(0, dir);
            recent.truncate(RECENT_DIRS);
            self.prefs_dirty = true;
        }
    }

    /// Closes the file browser, returning to Settings when it came from
    /// there. Returns whether a file browser was open.
    pub(super) fn close_file_dialog(&mut self) -> bool {
        let Some(fd) = self.file_dialog() else {
            return false;
        };
        let back = fd.return_to;
        self.dialog = back.map(Dialog::Settings);
        if self
            .edit
            .as_ref()
            .is_some_and(|e| e.target == EditTarget::DialogName)
        {
            self.edit = None;
        }
        true
    }

    /// Escape in the name field: leaves "new folder" naming first.
    pub(super) fn file_dialog_escape(&mut self) -> bool {
        let Some(fd) = self.file_dialog_mut() else {
            return false;
        };
        let Some(saved) = fd.new_folder.take() else {
            return false;
        };
        fd.error = None;
        self.set_dialog_name(saved, false);
        true
    }

    /// Notices typing in the name field: the typed text filters the
    /// listing and the best match is highlighted.
    fn sync_file_dialog(&mut self) {
        let typed = self.file_dialog_name().map(str::to_string);
        let Some(fd) = self.file_dialog_mut() else {
            return;
        };
        let Some(text) = typed else {
            return;
        };
        if text == fd.seen || fd.new_folder.is_some() {
            return;
        }
        fd.seen.clone_from(&text);
        fd.browsing = false;
        fd.overwrite = None;
        fd.error = None;
        let plain = !text.contains(['/', '\\']) && !text.starts_with('~');
        if fd.filters() {
            fd.filter = if plain {
                text.trim().to_string()
            } else {
                String::new()
            };
        }
        // Highlight the typed name, or the first match.
        let lower = text.trim().to_lowercase();
        let exact = fd
            .entries
            .iter()
            .find(|e| {
                let n = e.name.to_lowercase();
                n == lower
                    || (!fd.extension().is_empty() && n == format!("{lower}.{}", fd.extension()))
            })
            .map(|e| e.name.clone());
        fd.selected = exact.or_else(|| {
            if fd.filter.is_empty() {
                return None;
            }
            let shown = fd.visible();
            shown
                .iter()
                .map(|&i| &fd.entries[i])
                .find(|e| !e.dir)
                .or_else(|| shown.first().map(|&i| &fd.entries[i]))
                .map(|e| e.name.clone())
        });
        fd.reveal = true;
    }

    /// Handles a navigation key. Returns whether the browser used it.
    pub fn file_dialog_key(&mut self, key: NavKey) -> bool {
        self.sync_file_dialog();
        let Some(fd) = self.file_dialog_mut() else {
            return false;
        };
        if fd.new_folder.is_some() {
            return false;
        }
        match key {
            NavKey::Back => {
                if let Some(dir) = fd.back.pop() {
                    let here = fd.dir.clone();
                    fd.go(&dir, false);
                    fd.forward.push(here);
                    self.after_navigate();
                }
            }
            NavKey::Forward => {
                if let Some(dir) = fd.forward.pop() {
                    let here = fd.dir.clone();
                    fd.go(&dir, false);
                    fd.back.push(here);
                    self.after_navigate();
                }
            }
            NavKey::Parent => self.dialog_parent(),
            NavKey::Refresh => fd.refresh(),
            _ => {
                let shown = fd.visible();
                if shown.is_empty() {
                    return true;
                }
                let pos = fd
                    .selected
                    .as_ref()
                    .and_then(|s| shown.iter().position(|&i| &fd.entries[i].name == s));
                let last = shown.len() - 1;
                let page = fd.page.max(1);
                let next = match (key, pos) {
                    (NavKey::Down, None) | (NavKey::First, _) => 0,
                    (NavKey::Up, None) | (NavKey::Last, _) => last,
                    (NavKey::Down, Some(p)) => (p + 1).min(last),
                    (NavKey::Up, Some(p)) => p.saturating_sub(1),
                    (NavKey::PageDown, p) => p.map_or(0, |p| (p + page).min(last)),
                    (NavKey::PageUp, p) => p.map_or(0, |p| p.saturating_sub(page)),
                    _ => return true,
                };
                let name = fd.entries[shown[next]].name.clone();
                self.dialog_select(&name);
            }
        }
        true
    }

    /// Highlights entry `name`; a file also fills the name field.
    fn dialog_select(&mut self, name: &str) {
        let Some(fd) = self.file_dialog_mut() else {
            return;
        };
        let Some(entry) = fd.entry(name).cloned() else {
            return;
        };
        fd.selected = Some(entry.name.clone());
        fd.browsing = true;
        fd.reveal = true;
        fd.overwrite = None;
        fd.error = None;
        if !entry.dir && !fd.picks_folder() {
            self.set_dialog_name(entry.name, false);
        }
    }

    fn dialog_parent(&mut self) {
        let Some(fd) = self.file_dialog_mut() else {
            return;
        };
        if let Some(parent) = fd.dir.parent().map(Path::to_path_buf) {
            fd.go(&parent, true);
            self.after_navigate();
        }
    }

    /// After changing folder: the name field keeps a save name, but loses
    /// a filter or a picked file.
    fn after_navigate(&mut self) {
        let Some(fd) = self.file_dialog() else {
            return;
        };
        if fd.saves() {
            let keep = self.file_dialog_name().unwrap_or_default().to_string();
            self.set_dialog_name(keep, false);
        } else {
            self.set_dialog_name(String::new(), false);
        }
    }

    #[allow(clippy::needless_pass_by_value)] // callers build the path
    fn dialog_go(&mut self, dir: PathBuf) {
        if let Some(fd) = self.file_dialog_mut() {
            fd.go(&dir, true);
            self.after_navigate();
        }
    }

    /// Tab in the name field: completes to the highlighted entry (opening
    /// it if it's a folder).
    pub(super) fn file_dialog_complete(&mut self) {
        self.sync_file_dialog();
        let Some(fd) = self.file_dialog() else {
            return;
        };
        if fd.new_folder.is_some() {
            return;
        }
        let Some(entry) = fd.selected_entry().cloned() else {
            return;
        };
        if entry.dir {
            let dir = fd.dir.join(&entry.name);
            self.dialog_go(dir);
        } else {
            self.dialog_select(&entry.name);
        }
    }

    /// A file dropped from the desktop while the browser is open: go to
    /// its folder and highlight it. Returns whether the browser took it.
    pub fn file_dialog_drop(&mut self, path: &Path) -> bool {
        if !self.file_dialog_open() {
            return false;
        }
        let (dir, name) = if path.is_dir() {
            (path.to_path_buf(), None)
        } else {
            (
                path.parent().map_or_else(PathBuf::new, Path::to_path_buf),
                path.file_name().map(|n| n.to_string_lossy().into_owned()),
            )
        };
        self.dialog_go(dir);
        if let Some(name) = name {
            self.dialog_select(&name);
        }
        true
    }

    /// Starts naming a new folder in the name field.
    fn begin_new_folder(&mut self) {
        let current = self.file_dialog_name().unwrap_or_default().to_string();
        let Some(fd) = self.file_dialog_mut() else {
            return;
        };
        if fd.new_folder.is_some() {
            return;
        }
        fd.new_folder = Some(current);
        fd.error = None;
        let name = unique_child(&fd.dir, "New Folder");
        self.set_dialog_name(name, true);
    }

    /// Submits the file dialog with `name` typed in its field. Returns the
    /// resulting intent, or `None` if the dialog stays open.
    pub(super) fn submit_dialog(&mut self, name: &str) -> Option<Intent> {
        self.sync_file_dialog();
        let fd = self.file_dialog()?.clone();
        if fd.new_folder.is_some() {
            self.create_dialog_folder(name);
            return None;
        }
        let trimmed = name.trim();
        let selected = fd.selected_entry().cloned();
        // Enter on a highlighted folder opens it.
        if let Some(e) = selected.as_ref().filter(|e| e.dir && fd.browsing) {
            if !fd.picks_folder() || trimmed.is_empty() {
                self.dialog_go(fd.dir.join(&e.name));
                return None;
            }
        }
        if fd.picks_folder() {
            let path = if trimmed.is_empty() {
                fd.dir.clone()
            } else {
                fd.dir.join(files::expand_home(trimmed))
            };
            return self.finish_dialog(&fd, path);
        }
        if trimmed.is_empty() {
            return match selected {
                Some(e) if e.dir => {
                    self.dialog_go(fd.dir.join(&e.name));
                    None
                }
                Some(e) if !fd.saves() => self.finish_dialog(&fd, fd.dir.join(&e.name)),
                _ => {
                    self.dialog_error(if fd.saves() {
                        "Type a name for the file"
                    } else {
                        "Pick a file from the list"
                    });
                    None
                }
            };
        }
        // A folder name (or path) goes there.
        let candidate = fd.dir.join(files::expand_home(trimmed));
        if candidate.is_dir() {
            self.dialog_go(candidate);
            return None;
        }
        let mut path = fd.target(trimmed)?;
        if !path.exists() && fd.needs_existing() {
            // A partial name picks the highlighted match.
            match selected {
                Some(e) if e.dir => {
                    self.dialog_go(fd.dir.join(&e.name));
                    return None;
                }
                Some(e) => path = fd.dir.join(&e.name),
                None => {
                    self.dialog_error(&format!(
                        "Nothing called \"{trimmed}\" here — pick a file from the list"
                    ));
                    return None;
                }
            }
        }
        if fd.saves()
            && path.exists()
            && fd.overwrite.as_ref() != Some(&path)
            && fd.kind != DialogKind::NewScript
        {
            let msg = format!(
                "{} already exists. Press {} again to replace it.",
                files::display_name(&path),
                fd.action_label()
            );
            if let Some(fd) = self.file_dialog_mut() {
                fd.error = Some(msg);
                fd.overwrite = Some(path);
            }
            return None;
        }
        self.finish_dialog(&fd, path)
    }

    fn dialog_error(&mut self, msg: &str) {
        if let Some(fd) = self.file_dialog_mut() {
            fd.error = Some(msg.to_string());
        }
    }

    fn create_dialog_folder(&mut self, name: &str) {
        let Some(fd) = self.file_dialog() else {
            return;
        };
        let name = name.trim().to_string();
        if let Err(e) = files::check_name(&name) {
            self.dialog_error(&e);
            return;
        }
        let path = fd.dir.join(&name);
        if path.exists() {
            self.dialog_error(&format!("{name} already exists"));
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&path) {
            self.dialog_error(&format!("Couldn't create the folder: {e}"));
            return;
        }
        let saved = self
            .file_dialog_mut()
            .and_then(|fd| fd.new_folder.take())
            .unwrap_or_default();
        self.files_changed(None);
        // Into the new folder, keeping the name being saved.
        if let Some(fd) = self.file_dialog_mut() {
            fd.go(&path, true);
        }
        let keep = self.file_dialog().is_some_and(FileDialog::saves);
        self.set_dialog_name(if keep { saved } else { String::new() }, false);
    }

    /// Closes the browser on `path` and turns it into an intent (or a
    /// Settings value).
    fn finish_dialog(&mut self, fd: &FileDialog, path: PathBuf) -> Option<Intent> {
        let dir = if fd.picks_folder() {
            path.clone()
        } else {
            path.parent()
                .map_or_else(|| fd.dir.clone(), Path::to_path_buf)
        };
        if dir.is_dir() {
            self.remember_dialog_dir(fd.memory_key(), &dir);
        }
        let intent = match fd.kind {
            DialogKind::OpenScene => Intent::Open(path),
            DialogKind::SaveSceneAs => Intent::SaveAs(path),
            DialogKind::SavePrefab(id) => Intent::SavePrefab { id, path },
            DialogKind::InstantiatePrefab => Intent::InstantiatePrefab(path),
            DialogKind::OpenScript => Intent::OpenScript(path),
            DialogKind::NewScript => Intent::NewScript(path),
            DialogKind::AttachScript(id) => Intent::AttachScript { id, path },
            DialogKind::ImportModel(None) => Intent::AddModel { path, at: None },
            DialogKind::ImportModel(Some(id)) => Intent::SetModel { id, path },
            DialogKind::ImportSound(None) => Intent::AddSound { path, at: None },
            DialogKind::ImportSound(Some(id)) => Intent::SetAudioClip { id, path },
            DialogKind::ImportFont => Intent::ImportFont(path),
            DialogKind::BuildScene | DialogKind::BuildOutput => {
                let which = if fd.kind == DialogKind::BuildScene {
                    SettingText::BuildScene
                } else {
                    SettingText::BuildOutput
                };
                let value = self.project_relative(&path);
                self.close_file_dialog();
                self.commit_setting(which, &value);
                return None;
            }
            DialogKind::PythonInterpreter => {
                self.close_file_dialog();
                return self.commit_setting(SettingText::Python, &path.display().to_string());
            }
            DialogKind::Shortcuts | DialogKind::About => return None,
        };
        self.dialog = None;
        self.edit = None;
        Some(intent)
    }

    /// `path` relative to the project (`/`-separated), or absolute when
    /// it lies outside.
    fn project_relative(&self, path: &Path) -> String {
        let root =
            std::fs::canonicalize(&self.project_root).unwrap_or_else(|_| self.project_root.clone());
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        match abs.strip_prefix(&root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Ok(_) => ".".into(),
            Err(_) => path.display().to_string(),
        }
    }

    /// Draws the file browser in `r` and handles its pointer input.
    pub(super) fn draw_file_dialog(&mut self, canvas: &mut Canvas, r: Rect) {
        self.sync_file_dialog();
        if self.file_dialog_name().is_none() {
            let seen = self
                .file_dialog()
                .map(|fd| fd.seen.clone())
                .unwrap_or_default();
            self.set_dialog_name(seen, false);
        }
        let Some(fd) = self.file_dialog().cloned() else {
            return;
        };
        let mut acts = Vec::new();

        // Title and close.
        canvas.text_in(
            crate::canvas::Face::Medium,
            r.x + 20.0,
            r.y + 16.0,
            15.0,
            fd.title(),
            color::text(),
        );
        let close = Rect::new(r.x + r.w - 40.0, r.y + 12.0, 26.0, 26.0);
        self.tip(close, "Cancel (Esc)");
        if self.icon_button(canvas, close, super::Icon::Close, ButtonStyle::Ghost) {
            acts.push(Act::Cancel);
        }

        // Toolbar: Back / Forward / Up, the breadcrumbs, New Folder.
        let ty = r.y + 48.0;
        let nav = [
            ("←", "Back (Alt+←)", !fd.back.is_empty(), Act::Back),
            ("→", "Forward (Alt+→)", !fd.forward.is_empty(), Act::Forward),
            (
                "↑",
                "Up a folder (Backspace)",
                fd.dir.parent().is_some(),
                Act::Parent,
            ),
        ];
        let mut x = r.x + 16.0;
        for (label, tip, enabled, act) in nav {
            let b = Rect::new(x, ty, 28.0, 26.0);
            self.tip(b, tip);
            let style = if enabled {
                ButtonStyle::Ghost
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, b, label, style) {
                acts.push(act);
            }
            x += 30.0;
        }
        let nf_w = 116.0;
        let new_folder = Rect::new(r.x + r.w - 16.0 - nf_w, ty, nf_w, 26.0);
        self.tip(new_folder, "Create a folder here");
        if self.button(canvas, new_folder, "+ New Folder", ButtonStyle::Normal) {
            acts.push(Act::NewFolder);
        }
        let crumb_area = Rect::new(x + 6.0, ty, new_folder.x - x - 14.0, 26.0);
        self.draw_crumbs(canvas, crumb_area, &fd, &mut acts);

        // Body: places on the left, the listing on the right.
        let body_y = ty + 36.0;
        let body_h = (r.y + r.h - 108.0 - body_y).max(60.0);
        let side = Rect::new(r.x + 12.0, body_y, SIDEBAR_W, body_h);
        self.draw_places(canvas, side, &fd, &mut acts);
        let list = Rect::new(
            side.x + side.w + 10.0,
            body_y,
            r.x + r.w - 16.0 - (side.x + side.w + 10.0),
            body_h,
        );
        self.draw_listing(canvas, list, &fd, &mut acts);

        // Footer: the name field, a hint or error, and the buttons.
        let fy = list.y + list.h + 14.0;
        let label = if fd.new_folder.is_some() {
            "New folder"
        } else if fd.picks_folder() {
            "Subfolder"
        } else if fd.saves() {
            "Name"
        } else {
            "File"
        };
        canvas.text(
            r.x + 20.0,
            fy + 5.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
        );
        let field = Rect::new(r.x + 108.0, fy, r.w - 124.0, 26.0);
        canvas.fill_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            4.0,
            color::viewport_bg(),
        );
        canvas.stroke_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            4.0,
            color::accent(),
            1.0,
        );
        let typed = self.file_dialog_name().unwrap_or_default().to_string();
        // The extension a typed name gets, shown faintly.
        let ext = fd.extension();
        if fd.new_folder.is_none()
            && !ext.is_empty()
            && !typed.is_empty()
            && Path::new(typed.trim()).extension().is_none()
            && fd.saves()
        {
            let suffix = format!(".{ext} will be added");
            let sw = canvas.measure_text(layout::SMALL_SIZE, &suffix);
            canvas.text(
                field.x + field.w - sw - 8.0,
                field.y + 7.0,
                layout::SMALL_SIZE,
                &suffix,
                color::text_faint(),
            );
        }
        if typed.is_empty() && fd.new_folder.is_none() {
            let placeholder = if fd.picks_folder() {
                "Optional — or click Use This Folder"
            } else if fd.saves() {
                "Type a name"
            } else {
                "Type to filter, or pick from the list"
            };
            canvas.text(
                field.x + 8.0,
                field.y + 6.0,
                layout::FONT_SIZE,
                placeholder,
                color::text_faint(),
            );
        }
        self.draw_edit_text(canvas, field, field.x + 8.0);

        let hint_y = fy + 36.0;
        if let Some(err) = &fd.error {
            canvas.text_clipped(
                r.x + 20.0,
                hint_y,
                layout::SMALL_SIZE,
                err,
                color::warn(),
                r.w - 280.0,
            );
        } else {
            let hint = if fd.new_folder.is_some() {
                "Enter creates the folder · Esc cancels".to_string()
            } else if fd.picks_folder() {
                "Click a folder to open it · Enter or Use This Folder picks the one you're in"
                    .to_string()
            } else {
                format!(
                    "{}  ·  ↑↓ select · Enter {} · Tab complete · Backspace up",
                    fd.types_label(),
                    fd.action_label().to_lowercase()
                )
            };
            canvas.text_clipped(
                r.x + 20.0,
                hint_y,
                layout::SMALL_SIZE,
                &hint,
                color::text_faint(),
                r.w - 280.0,
            );
        }
        let by = r.y + r.h - 40.0;
        let action = if fd.new_folder.is_some() {
            "Create Folder"
        } else {
            fd.action_label()
        };
        let aw = canvas.measure_text(layout::FONT_SIZE, action).max(70.0) + 32.0;
        let ok = Rect::new(r.x + r.w - 16.0 - aw, by, aw, 28.0);
        let cancel = Rect::new(ok.x - 98.0, by, 90.0, 28.0);
        if self.button(canvas, cancel, "Cancel", ButtonStyle::Normal) {
            acts.push(Act::Cancel);
        }
        if self.button(canvas, ok, action, ButtonStyle::Accent) {
            acts.push(Act::Submit);
        }

        for act in acts {
            if !self.file_dialog_open() {
                break;
            }
            self.apply_dialog_act(act);
        }
    }

    fn apply_dialog_act(&mut self, act: Act) {
        match act {
            Act::Go(dir) => self.dialog_go(dir),
            Act::Select(name) => self.dialog_select(&name),
            Act::Choose(name) => {
                self.dialog_select(&name);
                self.submit_current();
            }
            Act::Submit => self.submit_current(),
            Act::Back => {
                self.file_dialog_key(NavKey::Back);
            }
            Act::Forward => {
                self.file_dialog_key(NavKey::Forward);
            }
            Act::Parent => self.dialog_parent(),
            Act::NewFolder => self.begin_new_folder(),
            Act::Sort(by) => {
                if let Some(fd) = self.file_dialog_mut() {
                    fd.sort = by;
                    fd.sort_entries();
                    fd.reveal = true;
                }
            }
            Act::Cancel => {
                if !self.file_dialog_escape() {
                    self.close_file_dialog();
                }
            }
        }
    }

    fn submit_current(&mut self) {
        let name = self.file_dialog_name().unwrap_or_default().to_string();
        if let Some(intent) = self.submit_dialog(&name) {
            self.out.push(intent);
        }
    }

    fn draw_crumbs(
        &mut self,
        canvas: &mut Canvas,
        area: Rect,
        fd: &FileDialog,
        acts: &mut Vec<Act>,
    ) {
        canvas.fill_rounded(area.x, area.y, area.w, area.h, 4.0, color::field());
        let crumbs = fd.crumbs(&self.project_root);
        let sep = " › ";
        let sep_w = canvas.measure_text(layout::FONT_SIZE, sep);
        let widths: Vec<f32> = crumbs
            .iter()
            .map(|(l, _)| canvas.measure_text(layout::FONT_SIZE, l) + 12.0)
            .collect();
        // Drop leading crumbs (shown as "…") until the trail fits.
        let ellipsis_w = canvas.measure_text(layout::FONT_SIZE, "…") + 12.0 + sep_w;
        let mut start = 0;
        let total = |from: usize| -> f32 {
            widths[from..].iter().sum::<f32>()
                + sep_w * (widths.len() - from).saturating_sub(1) as f32
                + if from > 0 { ellipsis_w } else { 0.0 }
        };
        while start + 1 < crumbs.len() && total(start) > area.w - 8.0 {
            start += 1;
        }
        let mut x = area.x + 4.0;
        if start > 0 {
            let b = Rect::new(x, area.y + 2.0, ellipsis_w - sep_w, area.h - 4.0);
            self.tip(b, &crumbs[start - 1].1.display().to_string());
            if self.button(canvas, b, "…", ButtonStyle::Ghost) {
                acts.push(Act::Go(crumbs[start - 1].1.clone()));
            }
            x += b.w;
            canvas.text(x, area.y + 6.0, layout::FONT_SIZE, sep, color::text_faint());
            x += sep_w;
        }
        let last = crumbs.len().saturating_sub(1);
        for (i, (label, path)) in crumbs.iter().enumerate().skip(start) {
            let w = widths[i].min(area.x + area.w - x - 4.0).max(0.0);
            let b = Rect::new(x, area.y + 2.0, w, area.h - 4.0);
            if i == last {
                canvas.fill_rounded(b.x, b.y, b.w, b.h, 3.0, color::select());
                canvas.text_clipped(
                    b.x + 6.0,
                    area.y + 6.0,
                    layout::FONT_SIZE,
                    label,
                    color::text(),
                    b.w - 8.0,
                );
            } else {
                self.tip(b, &path.display().to_string());
                if self.button(canvas, b, label, ButtonStyle::Ghost) {
                    acts.push(Act::Go(path.clone()));
                }
            }
            x += w;
            if i != last {
                canvas.text(x, area.y + 6.0, layout::FONT_SIZE, sep, color::text_faint());
                x += sep_w;
            }
        }
    }

    fn draw_places(
        &mut self,
        canvas: &mut Canvas,
        side: Rect,
        fd: &FileDialog,
        acts: &mut Vec<Act>,
    ) {
        let mut y = side.y + 2.0;
        let mut section = u8::MAX;
        let headers = ["PROJECT", "THIS COMPUTER", "RECENT"];
        let prev = canvas.set_clip(Some((side.x, side.y, side.w, side.h)));
        for place in &fd.places {
            if place.section != section {
                section = place.section;
                if y > side.y + 4.0 {
                    y += 6.0;
                }
                canvas.text(
                    side.x + 8.0,
                    y + 4.0,
                    layout::SMALL_SIZE - 1.0,
                    headers[usize::from(section.min(2))],
                    color::text_faint(),
                );
                y += 20.0;
            }
            let row = Rect::new(side.x, y, side.w, PLACE_H);
            if row.y + row.h > side.y + side.h {
                break;
            }
            let current = place.path == fd.dir;
            let hot = self.hot(row);
            if current {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 4.0, color::select());
            } else if hot {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 4.0, color::hover());
            }
            let kind = if place.section == 0 && fd.places.first() == Some(place) {
                AssetKind::Scene
            } else {
                AssetKind::Folder
            };
            draw_tree_icon(canvas, kind, row.x + 16.0, row.y + row.h / 2.0);
            canvas.text_clipped(
                row.x + 30.0,
                row.y + 5.0,
                layout::FONT_SIZE,
                &place.label,
                if current {
                    color::text()
                } else {
                    color::text_dim()
                },
                row.w - 36.0,
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                self.tip(row, &place.path.display().to_string());
                if self.p.pressed {
                    acts.push(Act::Go(place.path.clone()));
                }
            }
            y += PLACE_H;
        }
        canvas.restore_clip(prev);
        // The divider between the sidebar and the listing.
        canvas.fill_rect(side.x + side.w + 4.0, side.y, 1.0, side.h, color::border());
    }

    fn draw_listing(
        &mut self,
        canvas: &mut Canvas,
        list: Rect,
        fd: &FileDialog,
        acts: &mut Vec<Act>,
    ) {
        canvas.fill_rounded(list.x, list.y, list.w, list.h, 4.0, color::field());
        canvas.stroke_rounded(list.x, list.y, list.w, list.h, 4.0, color::border(), 1.0);
        // Column header: click to sort.
        let head = Rect::new(list.x + 1.0, list.y + 1.0, list.w - 2.0, 24.0);
        canvas.fill_rect(head.x, head.y + head.h - 1.0, head.w, 1.0, color::border());
        let date_w = 104.0;
        let size_w = 76.0;
        let date_x = list.x + list.w - date_w - 8.0;
        let size_x = date_x - size_w;
        for (label, sort, x, w) in [
            ("Name", SortBy::Name, list.x + 8.0, size_x - list.x - 8.0),
            ("Size", SortBy::Size, size_x, size_w),
            ("Modified", SortBy::Modified, date_x, date_w),
        ] {
            let cell = Rect::new(x - 4.0, head.y + 2.0, w, head.h - 4.0);
            let active = fd.sort == sort;
            let hot = self.hot(cell);
            if hot {
                canvas.fill_rounded(cell.x, cell.y, cell.w, cell.h, 3.0, color::hover());
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    acts.push(Act::Sort(sort));
                }
            }
            canvas.text(
                x,
                head.y + 6.0,
                layout::SMALL_SIZE,
                label,
                if active {
                    color::text()
                } else {
                    color::text_dim()
                },
            );
            if active {
                // A small down arrow: the sort column.
                let ax = x + canvas.measure_text(layout::SMALL_SIZE, label) + 8.0;
                let ay = head.y + head.h / 2.0;
                canvas.fill_polygon(
                    &[(ax - 3.5, ay - 1.5), (ax + 3.5, ay - 1.5), (ax, ay + 2.5)],
                    color::text_dim(),
                );
            }
        }

        let rows = Rect::new(list.x, head.y + head.h, list.w, list.h - head.h - 2.0);
        let shown = fd.visible();
        let page = ((rows.h - 8.0) / ROW_H).floor().max(1.0) as usize;
        let content = shown.len() as f32 * ROW_H + 8.0;
        let mut scroll = fd.scroll;
        if self.hot(rows) && self.p.scroll != 0.0 {
            scroll -= self.p.scroll * ROW_H * 2.0;
        }
        if fd.reveal {
            if let Some(pos) = fd
                .selected
                .as_ref()
                .and_then(|s| shown.iter().position(|&i| &fd.entries[i].name == s))
            {
                let top = pos as f32 * ROW_H;
                if top < scroll {
                    scroll = top;
                } else if top + ROW_H + 8.0 > scroll + rows.h {
                    scroll = top + ROW_H + 8.0 - rows.h;
                }
            }
        }
        scroll = scroll.clamp(0.0, (content - rows.h).max(0.0));
        if let Some(live) = self.file_dialog_mut() {
            live.scroll = scroll;
            live.reveal = false;
            live.page = page;
        }

        let prev = canvas.set_clip(Some((rows.x, rows.y, rows.w, rows.h)));
        let now = SystemTime::now();
        let mut y = rows.y + 4.0 - scroll;
        let pressed = self.p.pressed;
        let double = self.p.double_click;
        for &i in &shown {
            let e = &fd.entries[i];
            let row = Rect::new(rows.x + 4.0, y, rows.w - 8.0, ROW_H);
            y += ROW_H;
            if row.y + row.h < rows.y || row.y > rows.y + rows.h {
                continue;
            }
            let hot = self.hot(row) && rows.contains(self.p.x, self.p.y);
            let selected = fd.selected.as_deref() == Some(e.name.as_str());
            if selected {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::select());
            } else if hot {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::hover());
            }
            let path = fd.dir.join(&e.name);
            draw_tree_icon(
                canvas,
                asset_kind(&path, e.dir),
                row.x + 14.0,
                row.y + row.h / 2.0,
            );
            canvas.text_clipped(
                row.x + 28.0,
                row.y + 5.0,
                layout::FONT_SIZE,
                &e.name,
                color::text(),
                size_x - row.x - 36.0,
            );
            if !e.dir {
                canvas.text(
                    size_x,
                    row.y + 6.0,
                    layout::SMALL_SIZE,
                    &files::human_size(e.size),
                    color::text_dim(),
                );
            }
            if let Some(t) = e.modified {
                canvas.text_clipped(
                    date_x,
                    row.y + 6.0,
                    layout::SMALL_SIZE,
                    &files::age_label(t, now),
                    color::text_dim(),
                    date_w,
                );
            }
            if hot {
                self.cursor = CursorHint::Pointer;
                if pressed {
                    let ignore = double && fd.ignore_double;
                    if ignore {
                        // The second click of the double click that opened
                        // this folder.
                    } else if e.dir {
                        acts.push(Act::Go(path));
                    } else if double {
                        acts.push(Act::Choose(e.name.clone()));
                    } else {
                        acts.push(Act::Select(e.name.clone()));
                    }
                }
            }
        }
        if pressed && rows.contains(self.p.x, self.p.y) {
            let opened = acts.iter().any(|a| matches!(a, Act::Go(_)));
            if let Some(live) = self.file_dialog_mut() {
                live.ignore_double = opened;
            }
        }
        if shown.is_empty() {
            let msg = if let Some(err) = fd.error.as_ref().filter(|_| fd.entries.is_empty()) {
                err.clone()
            } else if !fd.filter.is_empty() {
                format!("Nothing here matches \"{}\"", fd.filter)
            } else if fd.picks_folder() {
                "No subfolders — click Use This Folder to pick this one".to_string()
            } else if fd.saves() {
                format!("No {} files here yet — type a name below", fd.types_label())
            } else {
                format!("No {} files in this folder", fd.types_label())
            };
            canvas.text_clipped(
                rows.x + 14.0,
                rows.y + 14.0,
                layout::SMALL_SIZE,
                &msg,
                color::text_dim(),
                rows.w - 28.0,
            );
        }
        canvas.restore_clip(prev);
        // A thin scrollbar when the listing overflows.
        if content > rows.h {
            let track = rows.h - 8.0;
            let thumb = (rows.h / content * track).max(24.0);
            let t = scroll / (content - rows.h);
            canvas.fill_rounded(
                rows.x + rows.w - 6.0,
                rows.y + 4.0 + t * (track - thumb),
                3.0,
                thumb,
                1.5,
                color::border(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{EditKey, Pointer};
    use std::cmp::Ordering;

    /// `level.ron`, `scripts/{coin,player,enemy_ai}.py`, `scripts/ai/`.
    fn project(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("oberon-browser-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("scripts/ai")).unwrap();
        std::fs::write(root.join("level.ron"), "()").unwrap();
        for f in ["coin.py", "player.py", "enemy_ai.py"] {
            std::fs::write(root.join("scripts").join(f), "").unwrap();
        }
        std::fs::write(root.join("scripts/ai/brain.py"), "").unwrap();
        root
    }

    fn ui_in(root: &Path) -> Ui {
        let mut ui = Ui::new();
        ui.project_root = root.to_path_buf();
        ui
    }

    fn shown(ui: &Ui) -> Vec<String> {
        let fd = ui.file_dialog().unwrap();
        fd.visible()
            .iter()
            .map(|&i| fd.entries[i].name.clone())
            .collect()
    }

    fn frame(ui: &mut Ui, c: &mut Canvas, doc: &Document, p: Pointer) -> Vec<Intent> {
        ui.frame(c, doc, p, None)
    }

    fn click(
        ui: &mut Ui,
        c: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        double: bool,
    ) -> Vec<Intent> {
        let mut out = frame(
            ui,
            c,
            doc,
            Pointer {
                x,
                y,
                down: true,
                pressed: true,
                double_click: double,
                ..Pointer::default()
            },
        );
        out.extend(frame(
            ui,
            c,
            doc,
            Pointer {
                x,
                y,
                ..Pointer::default()
            },
        ));
        out
    }

    /// The centre of listing row `i` in a 1600×950 window.
    fn row_center(ui: &Ui, i: usize) -> (f32, f32) {
        let r = ui.dialog_rect(1600.0, 950.0).unwrap();
        let list_x = r.x + 12.0 + SIDEBAR_W + 10.0;
        let body_y = r.y + 48.0 + 36.0;
        (
            list_x + 80.0,
            body_y + 1.0 + 24.0 + 4.0 + (i as f32 + 0.5) * ROW_H,
        )
    }

    #[test]
    fn natural_order_counts_numbers() {
        assert_eq!(natural_cmp("level2.ron", "level10.ron"), Ordering::Less);
        assert_eq!(natural_cmp("Level2", "level2"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "b"), Ordering::Less);
        assert_eq!(natural_cmp("enemy", "enemy_boss"), Ordering::Less);
        assert_eq!(natural_cmp("v010", "v9"), Ordering::Greater);
    }

    #[test]
    fn typing_filters_and_enter_takes_the_match() {
        let root = project("filter");
        let doc = Document::new();
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScript, &doc);
        // Scripts open in scripts/, folders first.
        assert_eq!(ui.file_dialog_dir(), Some(root.join("scripts").as_path()));
        assert_eq!(shown(&ui), ["ai", "coin.py", "enemy_ai.py", "player.py"]);
        ui.type_text("pla");
        ui.sync_file_dialog();
        assert_eq!(shown(&ui), ["player.py"]);
        assert_eq!(ui.file_dialog_selection(), Some("player.py"));
        // "pla" isn't a file, but Enter takes the highlighted match.
        let intent = ui.edit_key(EditKey::Enter);
        assert_eq!(
            intent,
            Some(Intent::OpenScript(root.join("scripts/player.py")))
        );
        assert!(!ui.dialog_open());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn arrows_backspace_and_history_walk_folders() {
        let root = project("keys");
        let doc = Document::new();
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScript, &doc);
        // Down highlights the first entry (the ai folder); Enter opens it.
        assert!(ui.file_dialog_key(NavKey::Down));
        assert_eq!(ui.file_dialog_selection(), Some("ai"));
        assert_eq!(ui.edit_key(EditKey::Enter), None);
        assert_eq!(
            ui.file_dialog_dir(),
            Some(root.join("scripts/ai").as_path())
        );
        // Backspace in the empty field goes up, landing on the folder.
        ui.edit_key(EditKey::Backspace);
        assert_eq!(ui.file_dialog_dir(), Some(root.join("scripts").as_path()));
        assert_eq!(ui.file_dialog_selection(), Some("ai"));
        // Back / Forward walk the history.
        ui.file_dialog_key(NavKey::Back);
        assert_eq!(
            ui.file_dialog_dir(),
            Some(root.join("scripts/ai").as_path())
        );
        ui.file_dialog_key(NavKey::Forward);
        assert_eq!(ui.file_dialog_dir(), Some(root.join("scripts").as_path()));
        // Down on a file fills the name; End jumps to the last entry.
        ui.file_dialog_key(NavKey::Down);
        assert_eq!(ui.file_dialog_selection(), Some("coin.py"));
        assert_eq!(ui.file_dialog_name(), Some("coin.py"));
        ui.file_dialog_key(NavKey::Last);
        assert_eq!(ui.file_dialog_name(), Some("player.py"));
        // Tab completes a partial name to the match.
        ui.set_dialog_name(String::new(), false);
        ui.type_text("ene");
        ui.edit_key(EditKey::Tab);
        assert_eq!(ui.file_dialog_name(), Some("enemy_ai.py"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_new_folder_is_made_in_place_and_the_save_lands_inside() {
        let root = project("newfolder");
        let doc = Document::new();
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::SaveSceneAs, &doc);
        assert_eq!(ui.file_dialog_name(), Some("scene.ron"));
        ui.apply_dialog_act(Act::NewFolder);
        assert_eq!(ui.file_dialog_name(), Some("New Folder"));
        ui.type_text("levels"); // the suggestion is selected: typing replaces it
        assert_eq!(ui.edit_key(EditKey::Enter), None);
        assert!(root.join("levels").is_dir());
        assert_eq!(ui.file_dialog_dir(), Some(root.join("levels").as_path()));
        // The name being saved survived the detour.
        assert_eq!(ui.file_dialog_name(), Some("scene.ron"));
        let intent = ui.edit_key(EditKey::Enter);
        assert_eq!(intent, Some(Intent::SaveAs(root.join("levels/scene.ron"))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn escape_leaves_new_folder_naming_before_closing() {
        let root = project("escape");
        let doc = Document::new();
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::SaveSceneAs, &doc);
        ui.apply_dialog_act(Act::NewFolder);
        ui.edit_key(EditKey::Escape);
        assert!(ui.dialog_open());
        assert_eq!(ui.file_dialog_name(), Some("scene.ron"));
        ui.edit_key(EditKey::Escape);
        assert!(!ui.dialog_open());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clicks_select_open_and_choose() {
        let root = project("clicks");
        let doc = Document::new();
        let mut c = Canvas::new(1600, 950);
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScript, &doc);
        frame(&mut ui, &mut c, &doc, Pointer::default());
        // One click on a file selects it and fills the name.
        let (x, y) = row_center(&ui, 1);
        click(&mut ui, &mut c, &doc, x, y, false);
        assert_eq!(ui.file_dialog_selection(), Some("coin.py"));
        assert_eq!(ui.file_dialog_name(), Some("coin.py"));
        // One click on a folder opens it; the double click's second press
        // doesn't choose whatever lands under the pointer there.
        let (x, y) = row_center(&ui, 0);
        click(&mut ui, &mut c, &doc, x, y, false);
        assert_eq!(
            ui.file_dialog_dir(),
            Some(root.join("scripts/ai").as_path())
        );
        let out = click(&mut ui, &mut c, &doc, x, y, true);
        assert!(out.is_empty(), "{out:?}");
        assert!(ui.dialog_open());
        // A double click on a file chooses it.
        let out = click(&mut ui, &mut c, &doc, x, y, true);
        assert!(
            out.contains(&Intent::OpenScript(root.join("scripts/ai/brain.py"))),
            "{out:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_sidebar_and_breadcrumbs_navigate() {
        let root = project("places");
        let doc = Document::new();
        let mut c = Canvas::new(1600, 950);
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScript, &doc);
        let labels: Vec<String> = ui
            .file_dialog()
            .unwrap()
            .places
            .iter()
            .map(|p| p.label.clone())
            .collect();
        assert_eq!(labels[1], "Scripts", "{labels:?}");
        // The project place is the first sidebar row.
        let r = ui.dialog_rect(1600.0, 950.0).unwrap();
        let body_y = r.y + 48.0 + 36.0;
        click(
            &mut ui,
            &mut c,
            &doc,
            r.x + 60.0,
            body_y + 2.0 + 20.0 + 12.0,
            false,
        );
        assert_eq!(ui.file_dialog_dir(), Some(root.as_path()));
        // Breadcrumbs: project › scripts › ai, each a way back.
        ui.dialog_go(root.join("scripts/ai"));
        let crumbs = ui.file_dialog().unwrap().crumbs(&root);
        let names: Vec<&str> = crumbs.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(names[1..], ["scripts", "ai"]);
        assert_eq!(crumbs[0].1, root);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dialogs_remember_where_they_were() {
        let root = project("memory");
        let doc = Document::new();
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScript, &doc);
        ui.dialog_go(root.join("scripts/ai"));
        let intent = ui.submit_dialog("brain");
        assert_eq!(
            intent,
            Some(Intent::OpenScript(root.join("scripts/ai/brain.py")))
        );
        ui.open_dialog_kind(DialogKind::OpenScript, &doc);
        assert_eq!(
            ui.file_dialog_dir(),
            Some(root.join("scripts/ai").as_path())
        );
        // A different kind keeps its own place.
        ui.close_dialog();
        ui.open_dialog_kind(DialogKind::OpenScene, &doc);
        assert_eq!(ui.file_dialog_dir(), Some(root.as_path()));
        assert_eq!(ui.prefs.recent_dirs.first(), Some(&root.join("scripts/ai")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_dialogs_start_on_the_current_file() {
        let root = project("current");
        let mut doc = Document::new();
        doc.path = Some(root.join("level.ron"));
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScene, &doc);
        assert_eq!(ui.file_dialog_selection(), Some("level.ron"));
        assert_eq!(ui.file_dialog_name(), Some("level.ron"));
        assert_eq!(
            ui.edit_key(EditKey::Enter),
            Some(Intent::Open(root.join("level.ron")))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_browse_picks_a_folder_and_returns() {
        let root = project("browse");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        let mut ui = ui_in(&root);
        ui.open_file_dialog(DialogKind::BuildOutput, None);
        // Only folders are listed.
        assert_eq!(shown(&ui), ["dist", "scripts"]);
        ui.dialog_go(root.join("dist"));
        assert_eq!(ui.submit_dialog(""), None);
        assert!(matches!(
            ui.dialog,
            Some(Dialog::Settings(SettingsPage::Build))
        ));
        assert_eq!(ui.build_settings().output, "dist");
        // Cancel goes back to Settings too.
        ui.open_file_dialog(DialogKind::BuildScene, None);
        ui.close_dialog();
        assert!(matches!(
            ui.dialog,
            Some(Dialog::Settings(SettingsPage::Build))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_drops_jump_to_the_file() {
        let root = project("drop");
        let doc = Document::new();
        let mut ui = ui_in(&root);
        ui.open_dialog_kind(DialogKind::OpenScene, &doc);
        assert!(ui.file_dialog_drop(&root.join("scripts/ai/brain.py")));
        assert_eq!(
            ui.file_dialog_dir(),
            Some(root.join("scripts/ai").as_path())
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
