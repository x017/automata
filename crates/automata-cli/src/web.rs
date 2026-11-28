//! Web builds: a folder any static web host (or itch.io) can serve, with
//! the game running in the browser.
//!
//! The web template is the player site without a game: `index.html`, the
//! page's scripts (`player.js`, `audio-worklet.js`), Pyodide's runtime in
//! `pyodide/` and the engine as a Pyodide wheel. A build copies it to
//! `<output>/web/<name>/`, adds the game as `game.zip` (the staged game
//! folder, as `automata-cli stage --zip` writes it), writes `player.json`
//! naming the wheel and the game, and puts the game's name in the page
//! title. With `archive` on it also writes `<output>/web/<name>-<version>-web.zip`
//! with `index.html` at the top, which is what itch.io wants for an HTML
//! game.
//!
//! Browsers don't run such a page from a `file://` URL (scripts, the wheel
//! and the game are fetched), so it's played through a web server:
//! [`crate::serve`] (`automata-cli serve`), Oberon's **Play in Browser**, or
//! any static host.

use std::path::{Path, PathBuf};

use crate::{archive, BuildConfig, Exported, Message, Target};

/// The page a web template (and build) opens with.
pub const PAGE: &str = "index.html";

/// What the page reads first: the engine wheel and the game.
pub const CONFIG: &str = "player.json";

/// The game, zipped, beside the page.
pub const GAME_ZIP: &str = "game.zip";

/// Files every web template has besides the wheel.
const REQUIRED: [&str; 4] = [PAGE, "player.js", "audio-worklet.js", "pyodide/pyodide.mjs"];

/// The engine wheel in a web template (the one `.whl` at its top).
///
/// # Errors
///
/// When `dir` isn't a complete web template: a file is missing, or there's
/// no wheel or more than one.
pub fn template_wheel(dir: &Path) -> Result<String, String> {
    if let Some(missing) = REQUIRED.iter().find(|f| !dir.join(f).is_file()) {
        return Err(format!(
            "{} is not a web template (no {missing})",
            dir.display()
        ));
    }
    let mut wheels: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("whl"))
        })
        .collect();
    wheels.sort();
    match wheels.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!(
            "{} is not a web template (no engine wheel)",
            dir.display()
        )),
        _ => Err(format!(
            "{} has {} engine wheels; a web template has one",
            dir.display(),
            wheels.len()
        )),
    }
}

/// Whether `dir` holds a complete web template.
#[must_use]
pub fn is_template(dir: &Path) -> bool {
    template_wheel(dir).is_ok()
}

/// The build folder for `cfg`: `<output>/web/<name>`.
#[must_use]
pub fn build_folder(project: &Path, cfg: &BuildConfig) -> PathBuf {
    project
        .join(&cfg.output)
        .join(Target::Web.name())
        .join(cfg.file_name())
}

/// Exports `project` for the web from the template in `template` (the
/// project was validated by [`crate::export`]).
///
/// # Errors
///
/// When the template is incomplete or a file can't be written.
pub fn export(
    project: &Path,
    cfg: &BuildConfig,
    template: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<Exported, String> {
    let wheel = template_wheel(template).map_err(|e| {
        format!("{e} (Build Settings → Download templates, or automata-cli templates download --target web)")
    })?;
    let parent = project.join(&cfg.output).join(Target::Web.name());
    let folder = build_folder(project, cfg);
    if folder.exists() {
        std::fs::remove_dir_all(&folder)
            .map_err(|e| format!("could not clear {}: {e}", folder.display()))?;
    }
    crate::copy_tree(template, &folder, &|rel| {
        rel != CONFIG && rel != GAME_ZIP && !rel.ends_with(".installing")
    })?;

    // The game: staged next to the build, zipped into it.
    let staged = parent.join(".game");
    let n = crate::copy_game(project, cfg, &staged)?;
    let zipped = crate::zip_staged(&staged, &folder.join(GAME_ZIP));
    let _ = std::fs::remove_dir_all(&staged);
    zipped?;
    progress(Message::info(format!("Web: game ({n} files)")));

    let config = format!(
        "{{\"wheel\": {}, \"game\": {}, \"name\": {}}}\n",
        crate::json_string(&wheel),
        crate::json_string(GAME_ZIP),
        crate::json_string(cfg.name.trim()),
    );
    std::fs::write(folder.join(CONFIG), config).map_err(|e| e.to_string())?;
    let page = folder.join(PAGE);
    let html = std::fs::read_to_string(&page).map_err(|e| format!("{}: {e}", page.display()))?;
    std::fs::write(&page, with_title(&html, cfg.name.trim())).map_err(|e| e.to_string())?;

    let archive = if cfg.archive {
        let path = parent.join(format!(
            "{}-{}-{}.zip",
            cfg.file_name(),
            cfg.version.trim(),
            Target::Web.triple()
        ));
        // itch.io wants index.html at the top of the zip.
        let entries = archive::entries(&folder, "", &|_| false).map_err(|e| e.to_string())?;
        archive::write_zip(&path, &entries).map_err(|e| format!("{}: {e}", path.display()))?;
        progress(Message::info(format!(
            "Web: {} ({:.1} MB, ready for itch.io)",
            path.display(),
            file_size(&path) as f64 / 1e6
        )));
        Some(path)
    } else {
        None
    };
    progress(Message::info(format!(
        "Web: done → {} ({:.1} MB). Play it through a web server: automata-cli serve \"{}\"",
        folder.display(),
        folder_size(&folder) as f64 / 1e6,
        folder.display()
    )));
    Ok(Exported {
        target: Target::Web,
        executable: page,
        folder,
        archive,
        standalone: true,
    })
}

/// `html` with its `<title>` set to `name` (HTML-escaped).
fn with_title(html: &str, name: &str) -> String {
    let escaped = name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    match (html.find("<title>"), html.find("</title>")) {
        (Some(start), Some(end)) if start < end => format!(
            "{}<title>{escaped}</title>{}",
            &html[..start],
            &html[end + "</title>".len()..]
        ),
        _ => html.to_string(),
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

fn folder_size(dir: &Path) -> u64 {
    archive::entries(dir, "", &|_| false)
        .map_or(0, |list| list.iter().map(|e| file_size(&e.source)).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_replaced_and_escaped() {
        let html = "<head>\n  <title>AUTOMATA</title>\n</head>";
        assert_eq!(
            with_title(html, "Coins & <Runs>"),
            "<head>\n  <title>Coins &amp; &lt;Runs&gt;</title>\n</head>"
        );
        assert_eq!(with_title("<p>no title</p>", "X"), "<p>no title</p>");
    }
}
