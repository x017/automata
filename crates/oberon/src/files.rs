//! File operations behind the Files panel and the file browser: rename,
//! move, duplicate, move to the Trash, reveal in the system file manager,
//! plus the size and age labels the browser shows.
//!
//! Everything here works on plain paths and returns readable errors; the
//! editor logs them and refreshes its listings.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What "delete" does on this platform, for menus and the confirmation.
///
/// Linux and macOS move files to the desktop Trash (recoverable); other
/// systems delete them permanently.
#[must_use]
pub fn delete_label() -> &'static str {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        "Move to Trash"
    } else {
        "Delete Permanently"
    }
}

/// Whether deleting is recoverable (goes to the Trash).
#[must_use]
pub fn deletes_to_trash() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos"))
}

/// The user's home folder.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Expands a leading `~` to the home folder.
#[must_use]
pub fn expand_home(text: &str) -> PathBuf {
    if text == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(text)
}

/// Checks a new file or folder name typed by the user.
pub fn check_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name".into());
    }
    if name == "." || name == ".." {
        return Err("That name is reserved".into());
    }
    if name.contains(['/', '\\']) {
        return Err("Names can't contain / or \\".into());
    }
    if cfg!(windows) && name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        return Err("Names can't contain < > : \" | ? *".into());
    }
    Ok(())
}

/// Renames (or moves) `from` to `to`, refusing to overwrite.
pub fn rename(from: &Path, to: &Path) -> Result<(), String> {
    if from == to {
        return Ok(());
    }
    // A case-only rename on a case-insensitive disk sees `to` as existing.
    let case_only = from.parent() == to.parent()
        && from.file_name().map(|n| n.to_string_lossy().to_lowercase())
            == to.file_name().map(|n| n.to_string_lossy().to_lowercase());
    if to.exists() && !case_only {
        return Err(format!("{} already exists", display_name(to)));
    }
    if from.is_dir() && to.starts_with(from) {
        return Err("Can't move a folder into itself".into());
    }
    std::fs::rename(from, to).map_err(|e| format!("Rename failed: {e}"))
}

/// Moves `from` into the folder `dir`. Returns the new path.
pub fn move_into(from: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = from.file_name().ok_or("Nothing to move")?;
    let to = dir.join(name);
    if to == from {
        return Ok(to);
    }
    rename(from, &to)?;
    Ok(to)
}

/// `name copy.ext`, `name copy 2.ext`, … next to `path`, whichever is free.
#[must_use]
pub fn copy_name(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let (stem, ext) = split_name(path);
    for i in 1..1000 {
        let suffix = if i == 1 {
            " copy".to_string()
        } else {
            format!(" copy {i}")
        };
        let candidate = dir.join(format!("{stem}{suffix}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} copy{ext}"))
}

/// A file name split into stem and extension (with its dot). Folders and
/// dotfiles keep the whole name as the stem.
#[must_use]
pub fn split_name(path: &Path) -> (String, String) {
    let name = display_name(path);
    if path.is_dir() {
        return (name, String::new());
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name, String::new()),
    }
}

/// Copies a file or folder next to itself. Returns the copy's path.
pub fn duplicate(path: &Path) -> Result<PathBuf, String> {
    let to = copy_name(path);
    copy_recursive(path, &to).map_err(|e| format!("Duplicate failed: {e}"))?;
    Ok(to)
}

fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Deletes `path`: into the Trash where the desktop has one, else for good.
pub fn delete(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} no longer exists", display_name(path)));
    }
    if deletes_to_trash() {
        return trash(path);
    }
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|e| format!("Delete failed: {e}"))
}

#[cfg(target_os = "macos")]
fn trash(path: &Path) -> Result<(), String> {
    let dir = home_dir().ok_or("No home folder")?.join(".Trash");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Trash: {e}"))?;
    let dest = free_name(&dir, &display_name(path));
    std::fs::rename(path, &dest).map_err(|e| format!("Couldn't move to the Trash: {e}"))
}

/// The freedesktop.org Trash: the file goes to `Trash/files`, and a
/// `.trashinfo` next to it remembers where it came from, so the desktop's
/// file manager can restore it.
#[cfg(not(target_os = "macos"))]
fn trash(path: &Path) -> Result<(), String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home_dir().map(|h| h.join(".local/share")))
        .ok_or("No home folder for the Trash")?;
    trash_in(path, &data)
}

/// Moves `path` into the Trash under the data folder `data`.
#[cfg(not(target_os = "macos"))]
fn trash_in(path: &Path, data: &Path) -> Result<(), String> {
    let files = data.join("Trash/files");
    let info = data.join("Trash/info");
    std::fs::create_dir_all(&files).map_err(|e| format!("Trash: {e}"))?;
    std::fs::create_dir_all(&info).map_err(|e| format!("Trash: {e}"))?;
    let dest = free_name(&files, &display_name(path));
    let name = display_name(&dest);
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let info_path = info.join(format!("{name}.trashinfo"));
    let record = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&abs.to_string_lossy()),
        iso_datetime(SystemTime::now())
    );
    std::fs::write(&info_path, record).map_err(|e| format!("Trash: {e}"))?;
    if let Err(e) = std::fs::rename(path, &dest) {
        let _ = std::fs::remove_file(&info_path);
        return Err(format!(
            "Couldn't move {} to the Trash ({e}); delete it from your file manager",
            display_name(path)
        ));
    }
    Ok(())
}

