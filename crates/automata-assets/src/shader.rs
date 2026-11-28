//! Custom shader files (`.wgsl`): read, checked and compiled through a
//! cache that notices when a file changes, so saving a shader shows the
//! change at once (hot reload).
//!
//! A shader is checked when it's read ([`automata_renderer::check_shader`]:
//! the fixed interface, then naga's parser and validator, then the WebGL2
//! translation), so a broken one never reaches the GPU: what uses it falls
//! back to the standard look and the problem goes to
//! [`take_shader_reports`] (Oberon's Console) and the log.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

pub use automata_renderer::ShaderKind;
use automata_renderer::{Renderer, Shader, ShaderError};

/// Whether `path` names a shader file (`.wgsl`).
#[must_use]
pub fn is_shader_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wgsl"))
}

/// A shader file as read: its source and whether it passed the checks.
#[derive(Debug)]
pub struct ShaderAsset {
    /// The file.
    pub path: PathBuf,
    /// How it was asked for (the name in messages).
    pub name: String,
    /// What it shapes.
    pub kind: ShaderKind,
    /// The WGSL in the file.
    pub source: String,
    /// Why it didn't pass the checks, if it didn't.
    pub error: Option<ShaderError>,
    /// The file's modification time when it was read.
    stamp: Option<SystemTime>,
    gpu: OnceLock<Option<Shader>>,
}

impl ShaderAsset {
    /// Reads and checks `source` (from `path`, called `name` in
    /// messages).
    #[must_use]
    pub fn new(path: PathBuf, name: &str, kind: ShaderKind, source: String) -> Self {
        let error = automata_renderer::check_shader(kind, name, &source).err();
        let stamp = crate::modified(&path);
        Self {
            path,
            name: name.to_string(),
            kind,
            source,
            error,
            stamp,
            gpu: OnceLock::new(),
        }
    }

    /// Whether the file changed since it was read.
    #[must_use]
    pub fn stale(&self) -> bool {
        crate::modified(&self.path) != self.stamp
    }

    /// The compiled shader, compiled on first use; `None` when it didn't
    /// pass the checks or the GPU refused it (reported once).
    pub fn gpu(&self, renderer: &Renderer) -> Option<Shader> {
        self.gpu
            .get_or_init(|| {
                if self.error.is_some() {
                    return None;
                }
                match renderer.create_shader(self.kind, &self.name, &self.source) {
                    Ok(shader) => Some(shader),
                    Err(e) => {
                        report(&self.path, Err(e.to_string()));
                        None
                    }
                }
            })
            .clone()
    }
}

/// Something to tell the user about a shader file: it (re)loaded fine, or
/// what's wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderReport {
    /// The file.
    pub path: PathBuf,
    /// `Ok(message)` when it loaded, `Err(message)` (`file:line:col:
    /// what`) when it didn't.
    pub result: Result<String, String>,
}

struct Entry {
    stamp: Option<SystemTime>,
    asset: Arc<ShaderAsset>,
}

static SHADERS: Mutex<BTreeMap<(PathBuf, bool), Entry>> = Mutex::new(BTreeMap::new());
static REPORTS: Mutex<Vec<ShaderReport>> = Mutex::new(Vec::new());
/// Whether each file's last load failed (to say when it's fixed).
static FAILED: Mutex<BTreeMap<PathBuf, bool>> = Mutex::new(BTreeMap::new());

fn kind_key(kind: ShaderKind) -> bool {
    kind == ShaderKind::Post
}

fn report(path: &Path, result: Result<String, String>) {
    match &result {
        Ok(m) => tracing::info!("{m}"),
        Err(e) => tracing::warn!("shader {e}"),
    }
    REPORTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(ShaderReport {
            path: path.to_path_buf(),
            result,
        });
}

/// The shader reports since the last call (Oberon shows them in its
/// Console).
pub fn take_shader_reports() -> Vec<ShaderReport> {
    std::mem::take(&mut *REPORTS.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Which kind of shader `source` is: a post effect when it defines
/// `fn post(`, otherwise a material.
#[must_use]
pub fn kind_of_source(source: &str) -> ShaderKind {
    let post = source.split("fn").skip(1).any(|rest| {
        rest.starts_with(char::is_whitespace)
            && rest
                .trim_start()
                .strip_prefix("post")
                .is_some_and(|r| r.trim_start().starts_with('('))
    });
    if post {
        ShaderKind::Post
    } else {
        ShaderKind::Material
    }
}

/// Loads the shader at `path` (relative paths resolve like model paths),
/// read again once the file changes.
///
/// # Errors
/// The file can't be read. (A shader that reads but breaks the rules
/// loads, with its [`ShaderAsset::error`] set.)
pub fn load_shader(path: &str, kind: ShaderKind) -> Result<Arc<ShaderAsset>, String> {
    load_shader_file(&crate::resolve(path), path, kind)
}

/// Loads the shader file at `file` (called `name` in messages), read again
/// once it changes.
///
/// # Errors
/// The file can't be read.
pub fn load_shader_file(
    file: &Path,
    name: &str,
    kind: ShaderKind,
) -> Result<Arc<ShaderAsset>, String> {
    let key = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let stamp = crate::modified(&key);
    if stamp.is_none() {
        return Err(format!("shader {name} not found"));
    }
    let cache_key = (key.clone(), kind_key(kind));
    {
        let cache = SHADERS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = cache.get(&cache_key) {
            if entry.stamp == stamp {
                return Ok(entry.asset.clone());
            }
        }
    }
    let source = std::fs::read_to_string(&key).map_err(|e| format!("shader {name}: {e}"))?;
    let reloaded = SHADERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(&cache_key);
    let asset = Arc::new(ShaderAsset::new(key.clone(), name, kind, source));
    let failed_before = FAILED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.clone(), asset.error.is_some())
        .unwrap_or(false);
    match &asset.error {
        Some(e) => report(&key, Err(e.to_string())),
        None if reloaded || failed_before => {
            report(&key, Ok(format!("{name}: shader reloaded")));
        }
        None => {}
    }
    SHADERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            cache_key,
            Entry {
                stamp,
                asset: asset.clone(),
            },
        );
    Ok(asset)
}

