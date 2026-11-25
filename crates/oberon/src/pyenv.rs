//! The project's Python: finding it, and making it.
//!
//! Every Oberon project gets its own virtual environment in `.venv`, with
//! the `automata-engine` wheel that matches this Oberon. Play, `oberon
//! run`, `oberon test`, `oberon python` and the code editor's language
//! server all use it, so a script behaves the same everywhere and IDEs see
//! the engine's types. The preferences' Python path still overrides it.
//!
//! The Oberon download carries what a machine with no Python needs, next to
//! the executable:
//!
//! ```text
//! oberon[.exe]
//! python/            a standalone CPython (python-build-standalone)
//! wheels/            automata-engine, basedpyright, pytest and their deps
//! ```
//!
//! `OBERON_PYTHON` and `OBERON_WHEELS` point elsewhere (a source checkout
//! uses them, or `target/wheels` from `maturin build`).

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `automata-engine` version this Oberon was built with; new projects
/// pin it.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The distribution name of the engine's Python package.
pub const ENGINE_PACKAGE: &str = "automata-engine";

/// Development tools installed into new venvs when their wheels are
/// available: the language server and the test runner.
pub const DEV_PACKAGES: &[&str] = &["basedpyright", "pytest"];

/// The folder of a virtual environment's executables.
#[must_use]
pub fn bin_dir(venv: &Path) -> PathBuf {
    venv.join(if cfg!(windows) { "Scripts" } else { "bin" })
}

/// A virtual environment's interpreter (which may not exist).
#[must_use]
pub fn venv_python(venv: &Path) -> PathBuf {
    bin_dir(venv).join(if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    })
}

/// A project's virtual environment folder.
#[must_use]
pub fn venv_dir(root: &Path) -> PathBuf {
    root.join(".venv")
}

/// The project folder holding `start` (a scene file or any folder inside the
/// project): the nearest folder upwards with a `pyproject.toml`, a `.venv`
/// or a `build.ron`. `None` when there is none.
#[must_use]
pub fn project_root(start: &Path) -> Option<PathBuf> {
    let start = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start
    };
    loop {
        if ["pyproject.toml", ".venv", "build.ron"]
            .iter()
            .any(|marker| dir.join(marker).exists())
        {
            return Some(automata_cli::plain_path(dir));
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// The interpreter a project uses: the preferences' override, else the
/// project's `.venv`, else the system's `python3` (`python` on Windows).
#[must_use]
pub fn project_python(prefs_override: Option<&str>, root: &Path) -> String {
    if let Some(p) = prefs_override.map(str::trim).filter(|p| !p.is_empty()) {
        return p.to_string();
    }
    let venv = venv_python(&venv_dir(root));
    if venv.is_file() {
        return venv.to_string_lossy().into_owned();
    }
    crate::bridge::default_python().to_string()
}

/// What the Oberon download brought along (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bundle {
    /// The standalone interpreter.
    pub python: Option<PathBuf>,
    /// The folder of wheels to install from, offline.
    pub wheels: Option<PathBuf>,
}

impl Bundle {
    /// Finds the bundle next to the running executable, honouring the
    /// `OBERON_PYTHON` / `OBERON_WHEELS` overrides.
    #[must_use]
    pub fn find() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        Self::find_in(
            exe_dir.as_deref(),
            std::env::var_os("OBERON_PYTHON").map(PathBuf::from),
            std::env::var_os("OBERON_WHEELS").map(PathBuf::from),
        )
    }

    /// [`Self::find`] with the executable's folder and overrides given.
    #[must_use]
    pub fn find_in(
        exe_dir: Option<&Path>,
        python: Option<PathBuf>,
        wheels: Option<PathBuf>,
    ) -> Self {
        let python = python.filter(|p| p.is_file()).or_else(|| {
            let home = exe_dir?.join("python");
            let candidates = if cfg!(windows) {
                vec![home.join("python.exe")]
            } else {
                vec![
                    home.join("bin").join("python3"),
                    home.join("bin").join("python"),
                ]
            };
            candidates.into_iter().find(|p| p.is_file())
        });
        let wheels = wheels
            .filter(|d| d.is_dir())
            .or_else(|| Some(exe_dir?.join("wheels")).filter(|d| d.is_dir()));
        Self { python, wheels }
    }

    /// The engine wheel in [`Self::wheels`] for this Oberon's version.
    #[must_use]
    pub fn engine_wheel(&self) -> Option<PathBuf> {
        let prefix = format!("automata_engine-{ENGINE_VERSION}-");
        wheels_in(self.wheels.as_deref()?)
            .into_iter()
            .find(|w| file_name(w).starts_with(&prefix))
    }

    /// The development packages with a wheel in [`Self::wheels`].
    #[must_use]
    pub fn dev_packages(&self) -> Vec<&'static str> {
        let Some(dir) = &self.wheels else {
            return Vec::new();
        };
        let wheels = wheels_in(dir);
        DEV_PACKAGES
            .iter()
            .copied()
            .filter(|pkg| {
                let prefix = format!("{}-", pkg.replace('-', "_"));
                wheels.iter().any(|w| file_name(w).starts_with(&prefix))
            })
            .collect()
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn wheels_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "whl"))
        .collect();
    out.sort();
    out
}

