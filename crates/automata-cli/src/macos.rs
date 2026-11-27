//! macOS export: one `.app` bundle for Apple Silicon and Intel Macs.
//!
//! ```text
//! Coin Run.app/Contents/
//!   Info.plist               name, bundle ID, version, icon
//!   PkgInfo
//!   MacOS/Coin Run           the launcher (universal: arm64 + x86_64)
//!   MacOS/python3-arm64      the interpreter per processor; inside the
//!   MacOS/python3-x86_64     bundle so the game is the app to macOS
//!   Resources/game/          the project
//!   Resources/runtime-arm64/ the Python standard library, the engine
//!   Resources/runtime-x86_64/
//!   Resources/AppIcon.icns   when `macos.icon` is set
//! ```
//!
//! The template (`automata-template-macos-universal.tar.gz`) holds the
//! launcher, `python3-<arch>` and `runtime-<arch>/` for each processor. CI
//! builds one per processor and merges them; a template with one processor
//! makes an app for that processor only, and a development template (just
//! the launcher) one that uses the Python on `PATH`.

use std::fmt::Write as _;
use std::path::Path;

use crate::{archive, BuildConfig, Exported, Message, Target};

/// The processors a macOS template can hold (folder suffixes).
pub const ARCHES: [&str; 2] = ["arm64", "x86_64"];

/// The runtime folders in a template: `runtime-<arch>/` for each processor
/// it holds, or a single `runtime/`.
#[must_use]
pub fn runtimes(template: &Path) -> Vec<String> {
    ARCHES
        .iter()
        .map(|a| format!("runtime-{a}"))
        .chain(std::iter::once("runtime".to_string()))
        .filter(|d| template.join(d).is_dir())
        .collect()
}

/// The archive suffix for the processors a template holds
/// (`macos-universal`, `macos-arm64`; `macos` without a runtime).
fn platform_suffix(template: &Path) -> String {
    let arches: Vec<&str> = ARCHES
        .iter()
        .copied()
        .filter(|a| template.join(format!("runtime-{a}")).is_dir())
        .collect();
    match arches.as_slice() {
        [_, _] => Target::MacOS.triple().to_string(),
        [one] => format!("macos-{one}"),
        _ => "macos".to_string(),
    }
}

/// Problems with the macOS settings (bundle ID, icon).
#[must_use]
pub fn problems(project: &Path, cfg: &BuildConfig) -> Vec<String> {
    let mut out = Vec::new();
    if let Err(e) = crate::check_bundle_id(&cfg.macos_bundle_id()) {
        out.push(e);
    }
    if let Err(e) = icon(project, cfg) {
        out.push(e);
    }
    out
}

/// The `.icns` for `macos.icon`, if one is set.
fn icon(project: &Path, cfg: &BuildConfig) -> Result<Option<Vec<u8>>, String> {
    let rel = cfg.macos.icon.trim();
    if rel.is_empty() {
        return Ok(None);
    }
    let path = project.join(rel);
    let bytes = std::fs::read(&path).map_err(|e| format!("macOS icon {rel}: {e}"))?;
    if bytes.starts_with(b"icns") {
        return Ok(Some(bytes));
    }
    icns_from_png(&bytes)
        .map(Some)
        .map_err(|e| format!("macOS icon {rel}: {e}"))
}

/// Wraps a square PNG in an `.icns` file (which holds PNGs as they are).
///
/// # Errors
///
/// When the data isn't a PNG, or isn't square with a side of 16 to 1024
/// pixels and a power of two.
pub fn icns_from_png(png: &[u8]) -> Result<Vec<u8>, String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !png.starts_with(SIGNATURE) || png.len() < 24 || &png[12..16] != b"IHDR" {
        return Err("not a PNG or .icns file".into());
    }
    let side = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    let (w, h) = (side(16), side(20));
    let kind: &[u8; 4] = match (w == h).then_some(w) {
        Some(16) => b"icp4",
        Some(32) => b"icp5",
        Some(64) => b"icp6",
        Some(128) => b"ic07",
        Some(256) => b"ic08",
        Some(512) => b"ic09",
        Some(1024) => b"ic10",
        _ => {
            return Err(format!(
                "the PNG is {w}×{h}; use a square one of 16, 32, 64, 128, 256, 512 or 1024 pixels (1024 looks sharpest)"
            ))
        }
    };
    let entry = u32::try_from(png.len() + 8).map_err(|_| "the PNG is too large".to_string())?;
    let mut out = Vec::with_capacity(png.len() + 16);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(entry + 8).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(&entry.to_be_bytes());
    out.extend_from_slice(png);
    Ok(out)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The version as macOS wants it: up to three numbers joined by dots
