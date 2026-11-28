//! Steam in desktop builds (`build.ron`'s `steam`): the game's app id
//! goes into `game.json`, and Valve's `steam_api` library from the
//! Steamworks SDK goes next to the game, where `automata.steam` loads it
//! when the game starts.
//!
//! The SDK isn't part of AUTOMATA (Valve's license covers shipping the
//! library with a game, not with an engine): download it from the
//! Steamworks partner site and point `steam.sdk` (or
//! `$AUTOMATA_STEAMWORKS_SDK`) at its folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Message, Target};

/// The environment variable that names the Steamworks SDK folder, over
/// `build.ron`'s `steam.sdk`.
pub const SDK_ENV: &str = "AUTOMATA_STEAMWORKS_SDK";

/// Steam settings (`build.ron`'s `steam`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SteamConfig {
    /// The game's Steam app id. 0: no Steam.
    pub app_id: u32,
    /// The Steamworks SDK folder (the one holding `redistributable_bin`,
    /// or that folder itself): absolute, under `~/`, or relative to the
    /// project. Keep it outside the project. `$AUTOMATA_STEAMWORKS_SDK`
    /// takes precedence.
    pub sdk: String,
    /// Whether a copy started outside Steam asks Steam to start it (and
    /// quits). On for release builds; off while testing, so the build
    /// runs on its own (Steam must still be running).
    pub restart: bool,
}

impl SteamConfig {
    /// Whether the game uses Steam.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.app_id != 0
    }

    /// The settings as `game.json` holds them, or `None` without Steam.
    #[must_use]
    pub fn to_json(&self) -> Option<String> {
        self.enabled().then(|| {
            format!(
                "{{\"app_id\": {}, \"restart\": {}}}",
                self.app_id, self.restart
            )
        })
    }

    /// The SDK's `redistributable_bin` folder, when one is set and found.
    ///
    /// # Errors
    ///
    /// When none is set, or the folder has no `redistributable_bin`.
    pub fn redistributables(&self, project: &Path) -> Result<PathBuf, String> {
        let env = std::env::var(SDK_ENV).ok();
        let set = env
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| Some(self.sdk.trim()).filter(|s| !s.is_empty()))
            .ok_or_else(|| {
                format!(
                    "steam: no Steamworks SDK. Download it from partner.steamgames.com and set steam.sdk (build.ron, or Build Settings → Steamworks SDK) or ${SDK_ENV} to its folder."
                )
            })?;
        let root = expand(project, set);
        for dir in [root.join("redistributable_bin"), root.clone()] {
            if dir.join("linux64").is_dir()
                || dir.join("win64").is_dir()
                || dir.join("osx").is_dir()
            {
                return Ok(dir);
            }
        }
        Err(format!(
            "steam: {} isn't the Steamworks SDK (it has no redistributable_bin with linux64, win64 or osx in it)",
            root.display()
        ))
    }

    /// The `steam_api` library a `target` build ships, from the SDK, or
    /// `None` for builds without Steam (Android, the web) or a game
    /// without it.
    ///
    /// # Errors
    ///
    /// When the game uses Steam and the SDK or its library for `target`
    /// is missing.
    pub fn library(&self, project: &Path, target: Target) -> Result<Option<PathBuf>, String> {
        let Some(rel) = library_in_sdk(target) else {
            return Ok(None);
        };
        if !self.enabled() {
            return Ok(None);
        }
        let path = self.redistributables(project)?.join(rel);
        if path.is_file() {
            Ok(Some(path))
        } else {
            Err(format!(
                "steam: the Steamworks SDK has no {} for {} builds ({})",
                file_name(rel),
                target.label(),
                path.display()
            ))
        }
    }

    /// What `validate` says about these settings.
    pub(crate) fn check(&self, project: &Path, targets: &[Target], out: &mut Vec<Message>) {
        if !self.enabled() {
            return;
        }
        if targets.iter().any(|t| library_in_sdk(*t).is_some()) {
            if let Err(e) = self.redistributables(project) {
                out.push(Message::error(e));
            }
        }
        let without: Vec<&str> = targets
            .iter()
            .filter(|t| library_in_sdk(**t).is_none())
            .map(|t| t.label())
            .collect();
        if !without.is_empty() {
            out.push(Message::info(format!(
                "steam: {} builds run without Steam (ctx.steam works offline there).",
                without.join(" and ")
            )));
        }
    }
}

/// Where a target's `steam_api` library sits in `redistributable_bin`.
#[must_use]
pub fn library_in_sdk(target: Target) -> Option<&'static str> {
    match target {
        Target::Linux => Some("linux64/libsteam_api.so"),
        Target::Windows => Some("win64/steam_api64.dll"),
        Target::MacOS => Some("osx/libsteam_api.dylib"),
        Target::Android | Target::Web => None,
    }
}

