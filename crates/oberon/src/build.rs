//! Game export from inside Oberon (Build menu, Ctrl+Shift+B).
//!
//! Exports run on a worker thread so the editor stays responsive: the job
//! streams [`BuildEvent`]s back over a channel and the app drains them into
//! the console each frame. The heavy lifting lives in `automata-export`, the
//! same library behind the `automata-export` command-line tool.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

pub use automata_export::{BuildConfig, Level, Target};

/// Something a running job reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildEvent {
    /// A progress or problem line for the console.
    Log(Level, String),
    /// One target finished: the executable, or why it failed.
    Finished(Target, Result<PathBuf, String>),
}

/// What a job does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobKind {
    /// Export the project.
    Export,
    /// Download and install export templates.
    Download,
}

/// A running export or template download.
#[derive(Debug)]
pub struct BuildJob {
    /// What it does.
    pub kind: JobKind,
    /// The targets it covers.
    pub targets: Vec<Target>,
    /// The project's build output folder.
    pub output: PathBuf,
    /// When it started.
    pub started: Instant,
    rx: Receiver<BuildEvent>,
    /// Successful targets so far.
    pub succeeded: Vec<(Target, PathBuf)>,
    /// Failed targets so far.
    pub failed: Vec<(Target, String)>,
}

impl BuildJob {
    /// Starts exporting `project` for each of `targets`.
    #[must_use]
    pub fn export(project: PathBuf, cfg: BuildConfig, targets: Vec<Target>) -> Self {
        let (tx, rx) = mpsc::channel();
        let output = project.join(&cfg.output);
        let list = targets.clone();
        std::thread::Builder::new()
            .name("oberon-export".into())
            .spawn(move || {
                for target in list {
                    let send = |e: BuildEvent| {
                        let _ = tx.send(e);
                    };
                    let Some(template) = template_for(target, &mut |l, t| {
                        send(BuildEvent::Log(l, t));
                    }) else {
                        send(BuildEvent::Finished(
                            target,
                            Err(format!(
                                "no {} export template installed — Build ▸ Download Export Templates",
                                target.label()
                            )),
                        ));
                        continue;
                    };
                    let result = automata_export::export(
                        &project,
                        &cfg,
                        target,
                        &template,
                        &mut |m| send(BuildEvent::Log(m.level, m.text)),
                    )
                    .map(|out| out.executable);
                    send(BuildEvent::Finished(target, result));
                }
            })
            .ok();
        Self::new(JobKind::Export, targets, output, rx)
    }

    /// Starts downloading the templates for `targets`.
    #[must_use]
    pub fn download(targets: Vec<Target>) -> Self {
        let (tx, rx) = mpsc::channel();
        let list = targets.clone();
        std::thread::Builder::new()
            .name("oberon-templates".into())
            .spawn(move || {
                for target in list {
                    let _ = tx.send(BuildEvent::Log(
                        Level::Info,
                        format!("Downloading the {} export template…", target.label()),
                    ));
                    let result = automata_export::download_template(target);
                    let _ = tx.send(BuildEvent::Finished(target, result));
                }
            })
            .ok();
        Self::new(
            JobKind::Download,
            targets,
            automata_export::templates_dir(),
            rx,
        )
    }

    fn new(kind: JobKind, targets: Vec<Target>, output: PathBuf, rx: Receiver<BuildEvent>) -> Self {
        Self {
            kind,
            targets,
            output,
            started: Instant::now(),
            rx,
            succeeded: Vec::new(),
            failed: Vec::new(),
        }
    }

