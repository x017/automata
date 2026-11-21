//! Minimal `.tar.gz` and `.zip` writers and a `.tar.gz` reader.
//!
//! Exported games ship as `.tar.gz` (Linux keeps the executable bit) or
//! `.zip` (what Windows users expect). Templates are `.tar.gz` on every
//! platform. Only what the exporter needs is implemented: regular files
//! and directories, ustar with GNU long names, and deflated zip entries.
//! Symlinks to files are stored as the file they point at (portable
//! Pythons link `bin/python3` → `python3.12`); symlinked directories are
//! skipped.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::{DeflateEncoder, GzEncoder};
use flate2::Compression;

/// One file to archive: where it is on disk and its path inside the archive.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The file on disk.
    pub source: PathBuf,
    /// Its path inside the archive (forward slashes).
    pub name: String,
    /// Whether it should be executable (Unix mode 0755).
    pub executable: bool,
}

/// Lists every file under `root` as archive entries prefixed with `prefix/`.
///
/// `executable(rel)` decides the executable bit by relative path; on Unix
/// the file's own mode also counts.
///
/// # Errors
///
/// When the tree can't be read.
pub fn entries(
    root: &Path,
    prefix: &str,
    executable: &dyn Fn(&str) -> bool,
) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    walk(root, root, prefix, executable, &mut out)?;
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    prefix: &str,
    executable: &dyn Fn(&str) -> bool,
    out: &mut Vec<Entry>,
) -> io::Result<()> {
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let kind = item.file_type()?;
        if kind.is_dir() {
            walk(root, &path, prefix, executable, out)?;
        } else if kind.is_file() || is_file_link(&path) {
            let rel = slash_path(path.strip_prefix(root).unwrap_or(&path));
            let exec = executable(&rel) || unix_executable(&path);
            let name = if prefix.is_empty() {
                rel
            } else {
                format!("{prefix}/{rel}")
            };
            out.push(Entry {
                source: path,
                name,
                executable: exec,
            });
        }
    }
    Ok(())
}

/// Whether `path` is a symlink that resolves to a regular file (a dangling
/// link or a link to a directory is not).
#[must_use]
pub fn is_file_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// `a/b/c` from a relative path, whatever the host separator.
#[must_use]
pub fn slash_path(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(unix)]
fn unix_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn unix_executable(_path: &Path) -> bool {
    false
}

// ----- tar ------------------------------------------------------------------

fn octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    let text = format!("{value:0digits$o}");
    let bytes = text.as_bytes();
    let start = bytes.len().saturating_sub(digits);
    field[..digits].copy_from_slice(&bytes[start..]);
    field[digits] = 0;
}

/// Seconds since the Unix epoch, for archive timestamps.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// MS-DOS `(time, date)` for a Unix timestamp (UTC), as zip stores it.
fn dos_datetime(secs: u64) -> (u16, u16) {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let year = year.clamp(1980, 2107);
    let pack = |v: i64| u16::try_from(v).unwrap_or(0);
    let time = pack(i64::try_from(rem / 3600).unwrap_or(0)) << 11
        | pack(i64::try_from(rem % 3600 / 60).unwrap_or(0)) << 5
        | pack(i64::try_from(rem % 60 / 2).unwrap_or(0));
    let date = pack(year - 1980) << 9 | pack(month) << 5 | pack(day);
    (time, date)
}

fn tar_header(name: &[u8], size: u64, mode: u32, kind: u8) -> [u8; 512] {
    let mut h = [0u8; 512];
    let n = name.len().min(100);
    h[..n].copy_from_slice(&name[..n]);
    octal(&mut h[100..108], u64::from(mode));
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], now_secs());
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    octal(&mut h[148..155], u64::from(sum));
    h[155] = b' ';
    h
}

fn write_padded(out: &mut impl Write, data: &[u8]) -> io::Result<()> {
    out.write_all(data)?;
    let pad = (512 - data.len() % 512) % 512;
    out.write_all(&vec![0u8; pad])
}

/// Writes `entries` to a gzip-compressed tar at `dest`.
///
/// # Errors
///
/// When a file can't be read or the archive can't be written.
pub fn write_tar_gz(dest: &Path, entries: &[Entry]) -> io::Result<()> {
    let file = BufWriter::new(File::create(dest)?);
    let mut gz = GzEncoder::new(file, Compression::default());
    for e in entries {
        let data = std::fs::read(&e.source)?;
        let name = e.name.as_bytes();
        if name.len() > 100 {
            // GNU long name: a pseudo-entry holding the full path.
            let mut long = name.to_vec();
            long.push(0);
            gz.write_all(&tar_header(
                b"././@LongLink",
                long.len() as u64,
                0o644,
                b'L',
            ))?;
            write_padded(&mut gz, &long)?;
        }
        let mode = if e.executable { 0o755 } else { 0o644 };
        gz.write_all(&tar_header(name, data.len() as u64, mode, b'0'))?;
        write_padded(&mut gz, &data)?;
    }
    gz.write_all(&[0u8; 1024])?;
    gz.finish()?.flush()
}