/// Copies the `steam_api` library for `target` into a build's game folder
/// (when the game uses Steam). Returns what was copied.
///
/// # Errors
///
/// When the library is missing or can't be copied.
pub fn copy_library(
    cfg: &SteamConfig,
    project: &Path,
    target: Target,
    game: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(lib) = cfg.library(project, target)? else {
        return Ok(None);
    };
    let dest = game.join(lib.file_name().unwrap_or_default());
    std::fs::copy(&lib, &dest).map_err(|e| format!("{}: {e}", lib.display()))?;
    Ok(Some(dest))
}

/// `steam.sdk` as a slash path under the project, when it's a relative
/// path there (an export leaves it out).
#[must_use]
pub fn sdk_in_project(cfg: &SteamConfig) -> Option<String> {
    let set = cfg.sdk.trim().replace('\\', "/");
    let set = set.trim_start_matches("./").trim_end_matches('/');
    let relative = !set.is_empty()
        && !set.starts_with('~')
        && !Path::new(set).is_absolute()
        && !set.split('/').any(|p| p == "..");
    relative.then(|| set.to_string())
}

/// [`copy_library`], saying what was copied through `progress`.
///
/// # Errors
///
/// As [`copy_library`].
pub fn ship(
    cfg: &SteamConfig,
    project: &Path,
    target: Target,
    game: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<(), String> {
    if let Some(lib) = copy_library(cfg, project, target, game)? {
        progress(Message::info(format!(
            "{}: Steam (app {}, {})",
            target.label(),
            cfg.app_id,
            lib.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    Ok(())
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn expand(project: &Path, set: &str) -> PathBuf {
    if let Some(rest) = set.strip_prefix("~/").or_else(|| set.strip_prefix("~\\")) {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(set);
    if path.is_absolute() {
        path
    } else {
        project.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("automata-steam-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sdk(dir: &Path) -> PathBuf {
        let bin = dir.join("sdk/redistributable_bin");
        for rel in [
            "linux64/libsteam_api.so",
            "win64/steam_api64.dll",
            "osx/libsteam_api.dylib",
        ] {
            let p = bin.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, rel).unwrap();
        }
        dir.join("sdk")
    }

    #[test]
    fn off_by_default_and_in_game_json_when_on() {
        let cfg = SteamConfig::default();
        assert!(!cfg.enabled());
        assert_eq!(cfg.to_json(), None);
        let on = SteamConfig {
            app_id: 480,
            restart: true,
            ..SteamConfig::default()
        };
        assert_eq!(on.to_json().unwrap(), r#"{"app_id": 480, "restart": true}"#);
    }

    #[test]
    fn finds_each_platforms_library() {
        let dir = scratch("find");
        let root = sdk(&dir);
        let mut cfg = SteamConfig {
            app_id: 480,
            sdk: "sdk".into(),
            restart: false,
        };
        let lib = |t| cfg.library(&dir, t).unwrap().unwrap();
        assert!(lib(Target::Linux).ends_with("linux64/libsteam_api.so"));
        assert!(lib(Target::Windows).ends_with("win64/steam_api64.dll"));
        assert!(lib(Target::MacOS).ends_with("osx/libsteam_api.dylib"));
        assert_eq!(cfg.library(&dir, Target::Web).unwrap(), None);
        assert_eq!(cfg.library(&dir, Target::Android).unwrap(), None);
        // The redistributable_bin folder itself works too, and absolute paths.
        cfg.sdk = root.join("redistributable_bin").display().to_string();
        assert!(cfg.library(&dir, Target::Linux).unwrap().is_some());
        let game = dir.join("game");
        std::fs::create_dir_all(&game).unwrap();
        let copied = copy_library(&cfg, &dir, Target::Windows, &game).unwrap();
        assert_eq!(copied.unwrap(), game.join("steam_api64.dll"));
        // Without Steam nothing is needed.
        cfg.app_id = 0;
        cfg.sdk.clear();
        assert_eq!(cfg.library(&dir, Target::Linux).unwrap(), None);
    }

    #[test]
    fn says_whats_missing() {
        let dir = scratch("missing");
        let mut cfg = SteamConfig {
            app_id: 480,
            ..SteamConfig::default()
        };
        if std::env::var_os(SDK_ENV).is_none() {
            let e = cfg.library(&dir, Target::Linux).unwrap_err();
            assert!(e.contains("no Steamworks SDK"), "{e}");
            let mut out = Vec::new();
            cfg.check(&dir, &[Target::Linux, Target::Web], &mut out);
            assert!(out[0].text.contains("no Steamworks SDK"), "{out:?}");
            assert!(
                out[1].text.contains("Web builds run without Steam"),
                "{out:?}"
            );
        }
        cfg.sdk = "nope".into();
        let e = cfg.library(&dir, Target::Linux).unwrap_err();
        assert!(e.contains("isn't the Steamworks SDK"), "{e}");
        sdk(&dir);
        std::fs::remove_file(dir.join("sdk/redistributable_bin/win64/steam_api64.dll")).unwrap();
        cfg.sdk = "sdk".into();
        let e = cfg.library(&dir, Target::Windows).unwrap_err();
        assert!(e.contains("no steam_api64.dll for Windows builds"), "{e}");
    }
}