/// `dir/name`, or `dir/name.2`, … whichever doesn't exist.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    (2..10_000)
        .map(|i| dir.join(format!("{name}.{i}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

#[cfg(not(target_os = "macos"))]
fn percent_encode(path: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Opens the system file manager showing `path` (its folder, with the file
/// selected where the platform supports it).
pub fn reveal(path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let mut c = Command::new("xdg-open");
        c.arg(dir);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the file manager: {e}"))
}

/// The last component of `path`, for messages.
#[must_use]
pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `512 B`, `2.1 KB`, `14 MB`, `1.2 GB`.
#[must_use]
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// How long ago `then` was: `just now`, `5 min ago`, `3 h ago`,
/// `yesterday`, `4 days ago`, or the date for anything older than a week.
#[must_use]
pub fn age_label(then: SystemTime, now: SystemTime) -> String {
    let Ok(age) = now.duration_since(then) else {
        return "just now".into();
    };
    let secs = age.as_secs();
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=172_799 => "yesterday".into(),
        172_800..=604_799 => format!("{} days ago", secs / 86_400),
        _ => date_label(then),
    }
}

/// `2026-11-18` (UTC).
#[must_use]
pub fn date_label(t: SystemTime) -> String {
    let (y, m, d, _) = civil(t);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(not(target_os = "macos"))]
fn iso_datetime(t: SystemTime) -> String {
    let (y, m, d, secs) = civil(t);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Year, month, day and seconds into the day (UTC) of `t` — Howard
/// Hinnant's days-to-civil algorithm.
fn civil(t: SystemTime) -> (i64, u32, u32, u64) {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, secs % 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon_files_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(2150), "2.1 KB");
        assert_eq!(human_size(14 * 1024 * 1024), "14 MB");
    }

    #[test]
    fn ages_read_naturally() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let ago = |s| age_label(now - Duration::from_secs(s), now);
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(300), "5 min ago");
        assert_eq!(ago(3 * 3600), "3 h ago");
        assert_eq!(ago(30 * 3600), "yesterday");
        assert_eq!(ago(4 * 86_400), "4 days ago");
        assert_eq!(date_label(SystemTime::UNIX_EPOCH), "1970-01-01");
        assert_eq!(
            date_label(SystemTime::UNIX_EPOCH + Duration::from_secs(1_763_424_000)),
            "2025-11-18"
        );
    }

    #[test]
    fn copies_get_free_names() {
        let dir = temp("copy");
        let file = dir.join("player.py");
        std::fs::write(&file, "x = 1\n").unwrap();
        let a = duplicate(&file).unwrap();
        assert_eq!(a, dir.join("player copy.py"));
        let b = duplicate(&file).unwrap();
        assert_eq!(b, dir.join("player copy 2.py"));
        assert_eq!(std::fs::read_to_string(b).unwrap(), "x = 1\n");
        let sub = dir.join("levels");
        std::fs::create_dir_all(sub.join("deep")).unwrap();
        std::fs::write(sub.join("deep/a.ron"), "()").unwrap();
        let copy = duplicate(&sub).unwrap();
        assert!(copy.join("deep/a.ron").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_refuses_to_overwrite_or_nest() {
        let dir = temp("rename");
        std::fs::write(dir.join("a.py"), "").unwrap();
        std::fs::write(dir.join("b.py"), "").unwrap();
        assert!(rename(&dir.join("a.py"), &dir.join("b.py")).is_err());
        rename(&dir.join("a.py"), &dir.join("c.py")).unwrap();
        assert!(dir.join("c.py").is_file());
        std::fs::create_dir_all(dir.join("f")).unwrap();
        assert!(move_into(&dir.join("f"), &dir.join("f")).is_err());
        let moved = move_into(&dir.join("c.py"), &dir.join("f")).unwrap();
        assert_eq!(moved, dir.join("f/c.py"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_are_checked() {
        assert!(check_name("enemy").is_ok());
        assert!(check_name("  ").is_err());
        assert!(check_name("a/b").is_err());
        assert!(check_name("..").is_err());
    }

    #[test]
    fn name_splits_keep_dotfiles_whole() {
        assert_eq!(
            split_name(Path::new("/nope/player.py")),
            ("player".into(), ".py".into())
        );
        assert_eq!(
            split_name(Path::new("/nope/.env")),
            (".env".into(), String::new())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trash_keeps_a_restore_record() {
        let dir = temp("trash");
        let data = dir.join("data");
        let file = dir.join("old level.ron");
        std::fs::write(&file, "()").unwrap();
        trash_in(&file, &data).unwrap();
        assert!(!file.exists());
        assert!(data.join("Trash/files/old level.ron").is_file());
        let info =
            std::fs::read_to_string(data.join("Trash/info/old level.ron.trashinfo")).unwrap();
        assert!(info.contains("old%20level.ron"), "{info}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
