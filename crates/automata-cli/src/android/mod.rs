//! Android builds: the template APK with the game inside, signed.
//!
//! The template (`automata-template.apk`, built by CI with
//! python-for-android) holds Python, the engine and the player, but no
//! game. A build copies it and changes what makes it this game's app:
//!
//! - the game files go in as `assets/automata/game/…`, with a `stamp` (a
//!   hash of them) that tells the player when to unpack them again;
//! - the manifest gets the game's app ID (`android.package` in
//!   `build.ron`), name and version, and `resources.arsc` the app ID;
//! - the APK is aligned and signed (v2) with this machine's debug key.
//!
//! So building for Android needs no Android SDK, NDK or Java.

pub mod axml;
pub mod der;
pub mod key;
pub mod pem;
pub mod sha256;
pub mod sign;
pub mod zip;

use std::path::{Path, PathBuf};

use crate::{archive, BuildConfig, Exported, Message, Target};
use axml::ManifestPatch;
use key::RsaKey;

/// The template's file name in its template folder.
pub const TEMPLATE_FILE: &str = "automata-template.apk";
/// Where the game's files go in the APK.
pub const GAME_PREFIX: &str = "assets/automata/game/";
/// The APK entry holding the hash of the game files.
pub const STAMP: &str = "assets/automata/stamp";