    /// Drains the events that arrived since the last call. The job is done
    /// when the second value is true.
    pub fn poll(&mut self) -> (Vec<BuildEvent>, bool) {
        let mut events = Vec::new();
        let mut closed = false;
        loop {
            match self.rx.try_recv() {
                Ok(e) => {
                    if let BuildEvent::Finished(t, r) = &e {
                        match r {
                            Ok(p) => self.succeeded.push((*t, p.clone())),
                            Err(msg) => self.failed.push((*t, msg.clone())),
                        }
                    }
                    events.push(e);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        let done = closed || self.succeeded.len() + self.failed.len() >= self.targets.len();
        (events, done)
    }

    /// A one-line description of the job for the status bar.
    #[must_use]
    pub fn label(&self) -> String {
        let names: Vec<&str> = self.targets.iter().map(|t| t.label()).collect();
        match self.kind {
            JobKind::Export => format!("Building for {}…", names.join(" + ")),
            JobKind::Download => format!("Downloading templates ({})…", names.join(", ")),
        }
    }
}

/// The installed template for `target`. Falls back to a development
/// template built from an `automata-player` next to the Oberon executable
/// (a `cargo build` checkout), so exports work before templates are
/// downloaded.
fn template_for(target: Target, log: &mut dyn FnMut(Level, String)) -> Option<PathBuf> {
    if let Some(dir) = automata_export::find_template(target) {
        return Some(dir);
    }
    if Target::host() != Some(target) {
        return None;
    }
    let player = sibling_player(target)?;
    match automata_export::install_dev_template(&player, target) {
        Ok(dir) => {
            log(
                Level::Warning,
                format!(
                    "Using a development template from {} — the build needs Python with automata installed. Download the export templates for a standalone game.",
                    player.display()
                ),
            );
            Some(dir)
        }
        Err(e) => {
            log(Level::Warning, e);
            None
        }
    }
}

/// `automata-player` beside the running executable, if built.
fn sibling_player(target: Target) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join(target.player_file());
    p.is_file().then_some(p)
}

/// How a target's template stands, for the Build Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateState {
    /// Installed with a bundled Python: builds are standalone.
    Ready,
    /// A development template: builds use the Python on `PATH`.
    Development,
    /// Not installed.
    Missing,
}

/// The template state of `target`.
#[must_use]
pub fn template_state(target: Target) -> TemplateState {
    match automata_export::find_template(target) {
        Some(dir) if automata_export::template_has_runtime(&dir) => TemplateState::Ready,
        Some(_) => TemplateState::Development,
        None => TemplateState::Missing,
    }
}

/// The settings to export with: `build.ron`, or defaults that start in the
/// open scene.
#[must_use]
pub fn config_for(project: &Path, scene: Option<&Path>) -> BuildConfig {
    let mut cfg = BuildConfig::for_project(project, scene);
    if cfg.start_scene.is_none() && cfg.main_script.is_none() {
        cfg.start_scene = scene
            .map(|s| s.strip_prefix(project).unwrap_or(s))
            .map(|p| p.to_string_lossy().replace('\\', "/"));
    }
    cfg
}

/// Where templates are installed, for display.
#[must_use]
pub fn templates_dir_label() -> String {
    format!(
        "Installed in {}",
        automata_export::templates_dir().display()
    )
}

/// Opens a folder in the system file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_starts_in_the_open_scene() {
        let dir = std::env::temp_dir().join("oberon-build-config");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("levels")).unwrap();
        let cfg = config_for(&dir, Some(&dir.join("levels").join("one.ron")));
        assert_eq!(cfg.start_scene.as_deref(), Some("levels/one.ron"));
        assert_eq!(cfg.name, "Oberon Build Config");
        // A saved build.ron wins.
        let mut saved = cfg.clone();
        saved.start_scene = Some("menu.ron".into());
        saved.save(&dir).unwrap();
        let cfg = config_for(&dir, Some(&dir.join("levels").join("one.ron")));
        assert_eq!(cfg.start_scene.as_deref(), Some("menu.ron"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_export_reports_every_target() {
        let dir = std::env::temp_dir().join("oberon-build-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // No start scene and no main script: validation fails.
        let mut job = BuildJob::export(dir.clone(), BuildConfig::default(), Target::ALL.to_vec());
        let start = Instant::now();
        let mut logs = 0;
        loop {
            let (events, done) = job.poll();
            logs += events
                .iter()
                .filter(|e| matches!(e, BuildEvent::Log(..)))
                .count();
            if done {
                break;
            }
            assert!(start.elapsed().as_secs() < 20, "the job finishes");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(job.failed.len(), 2);
        assert_eq!(job.succeeded, Vec::new());
        assert!(logs > 0 || job.failed.iter().all(|(_, m)| m.contains("template")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