/// Extracts a `.tar.gz` into `dest`. Returns the number of files written.
///
/// Paths that would escape `dest` are rejected.
///
/// # Errors
///
/// When the archive is malformed or a file can't be written.
pub fn extract_tar_gz(archive: &Path, dest: &Path) -> io::Result<usize> {
    let mut gz = GzDecoder::new(File::open(archive)?);
    let mut long_name: Option<String> = None;
    let mut count = 0;
    loop {
        let mut h = [0u8; 512];
        if read_full(&mut gz, &mut h)? == 0 || h.iter().all(|&b| b == 0) {
            break;
        }
        let size = parse_octal(&h[124..136]);
        let kind = h[156];
        let mut data = vec![0u8; usize::try_from(size).map_err(|_| bad("entry too large"))?];
        read_full(&mut gz, &mut data)?;
        let pad = (512 - data.len() % 512) % 512;
        let mut skip = vec![0u8; pad];
        read_full(&mut gz, &mut skip)?;

        if kind == b'L' {
            long_name = Some(cstr(&data));
            continue;
        }
        let name = long_name.take().unwrap_or_else(|| {
            let prefix = cstr(&h[345..500]);
            let base = cstr(&h[..100]);
            if prefix.is_empty() {
                base
            } else {
                format!("{prefix}/{base}")
            }
        });
        let rel =
            safe_relative(&name).ok_or_else(|| bad(&format!("unsafe path in archive: {name}")))?;
        let target = dest.join(rel);
        match kind {
            b'5' => std::fs::create_dir_all(&target)?,
            b'0' | 0 => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&target, &data)?;
                set_mode(
                    &target,
                    u32::try_from(parse_octal(&h[100..108])).unwrap_or(0o644),
                );
                count += 1;
            }
            _ => {} // links and pax headers are not used by templates
        }
    }
    Ok(count)
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    if filled != 0 && filled != buf.len() {
        return Err(bad("truncated archive"));
    }
    Ok(filled)
}

fn parse_octal(field: &[u8]) -> u64 {
    field
        .iter()
        .take_while(|&&b| b != 0 && b != b' ')
        .filter(|b| b.is_ascii_digit())
        .fold(0, |acc, &b| acc * 8 + u64::from(b - b'0'))
}