/// (`1.2.0-beta` → `1.2.0`; no number → `1`).
fn bundle_version(version: &str) -> String {
    let numbers: Vec<String> = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map_while(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            (!digits.is_empty()).then_some(digits)
        })
        .take(3)
        .collect();
    if numbers.is_empty() {
        "1".into()
    } else {
        numbers.join(".")
    }
}

/// The bundle's `Info.plist`.
#[must_use]
pub fn info_plist(cfg: &BuildConfig, executable: &str, has_icon: bool) -> String {
    let version = bundle_version(&cfg.version);
    let mut keys: Vec<(&str, String)> = vec![
        ("CFBundleDevelopmentRegion", "en".into()),
        ("CFBundleDisplayName", cfg.name.trim().to_string()),
        ("CFBundleExecutable", executable.to_string()),
        ("CFBundleIdentifier", cfg.macos_bundle_id()),
        ("CFBundleInfoDictionaryVersion", "6.0".into()),
        ("CFBundleName", cfg.name.trim().to_string()),
        ("CFBundlePackageType", "APPL".into()),
        ("CFBundleShortVersionString", version.clone()),
        ("CFBundleVersion", version),
        (
            "LSApplicationCategoryType",
            "public.app-category.games".into(),
        ),
        ("LSMinimumSystemVersion", "11.0".into()),
    ];
    if has_icon {
        keys.push(("CFBundleIconFile", "AppIcon".into()));
        keys.sort_by_key(|(k, _)| *k);
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in keys {
        let _ = writeln!(
            out,
            "\t<key>{key}</key>\n\t<string>{}</string>",
            xml_escape(&value)
        );
    }
    out.push_str(
        "\t<key>NSHighResolutionCapable</key>\n\t<true/>\n\
         \t<key>NSSupportsAutomaticGraphicsSwitching</key>\n\t<true/>\n\
         </dict>\n</plist>\n",
    );
    out
}

/// Whether a file in an exported `.app` (or a macOS template) must be
/// executable: the launcher and interpreters, the runtime's programs, and
/// native libraries.
pub(crate) fn executable(rel: &str) -> bool {
    let in_runtime = rel.split('/').any(|p| p.starts_with("runtime"));
    let library = Path::new(rel)
        .extension()
        .is_some_and(|e| e == "so" || e == "dylib");
    rel.starts_with("Contents/MacOS/")
        || rel == Target::MacOS.player_file()
        || rel.starts_with("python3-")
        || (in_runtime && (rel.contains("/bin/") || library))
}

/// Exports `project` as `<output>/macos/<name>.app` from `template`.
pub(crate) fn export(
    project: &Path,
    cfg: &BuildConfig,
    template: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<Exported, String> {
    let target = Target::MacOS;
    let player = template.join(target.player_file());
    if !player.is_file() {
        return Err(format!(
            "no macOS template at {} (Build Settings → Download templates)",
            template.display()
        ));
    }
    if let Some(problem) = problems(project, cfg).into_iter().next() {
        return Err(problem);
    }
    let icon = icon(project, cfg)?;
    let runtimes = runtimes(template);
    let standalone = !runtimes.is_empty();
    if !standalone {
        progress(Message::warn(
            "macOS template has no bundled Python: the build runs with the Python on PATH (development template).",
        ));
    }

    let name = cfg.file_name();
    let out_root = project.join(&cfg.output);
    let app = out_root.join("macos").join(format!("{name}.app"));
    progress(Message::info(format!(
        "macOS: exporting to {}",
        app.display()
    )));
    if app.exists() {
        std::fs::remove_dir_all(&app)
            .map_err(|e| format!("could not clear {}: {e}", app.display()))?;
    }
    let contents = app.join("Contents");
    let bin = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&bin, &resources] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    // The launcher, renamed after the game, and the interpreters.
    let exe = bin.join(&name);
    std::fs::copy(&player, &exe).map_err(|e| format!("{}: {e}", player.display()))?;
    crate::make_executable(&exe);
    for arch in ARCHES {
        let file = format!("python3-{arch}");
        let from = template.join(&file);
        if from.is_file() {
            let to = bin.join(&file);
            std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
            crate::make_executable(&to);
        }
    }

    // The runtimes.
    for rt in &runtimes {
        let n = crate::copy_tree(&template.join(rt), &resources.join(rt), &|_| true)?;
        progress(Message::info(format!("macOS: {rt} ({n} files)")));
    }

    // The game files.
    let n = crate::copy_game(project, cfg, &resources.join("game"))?;
    progress(Message::info(format!("macOS: game ({n} files)")));

    if let Some(icns) = &icon {
        std::fs::write(resources.join("AppIcon.icns"), icns).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        contents.join("Info.plist"),
        info_plist(cfg, &name, icon.is_some()),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(contents.join("PkgInfo"), "APPL????").map_err(|e| e.to_string())?;

    let archive_path = if cfg.archive {
        let file = format!(
            "{name}-{}-{}.zip",
            cfg.version.trim(),
            platform_suffix(template)
        );
        let path = out_root.join(file);
        let entries = archive::entries(&app, &format!("{name}.app"), &executable)
            .map_err(|e| e.to_string())?;
        archive::write_zip(&path, &entries).map_err(|e| format!("{}: {e}", path.display()))?;
        progress(Message::info(format!("macOS: archive {}", path.display())));
        Some(path)
    } else {
        None
    };
    progress(Message::info(format!("macOS: done → {}", app.display())));
    Ok(Exported {
        target,
        folder: app,
        executable: exe,
        archive: archive_path,
        standalone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_become_bundle_versions() {
        assert_eq!(bundle_version("1.2.3"), "1.2.3");
        assert_eq!(bundle_version("v0.4"), "0.4");
        assert_eq!(bundle_version("1.2.0-beta.1"), "1.2.0");
        assert_eq!(bundle_version("2.0.1.7"), "2.0.1");
        assert_eq!(bundle_version("alpha"), "1");
    }

    #[test]
    fn png_icons_are_wrapped_in_icns() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&512u32.to_be_bytes());
        png.extend_from_slice(&512u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let icns = icns_from_png(&png).unwrap();
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize,
            icns.len()
        );
        assert_eq!(&icns[8..12], b"ic09");
        assert_eq!(&icns[16..], &png[..]);

        png[19] = 200; // 512 × 712
        assert!(icns_from_png(&png).unwrap_err().contains("square"));
        assert!(icns_from_png(b"GIF89a").is_err());
    }

    #[test]
    fn executables_in_a_bundle() {
        assert!(executable("Contents/MacOS/Coin Run"));
        assert!(executable("Contents/MacOS/python3-arm64"));
        assert!(executable(
            "Contents/Resources/runtime-arm64/lib/libpython3.12.dylib"
        ));
        assert!(executable("Contents/Resources/runtime-x86_64/bin/python3"));
        assert!(executable("runtime-arm64/bin/python3.12"));
        assert!(executable("python3-x86_64"));
        assert!(executable("automata-player"));
        assert!(!executable("Contents/Info.plist"));
        assert!(!executable("Contents/Resources/game/scripts/player.py"));
        assert!(!executable(
            "Contents/Resources/runtime-arm64/lib/python3.12/os.py"
        ));
    }
}