/// Whether any cached shader's file changed since it was read (so what
/// uses it should be rebuilt).
pub fn shaders_changed() -> bool {
    SHADERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|((path, _), e)| crate::modified(path) != e.stamp)
}

/// Forgets every cached shader.
pub fn clear_shaders() {
    SHADERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("automata-shader-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn shaders_load_check_and_reload_when_the_file_changes() {
        let dir = temp_dir("reload");
        let file = dir.join("glow.wgsl");
        std::fs::write(
            &file,
            "fn post(in: PostInput) -> vec3<f32> { return in.color; }",
        )
        .unwrap();
        let a = load_shader_file(&file, "shaders/glow.wgsl", ShaderKind::Post).unwrap();
        assert!(a.error.is_none(), "{:?}", a.error);
        let b = load_shader_file(&file, "shaders/glow.wgsl", ShaderKind::Post).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "cached while unchanged");
        // Asked for as a material, the same file is a different shader
        // (and not one: it has no `material`).
        let m = load_shader_file(&file, "shaders/glow.wgsl", ShaderKind::Material).unwrap();
        assert!(m
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("fn material")));

        // A broken save: reported with its line, then fixed.
        let _ = take_shader_reports();
        std::fs::write(
            &file,
            "fn post(in: PostInput) -> vec3<f32> {\n    return in.colr;\n}",
        )
        .unwrap();
        bump(&file);
        assert!(shaders_changed());
        let broken = load_shader_file(&file, "shaders/glow.wgsl", ShaderKind::Post).unwrap();
        let e = broken.error.clone().expect("broken");
        assert_eq!(e.line, Some(2));
        let reports = take_shader_reports();
        assert!(
            reports.iter().any(|r| r
                .result
                .as_ref()
                .is_err_and(|m| m.starts_with("shaders/glow.wgsl:2:"))),
            "{reports:?}"
        );
        std::fs::write(
            &file,
            "fn post(in: PostInput) -> vec3<f32> { return in.color * 0.5; }",
        )
        .unwrap();
        bump(&file);
        let fixed = load_shader_file(&file, "shaders/glow.wgsl", ShaderKind::Post).unwrap();
        assert!(fixed.error.is_none(), "{:?}", fixed.error);
        let reports = take_shader_reports();
        assert!(
            reports
                .iter()
                .any(|r| r.result.as_ref().is_ok_and(|m| m.contains("reloaded"))),
            "{reports:?}"
        );
        assert!(
            load_shader_file(&dir.join("missing.wgsl"), "missing.wgsl", ShaderKind::Post).is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Moves a file's modification time on, so a rewrite within the
    /// filesystem's timestamp resolution still reads as a change.
    fn bump(file: &Path) {
        static BUMPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = BUMPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let later = crate::modified(file).unwrap() + std::time::Duration::from_secs(10 * n);
        let f = std::fs::File::options().write(true).open(file).unwrap();
        f.set_modified(later).unwrap();
    }

    #[test]
    fn shader_paths_are_wgsl_files() {
        assert!(is_shader_path(Path::new("shaders/a.wgsl")));
        assert!(is_shader_path(Path::new("A.WGSL")));
        assert!(!is_shader_path(Path::new("a.glsl")));
    }

    #[test]
    fn shader_kinds_come_from_the_entry_point() {
        assert_eq!(
            kind_of_source("fn post(in: PostInput) -> vec3<f32> { return in.color; }"),
            ShaderKind::Post
        );
        assert_eq!(
            kind_of_source("fn  post (in: PostInput) -> vec3<f32> { return in.color; }"),
            ShaderKind::Post
        );
        assert_eq!(
            kind_of_source(automata_renderer::MATERIAL_TEMPLATE),
            ShaderKind::Material
        );
        assert_eq!(
            kind_of_source(automata_renderer::POST_TEMPLATE),
            ShaderKind::Post
        );
        assert_eq!(kind_of_source("fn postpone() {}"), ShaderKind::Material);
    }
}