fn cstr(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn safe_relative(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.contains(':') || p.contains('\\') => return None,
            p => out.push(p),
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

// ----- zip ------------------------------------------------------------------

/// Writes `entries` to a zip archive at `dest` (deflated).
///
/// # Errors
///
/// When a file can't be read or the archive can't be written, or the
/// archive would need zip64 (over 4 GiB or 65 535 files).
pub fn write_zip(dest: &Path, entries: &[Entry]) -> io::Result<()> {
    if entries.len() > usize::from(u16::MAX) {
        return Err(bad("too many files for a zip archive"));
    }
    let mut out = BufWriter::new(File::create(dest)?);
    let mut central = Vec::new();
    let mut offset: u64 = 0;
    let (dos_time, dos_date) = dos_datetime(now_secs());
    for e in entries {
        let data = std::fs::read(&e.source)?;
        let crc = crc32fast::hash(&data);
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&data)?;
        let packed = enc.finish()?;
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() {
            (8, &packed)
        } else {
            (0, &data)
        };
        let name = e.name.as_bytes();
        let size32 = |n: usize| u32::try_from(n).map_err(|_| bad("file too large for zip"));
        let name_len = u16::try_from(name.len()).map_err(|_| bad("path too long"))?;
        let local_offset = u32::try_from(offset).map_err(|_| bad("archive too large for zip"))?;

        let mut local = Vec::with_capacity(30 + name.len());
        local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes()); // version needed
        local.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        local.extend_from_slice(&method.to_le_bytes());
        local.extend_from_slice(&dos_time.to_le_bytes());
        local.extend_from_slice(&dos_date.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&size32(body.len())?.to_le_bytes());
        local.extend_from_slice(&size32(data.len())?.to_le_bytes());
        local.extend_from_slice(&name_len.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name);
        out.write_all(&local)?;
        out.write_all(body)?;

        let mode: u32 = if e.executable { 0o100_755 } else { 0o100_644 };
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&0x031Eu16.to_le_bytes()); // made by: Unix, 3.0
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&dos_time.to_le_bytes());
        central.extend_from_slice(&dos_date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size32(body.len())?.to_le_bytes());
        central.extend_from_slice(&size32(data.len())?.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, comment, disk, internal attrs
        central.extend_from_slice(&(mode << 16).to_le_bytes());
        central.extend_from_slice(&local_offset.to_le_bytes());
        central.extend_from_slice(name);

        offset += (local.len() + body.len()) as u64;
    }
    let count = u16::try_from(entries.len()).unwrap_or(u16::MAX);
    let central_offset = u32::try_from(offset).map_err(|_| bad("archive too large for zip"))?;
    let central_len = u32::try_from(central.len()).map_err(|_| bad("archive too large for zip"))?;
    out.write_all(&central)?;
    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    end.extend_from_slice(&[0u8; 4]);
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&central_len.to_le_bytes());
    end.extend_from_slice(&central_offset.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    out.write_all(&end)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("automata-archive-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tar_gz_round_trips_long_names_and_modes() {
        let dir = scratch("tar");
        let src = dir.join("src");
        let deep = "a/".repeat(60) + "file.txt";
        std::fs::create_dir_all(src.join(Path::new(&deep).parent().unwrap())).unwrap();
        std::fs::write(src.join(&deep), b"deep").unwrap();
        std::fs::write(src.join("run"), b"#!/bin/sh\n").unwrap();
        let list = entries(&src, "pkg", &|rel| rel == "run").unwrap();
        let archive = dir.join("t.tar.gz");
        write_tar_gz(&archive, &list).unwrap();
        let out = dir.join("out");
        assert_eq!(extract_tar_gz(&archive, &out).unwrap(), 2);
        assert_eq!(std::fs::read(out.join("pkg").join(&deep)).unwrap(), b"deep");
        assert!(unix_executable(&out.join("pkg/run")) || cfg!(not(unix)));
    }

    #[cfg(unix)]
    #[test]
    fn file_symlinks_are_archived_as_their_target() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("links");
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("bin")).unwrap();
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::write(src.join("bin/python3.12"), b"ELF").unwrap();
        std::fs::set_permissions(
            src.join("bin/python3.12"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::os::unix::fs::symlink("python3.12", src.join("bin/python3")).unwrap();
        // A directory link is skipped (no loops), a dangling one ignored.
        std::os::unix::fs::symlink("../lib", src.join("bin/loop")).unwrap();
        std::os::unix::fs::symlink("missing", src.join("bin/dangling")).unwrap();
        let list = entries(&src, "", &|_| false).unwrap();
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["bin/python3", "bin/python3.12"]);
        let archive = dir.join("t.tar.gz");
        write_tar_gz(&archive, &list).unwrap();
        let out = dir.join("out");
        assert_eq!(extract_tar_gz(&archive, &out).unwrap(), 2);
        assert_eq!(std::fs::read(out.join("bin/python3")).unwrap(), b"ELF");
        assert!(unix_executable(&out.join("bin/python3")));
    }

    #[test]
    fn zip_has_a_valid_directory() {
        let dir = scratch("zip");
        std::fs::write(dir.join("a.txt"), "hello ".repeat(100)).unwrap();
        let list = vec![Entry {
            source: dir.join("a.txt"),
            name: "Game/a.txt".into(),
            executable: false,
        }];
        let archive = dir.join("g.zip");
        write_zip(&archive, &list).unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        assert_eq!(&bytes[..4], &[0x50, 0x4b, 0x03, 0x04]);
        let end = &bytes[bytes.len() - 22..];
        assert_eq!(&end[..4], &[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(u16::from_le_bytes([end[10], end[11]]), 1);
        // Deflated: smaller than the 600-byte file.
        assert!(bytes.len() < 400);
    }

    #[test]
    fn dos_dates_are_packed() {
        // 2026-11-18 13:45:30 UTC.
        let (time, date) = dos_datetime(1_795_009_530);
        assert_eq!(date >> 9, 46);
        assert_eq!((date >> 5) & 0xF, 11);
        assert_eq!(date & 0x1F, 18);
        assert_eq!(time >> 11, 13);
        assert_eq!((time >> 5) & 0x3F, 45);
        assert_eq!((time & 0x1F) * 2, 30);
    }

    #[test]
    fn unsafe_archive_paths_are_rejected() {
        assert!(safe_relative("../etc/passwd").is_none());
        assert!(safe_relative("C:/x").is_none());
        assert_eq!(
            safe_relative("./a/b").unwrap(),
            PathBuf::from("a").join("b")
        );
    }
}