/// A length, offset or index as a 32-bit field. [`build_apk`] stops
/// before an APK reaches 4 GB, so every one fits.
pub(crate) fn n32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A length or count as a 16-bit field (names, extra fields and entry
/// counts are checked by [`build_apk`]).
pub(crate) fn n16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// The lowest byte of `n`.
pub(crate) fn low_byte(n: usize) -> u8 {
    n.to_le_bytes()[0]
}

/// A 64-bit size as `usize`.
pub(crate) fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Whether `name` is a JAR signature file, which a new signature replaces.
fn old_signature(name: &str) -> bool {
    let Some(file) = name.strip_prefix("META-INF/") else {
        return false;
    };
    let upper = file.to_ascii_uppercase();
    upper == "MANIFEST.MF"
        || [".SF", ".RSA", ".DSA", ".EC"]
            .iter()
            .any(|ext| upper.ends_with(ext))
}

/// The hash that tells the player its unpacked game is out of date.
#[must_use]
pub fn game_stamp(files: &[(String, Vec<u8>)]) -> String {
    let mut h = sha256::Sha256::new();
    for (name, bytes) in files {
        h.update(name.as_bytes());
        h.update(&[0]);
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    hex(&h.finish())
}

/// A signed APK: `template` with `game`'s files (paths relative to the
/// game folder) and `patch`'s app ID, name and version.
///
/// # Errors
///
/// When the template isn't an APK this understands.
pub fn build_apk(
    template: &[u8],
    patch: &ManifestPatch,
    game: &[(String, Vec<u8>)],
    key: &RsaKey,
) -> Result<Vec<u8>, String> {
    build_apk_for(template, patch, game, key, &[])
}

/// The ABI a `lib/<abi>/…` entry is for.
fn lib_abi(name: &str) -> Option<&str> {
    name.strip_prefix("lib/")?
        .split_once('/')
        .map(|(abi, _)| abi)
}

/// [`build_apk`] with only the native code for `abis` (all when empty).
///
/// # Errors
///
/// As [`build_apk`], and when the template has no code for one of `abis`.
pub fn build_apk_for(
    template: &[u8],
    patch: &ManifestPatch,
    game: &[(String, Vec<u8>)],
    key: &RsaKey,
    abis: &[String],
) -> Result<Vec<u8>, String> {
    let mut entries = Vec::new();
    let mut found = (false, false);
    let mut have = std::collections::BTreeSet::new();
    for e in zip::read(template)? {
        if old_signature(&e.name) || e.name.starts_with("assets/automata/") {
            continue;
        }
        if let Some(abi) = lib_abi(&e.name) {
            have.insert(abi.to_string());
            if !abis.is_empty() && !abis.iter().any(|a| a == abi) {
                continue;
            }
        }
        match e.name.as_str() {
            "AndroidManifest.xml" => {
                let (xml, _) = axml::patch_manifest(&e.bytes()?, patch)?;
                entries.push(zip::Entry::new(&e.name, &xml));
                found.0 = true;
            }
            "resources.arsc" => {
                let mut arsc = e.bytes()?;
                axml::rename_resource_package(&mut arsc, &patch.package)?;
                // Android 11+ wants it stored, so it can be mapped.
                entries.push(zip::Entry {
                    method: 0,
                    crc: crc32fast::hash(&arsc),
                    size: n32(arsc.len()),
                    data: arsc,
                    name: e.name,
                });
                found.1 = true;
            }
            _ => entries.push(e),
        }
    }
    if found != (true, true) {
        return Err("the template is not an APK (no manifest or resources)".into());
    }
    if let Some(missing) = abis.iter().find(|a| !have.contains(*a)) {
        let list: Vec<&str> = have.iter().map(String::as_str).collect();
        return Err(format!(
            "the template has no {missing} code (it has: {}); check android.abis",
            list.join(", ")
        ));
    }
    for (name, bytes) in game {
        entries.push(zip::Entry::new(&format!("{GAME_PREFIX}{name}"), bytes));
    }
    entries.push(zip::Entry::new(STAMP, game_stamp(game).as_bytes()));
    if entries.len() > 0xffff || entries.iter().any(|e| e.name.len() > 0xff00) {
        return Err("too many files, or a path too long, for an APK".into());
    }
    let written = zip::write(&entries);
    if written.entries.len() + written.central.len() > 0xf000_0000 {
        return Err("the game is too large for an APK (4 GB)".into());
    }
    Ok(sign::sign(&written, key))
}

/// Every file under `dir`, as (`a/b.txt`, bytes), sorted.
fn read_tree(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let entries = archive::entries(dir, "", &|_| false).map_err(|e| e.to_string())?;
    entries
        .into_iter()
        .map(|e| {
            std::fs::read(&e.source)
                .map(|b| (e.name, b))
                .map_err(|err| format!("{}: {err}", e.source.display()))
        })
        .collect()
}

/// Builds `project` into an APK from the template in `template` (a folder
/// with [`TEMPLATE_FILE`]): `build/android/<name>-<version>.apk`.
///
/// # Errors
///
/// When the template is missing or damaged, the app ID is invalid, the
/// signing key can't be loaded or made, or files can't be read or written.
pub fn export(
    project: &Path,
    cfg: &BuildConfig,
    template: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<Exported, String> {
    export_with_key(project, cfg, template, &key::key_path(), progress)
}

/// [`export`], signing with the key in `key_file` (made when missing).
///
/// # Errors
///
/// As [`export`].
pub fn export_with_key(
    project: &Path,
    cfg: &BuildConfig,
    template: &Path,
    key_file: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<Exported, String> {
    let template_apk = template.join(TEMPLATE_FILE);
    let template_bytes = std::fs::read(&template_apk).map_err(|e| {
        format!(
            "no Android template at {} ({e}; Build Settings → Download templates)",
            template_apk.display()
        )
    })?;
    let package = cfg.android_package();
    crate::check_android_package(&package)?;
    if cfg.android.package.trim().is_empty() {
        progress(Message::warn(format!(
            "Android: the app ID is {package}. Set your own (android.package in build.ron, or Build Settings) before publishing: it can't change once people install the game."
        )));
    }
    let (key, signed_with) = signing_key(project, cfg, key_file, progress)?;
    let folder = project.join(&cfg.output).join(Target::Android.name());
    let game_dir = folder.join("game");
    let n = crate::copy_game(project, cfg, &game_dir)?;
    let game = read_tree(&game_dir);
    // The files are in memory now: the APK is the only thing to leave.
    let _ = std::fs::remove_dir_all(&game_dir);
    let game = game?;
    progress(Message::info(format!("Android: game ({n} files)")));

    let patch = ManifestPatch {
        package: package.clone(),
        label: cfg.name.trim().to_string(),
        version_code: cfg.android.version_code.max(1),
        version_name: cfg.version.trim().to_string(),
    };
    let abis: Vec<String> = cfg
        .android
        .abis
        .iter()
        .map(|a| a.trim().to_string())
        .collect();
    let apk = build_apk_for(&template_bytes, &patch, &game, &key, &abis)
        .map_err(|e| format!("{}: {e}", template_apk.display()))?;
    let path = folder.join(format!("{}-{}.apk", cfg.file_name(), cfg.version.trim()));
    std::fs::write(&path, &apk).map_err(|e| format!("{}: {e}", path.display()))?;
    let abis = if abis.is_empty() {
        String::new()
    } else {
        format!(" ({})", abis.join(", "))
    };
    progress(Message::info(format!(
        "Android: {package}{abis}, {:.1} MB, signed with {signed_with} (SHA-256 {})",
        apk.len() as f64 / 1e6,
        &sign::certificate_fingerprint(&key)[..23]
    )));
    progress(Message::info(format!("Android: done → {}", path.display())));
    Ok(Exported {
        target: Target::Android,
        folder,
        executable: path,
        archive: None,
        standalone: true,
    })
}

/// The key a build signs with, and how to name it in messages: the
/// release key when one is set (`android.key` or
/// `$AUTOMATA_ANDROID_RELEASE_KEY`), else the debug key in `debug_file`,
/// made on first use.
fn signing_key(
    project: &Path,
    cfg: &BuildConfig,
    debug_file: &Path,
    progress: &mut dyn FnMut(Message),
) -> Result<(RsaKey, String), String> {
    if let Some(path) = pem::release_key_path(project, &cfg.android.key) {
        let inside = |p: &Path| {
            let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            real(p).starts_with(real(project))
        };
        if inside(&path) {
            return Err(format!(
                "the release key {} is inside the project folder: move it out (it would ship inside the game, and end up in version control)",
                path.display()
            ));
        }
        let key = pem::load_release_key(&path)?;
        return Ok((key, format!("your release key {}", path.display())));
    }
    let (key, new) = key::load_or_create(debug_file)?;
    if new {
        progress(Message::info(format!(
            "Android: made a debug signing key in {}. Keep it: builds signed with another key can't install over these.",
            debug_file.display()
        )));
    }
    progress(Message::info(
        "Android: signing with this machine's debug key, for testing. To publish, make your own key (automata-cli keygen FILE) and set android.key in build.ron.",
    ));
    Ok((key, "the debug key".into()))
}

/// The installed Android template folder's APK, if there is one.
#[must_use]
pub fn installed_template(dir: &Path) -> Option<PathBuf> {
    let apk = dir.join(TEMPLATE_FILE);
    apk.is_file().then_some(apk)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A template-like APK: manifest, resources, a library, a JAR
    /// signature and an old game.
    pub(crate) fn template() -> Vec<u8> {
        let entries = vec![
            zip::Entry::new("AndroidManifest.xml", &axml::tests::sample_manifest(true)),
            zip::Entry::new(
                "resources.arsc",
                &axml::tests::sample_arsc("org.automata.template"),
            ),
            zip::Entry::new("lib/x86_64/libpython.so", &[0x7f; 3000]),
            zip::Entry::new("META-INF/CERT.RSA", b"old"),
            zip::Entry::new("assets/automata/game/old.txt", b"stale"),
            zip::Entry::new("assets/private.tar", b"player"),
        ];
        zip::write(&entries).concat()
    }

    #[test]
    fn builds_keep_only_the_chosen_abis() {
        let key = test_key();
        let patch = ManifestPatch {
            package: "com.example.coinrun".into(),
            label: "Coin Run".into(),
            version_code: 1,
            version_name: "0.1.0".into(),
        };
        let x86 = vec!["x86_64".to_string()];
        let apk = build_apk_for(&template(), &patch, &[], &key, &x86).unwrap();
        let names: Vec<String> = zip::read(&apk)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(
            names.contains(&"lib/x86_64/libpython.so".to_string()),
            "{names:?}"
        );
        let phones = vec!["arm64-v8a".to_string()];
        let err = build_apk_for(&template(), &patch, &[], &key, &phones).unwrap_err();
        assert!(err.contains("no arm64-v8a code (it has: x86_64)"), "{err}");
        assert_eq!(lib_abi("lib/arm64-v8a/libmain.so"), Some("arm64-v8a"));
        assert_eq!(lib_abi("assets/lib/x.so"), None);
    }

    fn test_key() -> RsaKey {
        RsaKey::generate(1024).unwrap()
    }

    #[test]
    fn a_game_apk_holds_the_game_and_is_signed() {
        let key = test_key();
        let patch = ManifestPatch {
            package: "com.example.coinrun".into(),
            label: "Coin Run".into(),
            version_code: 3,
            version_name: "0.3.0".into(),
        };
        let game = vec![
            ("game.json".to_string(), b"{}".to_vec()),
            ("scripts/rules.py".to_string(), b"print('hi')\n".repeat(50)),
        ];
        let apk = build_apk(&template(), &patch, &game, &key).unwrap();
        sign::verify(&apk, &key).unwrap();
        let entries = zip::read(&apk).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "AndroidManifest.xml",
                "resources.arsc",
                "lib/x86_64/libpython.so",
                "assets/private.tar",
                "assets/automata/game/game.json",
                "assets/automata/game/scripts/rules.py",
                "assets/automata/stamp",
            ]
        );
        let arsc = entries.iter().find(|e| e.name == "resources.arsc").unwrap();
        assert_eq!(arsc.method, 0);
        assert_eq!(
            axml::resource_package(&arsc.bytes().unwrap()).unwrap(),
            "com.example.coinrun"
        );
        let stamp = entries.last().unwrap().bytes().unwrap();
        assert_eq!(String::from_utf8(stamp).unwrap(), game_stamp(&game));
        assert_ne!(game_stamp(&game), game_stamp(&game[..1]));

        // Same inputs, same APK.
        assert_eq!(build_apk(&template(), &patch, &game, &key).unwrap(), apk);
        // Tampering breaks the signature.
        let mut bad = apk.clone();
        bad[40] ^= 1;
        assert!(sign::verify(&bad, &key).is_err());
    }

    #[test]
    fn not_a_template() {
        let key = test_key();
        let patch = ManifestPatch {
            package: "a.b".into(),
            label: "x".into(),
            version_code: 1,
            version_name: "1".into(),
        };
        let only = zip::write(&[zip::Entry::new("x.txt", b"x")]).concat();
        assert!(build_apk(&only, &patch, &[], &key).is_err());
        assert!(build_apk(b"junk", &patch, &[], &key).is_err());
    }

    #[test]
    fn jar_signature_files_are_dropped() {
        assert!(old_signature("META-INF/CERT.RSA"));
        assert!(old_signature("META-INF/MANIFEST.MF"));
        assert!(old_signature("META-INF/KEY0.sf"));
        assert!(!old_signature("META-INF/services/x"));
        assert!(!old_signature("CERT.RSA"));
    }
}