/// The `automata-engine` version installed in `venv`, read from its
/// `.dist-info` folder (no interpreter started).
#[must_use]
pub fn installed_engine_version(venv: &Path) -> Option<String> {
    let lib = if cfg!(windows) {
        vec![venv.join("Lib").join("site-packages")]
    } else {
        std::fs::read_dir(venv.join("lib"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("site-packages"))
            .collect()
    };
    lib.iter().find_map(|site| {
        std::fs::read_dir(site).ok()?.flatten().find_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix("automata_engine-")?;
            rest.strip_suffix(".dist-info").map(str::to_string)
        })
    })
}

/// How a project's venv compares with this Oberon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenvState {
    /// No `.venv` (run `oberon setup`).
    Missing,
    /// A venv without the engine.
    NoEngine,
    /// The engine at another version than this Oberon's.
    Outdated(String),
    /// Ready.
    Ready,
}

/// Checks the project's venv without starting Python.
#[must_use]
pub fn venv_state(root: &Path) -> VenvState {
    let venv = venv_dir(root);
    if !venv_python(&venv).is_file() {
        return VenvState::Missing;
    }
    match installed_engine_version(&venv) {
        None => VenvState::NoEngine,
        Some(v) if v == ENGINE_VERSION => VenvState::Ready,
        Some(v) => VenvState::Outdated(v),
    }
}

/// Options for [`setup_venv`].
#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    /// The interpreter to base the venv on (default: the bundled one, else
    /// the system's).
    pub base_python: Option<PathBuf>,
    /// Reinstall the engine even when the venv has one (an upgrade).
    pub upgrade: bool,
    /// Don't use `uv` even if it's installed.
    pub no_uv: bool,
}

/// Makes (or repairs, or upgrades) the project's `.venv`: from the bundled
/// interpreter and wheels, offline, with `uv` when it's on the PATH and
/// `python -m venv` / `pip` otherwise. `log` hears each step.
///
/// Without bundled wheels the venv is still made, and the engine comes
/// from the package index if it can (a source checkout sets
/// `OBERON_WHEELS` instead).
///
/// # Errors
///
/// Returns what failed, with the command's output.
pub fn setup_venv(
    root: &Path,
    bundle: &Bundle,
    options: &SetupOptions,
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    // Commands run inside the project, so every path must be absolute.
    let root = &std::fs::canonicalize(root)
        .map(automata_cli::plain_path)
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let venv = venv_dir(root);
    let python = venv_python(&venv);
    let uv = if options.no_uv {
        None
    } else {
        crate::lsp::find_executable("uv", &[])
    };
    if python.is_file() {
        log(&format!("using {}", python.display()));
    } else {
        let base = options
            .base_python
            .clone()
            .or_else(|| bundle.python.clone())
            .or_else(|| crate::lsp::find_executable(crate::bridge::default_python(), &[]))
            .ok_or_else(|| {
                "no Python found: this Oberon has no bundled python/ folder and there's no \
                 python3 on the PATH (pass --python PATH)"
                    .to_string()
            })?;
        log(&format!("creating .venv from {}", base.display()));
        let mut cmd = match &uv {
            Some(uv) => {
                let mut c = Command::new(uv);
                c.arg("venv").arg("--python").arg(&base).arg(&venv);
                c
            }
            None => {
                let mut c = Command::new(&base);
                c.args(["-m", "venv"]).arg(&venv);
                c
            }
        };
        run(&mut cmd, root)?;
    }

    let state = venv_state(root);
    let engine = match bundle.engine_wheel() {
        Some(wheel) => wheel.to_string_lossy().into_owned(),
        None => format!("{ENGINE_PACKAGE}=={ENGINE_VERSION}"),
    };
    let reinstall = options.upgrade && state == VenvState::Ready;
    let mut packages: Vec<String> = Vec::new();
    if options.upgrade || state != VenvState::Ready {
        packages.push(engine.clone());
    }
    let bin = bin_dir(&venv);
    for pkg in bundle.dev_packages() {
        let tool = if pkg == "basedpyright" {
            "basedpyright-langserver"
        } else {
            pkg
        };
        let installed = crate::lsp::find_executable(tool, std::slice::from_ref(&bin))
            .is_some_and(|p| p.starts_with(&bin));
        if !installed {
            packages.push(pkg.to_string());
        }
    }
    if packages.is_empty() {
        log("everything is installed");
        return Ok(());
    }
    log(&format!("installing {}", short_names(&packages).join(", ")));
    let install = |packages: &[String], force: bool| -> Result<(), String> {
        let mut cmd = match &uv {
            Some(uv) => {
                let mut c = Command::new(uv);
                c.args(["pip", "install", "--python"]).arg(&python);
                if force {
                    c.args(["--reinstall-package", ENGINE_PACKAGE]);
                }
                c
            }
            None => {
                let mut c = Command::new(&python);
                c.args(["-m", "pip", "install", "--disable-pip-version-check"]);
                if force {
                    c.arg("--force-reinstall");
                }
                c
            }
        };
        if let Some(dir) = &bundle.wheels {
            cmd.arg("--no-index").arg("--find-links").arg(dir);
        }
        cmd.args(packages);
        run(&mut cmd, root)
    };
    let result = if reinstall && uv.is_none() {
        // pip forces every package it's given: the engine on its own first.
        install(std::slice::from_ref(&engine), true).and_then(|()| {
            let rest: Vec<String> = packages.into_iter().filter(|p| *p != engine).collect();
            if rest.is_empty() {
                Ok(())
            } else {
                install(&rest, false)
            }
        })
    } else {
        install(&packages, reinstall)
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if bundle.wheels.is_none() => Err(format!(
            "{e}\nthere are no bundled wheels: set OBERON_WHEELS to a folder with the \
             automata-engine {ENGINE_VERSION} wheel (from `maturin build`), then run `oberon setup`"
        )),
        Err(e) => Err(e),
    }
}

/// Package names for messages: wheel paths become their file names.
fn short_names(packages: &[String]) -> Vec<String> {
    packages
        .iter()
        .map(|p| {
            if Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("whl"))
            {
                file_name(Path::new(p))
            } else {
                p.clone()
            }
        })
        .collect()
}

/// Runs `cmd` in `cwd`, turning a failure into its output.
fn run(cmd: &mut Command, cwd: &Path) -> Result<(), String> {
    let shown = format!("{cmd:?}");
    let out = cmd
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("cannot run {shown}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{shown} failed ({}):\n{}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon-pyenv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_project_root_is_the_nearest_folder_with_a_marker() {
        let root = temp("root");
        std::fs::write(root.join("pyproject.toml"), "").unwrap();
        let deep = root.join("levels").join("forest");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("level.ron"), "").unwrap();
        let found = project_root(&deep.join("level.ron")).unwrap();
        assert_eq!(
            found,
            automata_cli::plain_path(std::fs::canonicalize(&root).unwrap())
        );
        assert_eq!(project_root(&deep).unwrap(), found);
    }

    #[test]
    fn the_venv_is_used_unless_the_preferences_override_it() {
        let root = temp("python");
        assert_eq!(project_python(None, &root), crate::bridge::default_python());
        let python = venv_python(&venv_dir(&root));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        assert_eq!(project_python(None, &root), python.to_string_lossy());
        assert_eq!(project_python(Some(" /opt/py "), &root), "/opt/py");
        assert_eq!(project_python(Some(""), &root), python.to_string_lossy());
    }

    #[test]
    fn venv_states_come_from_dist_info_folders() {
        let root = temp("state");
        assert_eq!(venv_state(&root), VenvState::Missing);
        let venv = venv_dir(&root);
        let python = venv_python(&venv);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        assert_eq!(venv_state(&root), VenvState::NoEngine);
        let site = if cfg!(windows) {
            venv.join("Lib").join("site-packages")
        } else {
            venv.join("lib").join("python3.12").join("site-packages")
        };
        std::fs::create_dir_all(site.join("automata_engine-0.0.1.dist-info")).unwrap();
        assert_eq!(venv_state(&root), VenvState::Outdated("0.0.1".into()));
        std::fs::remove_dir_all(site.join("automata_engine-0.0.1.dist-info")).unwrap();
        std::fs::create_dir_all(site.join(format!("automata_engine-{ENGINE_VERSION}.dist-info")))
            .unwrap();
        assert_eq!(venv_state(&root), VenvState::Ready);
    }

    #[test]
    fn the_bundle_is_found_next_to_the_executable() {
        let dir = temp("bundle");
        assert_eq!(Bundle::find_in(Some(&dir), None, None), Bundle::default());
        let python = if cfg!(windows) {
            dir.join("python").join("python.exe")
        } else {
            dir.join("python").join("bin").join("python3")
        };
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        let wheels = dir.join("wheels");
        std::fs::create_dir_all(&wheels).unwrap();
        for w in [
            format!("automata_engine-{ENGINE_VERSION}-cp310-abi3-manylinux_2_34_x86_64.whl"),
            "automata_engine-0.0.1-cp310-abi3-manylinux_2_34_x86_64.whl".to_string(),
            "pytest-8.3.0-py3-none-any.whl".to_string(),
        ] {
            std::fs::write(wheels.join(w), "").unwrap();
        }
        let bundle = Bundle::find_in(Some(&dir), None, None);
        assert_eq!(bundle.python.as_deref(), Some(python.as_path()));
        assert_eq!(bundle.wheels.as_deref(), Some(wheels.as_path()));
        let engine = bundle.engine_wheel().unwrap();
        assert!(file_name(&engine).starts_with(&format!("automata_engine-{ENGINE_VERSION}-")));
        assert_eq!(bundle.dev_packages(), ["pytest"]);
        // Overrides win when they exist.
        let other = temp("bundle-wheels");
        let found = Bundle::find_in(Some(&dir), None, Some(other.clone()));
        assert_eq!(found.wheels, Some(other));
    }
}
