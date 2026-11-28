//! Release keys: the developer's own RSA key, in a standard PEM file.
//!
//! - `automata-cli keygen` writes one: the private key (PKCS #8,
//!   `BEGIN PRIVATE KEY`) and a self-signed certificate
//!   (`BEGIN CERTIFICATE`) in one file.
//! - A key already in use elsewhere works too, and its certificate comes
//!   along, so apps published with it can be updated. From a Java keystore
//!   (Android Studio's `.jks`):
//!
//!   ```text
//!   keytool -importkeystore -srckeystore release.jks -destkeystore release.p12 -deststoretype PKCS12
//!   openssl pkcs12 -in release.p12 -nodes -out release.pem
//!   ```
//!
//! PKCS #1 keys (`BEGIN RSA PRIVATE KEY`) work as well. Encrypted keys
//! don't: the build would need the password.

use std::path::{Path, PathBuf};

use base64::Engine;
use num_bigint::BigUint;

use super::der::{self, Reader};
use super::key::{self, RsaKey};

/// The environment variable that names a release key file, ahead of
/// `build.ron` (handy for CI secrets).
pub const RELEASE_KEY_ENV: &str = "AUTOMATA_ANDROID_RELEASE_KEY";

/// One `-----BEGIN LABEL-----` … `-----END LABEL-----` block.
struct Block {
    label: String,
    der: Vec<u8>,
    encrypted: bool,
}

/// The PEM blocks in `text`; anything outside them (comments, the
/// "Bag Attributes" openssl writes) is skipped.
fn blocks(text: &str) -> Result<Vec<Block>, String> {
    let mut out = Vec::new();
    let mut current: Option<(String, String, bool)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            current = Some((label.to_string(), String::new(), false));
        } else if line.starts_with("-----END ") {
            let (label, body, encrypted) = current.take().ok_or("a PEM END without a BEGIN")?;
            let der = base64::engine::general_purpose::STANDARD
                .decode(body.as_bytes())
                .map_err(|e| format!("the {label} block isn't valid base64 ({e})"))?;
            out.push(Block {
                label,
                der,
                encrypted,
            });
        } else if let Some((_, body, encrypted)) = current.as_mut() {
            if line.contains(':') {
                // Old-style headers, like `Proc-Type: 4,ENCRYPTED`.
                *encrypted |= line.contains("ENCRYPTED");
            } else {
                body.push_str(line);
            }
        }
    }
    Ok(out)
}

/// The message for a key that needs a password.
const ENCRYPTED: &str = "the key is encrypted (it has a password). Export it without one, e.g. `openssl pkcs8 -in key.pem -nocrypt -out key-plain.pem` or `openssl pkcs12 -in release.p12 -nodes -out release.pem`, and keep that file private";

/// Reads a release key: a PEM file with an RSA private key and, maybe,
/// its certificate. Also accepts the debug key's own text format.
///
/// # Errors
///
/// When there's no usable RSA private key, it's encrypted, or the
/// certificate is for another key.
pub fn read_key(text: &str) -> Result<RsaKey, String> {
    let blocks = blocks(text)?;
    if blocks.is_empty() {
        if text.lines().any(|l| l.trim_start().starts_with("n=")) {
            return RsaKey::from_text(text);
        }
        return Err("no key in the file (expected a PEM key: -----BEGIN PRIVATE KEY-----)".into());
    }
    let block = blocks
        .iter()
        .find(|b| b.label.ends_with("PRIVATE KEY"))
        .ok_or("no private key in the file (only a certificate?)")?;
    if block.encrypted || block.label.contains("ENCRYPTED") {
        return Err(ENCRYPTED.into());
    }
    let mut key = match block.label.as_str() {
        "PRIVATE KEY" => pkcs8(&block.der)?,
        "RSA PRIVATE KEY" => pkcs1(&block.der)?,
        other => {
            return Err(format!(
                "only RSA keys can sign APKs here (this is a {other})"
            ))
        }
    };
    if let Some(cert) = blocks.iter().find(|b| b.label == "CERTIFICATE") {
        let public = key.public_key_der();
        if !cert.der.windows(public.len()).any(|w| w == &public[..]) {
            return Err("the certificate in the file belongs to another key".into());
        }
        key.own_certificate = Some(cert.der.clone());
    }
    check(&key)?;
    Ok(key)
}

/// A PKCS #8 `PrivateKeyInfo` holding an RSA key.
fn pkcs8(data: &[u8]) -> Result<RsaKey, String> {
    let mut info = Reader::new(Reader::new(data).expect(0x30)?);
    info.integer()?;
    let algorithm = info.expect(0x30)?;
    if !algorithm.starts_with(&der::RSA_ENCRYPTION) {
        return Err("only RSA keys can sign APKs here (this key is another type, like EC)".into());
    }
    pkcs1(info.expect(0x04)?)
}

/// A PKCS #1 `RSAPrivateKey`.
fn pkcs1(data: &[u8]) -> Result<RsaKey, String> {
    let mut rsa = Reader::new(Reader::new(data).expect(0x30)?);
    rsa.integer()?;
    let mut int = || rsa.integer().map(BigUint::from_bytes_be);
    Ok(RsaKey {
        n: int()?,
        e: int()?,
        d: int()?,
        primes: Some((int()?, int()?)),
        own_certificate: None,
    })
}

/// Checks that a key read from a file holds together and can sign.
fn check(key: &RsaKey) -> Result<(), String> {
    if key.n.bits() < 2048 {
        return Err(format!(
            "the key has {} bits; Android stores want at least 2048",
            key.n.bits()
        ));
    }
    let primes_fit = key.primes.as_ref().is_none_or(|(p, q)| p * q == key.n);
    let sig = key.sign(b"automata");
    if !primes_fit || !key.verify(b"automata", &sig) {
        return Err("the key file's numbers don't make a working RSA key".into());
    }
    Ok(())
}

/// `der` as a PEM block.
fn pem(label: &str, der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(line));
        out.push('\n');
    }
    out + "-----END " + label + "-----\n"
}

/// The key as a PEM file: the private key (PKCS #8) and its certificate
/// (its own, or a self-signed one naming `common_name`).
///
/// # Errors
///
/// When the key's primes aren't known (a debug key file has none).
pub fn write_key(key: &RsaKey, common_name: &str) -> Result<String, String> {
    let (p, q) = key
        .primes
        .as_ref()
        .ok_or("this key can't be written as a PEM file (its primes aren't known)")?;
    let one = BigUint::from(1u32);
    let qinv = q.modinv(p).ok_or("the key's primes are not coprime")?;
    let ints = [
        &key.n,
        &key.e,
        &key.d,
        p,
        q,
        &(&key.d % (p - &one)),
        &(&key.d % (q - &one)),
        &qinv,
    ]
    .map(|i| der::unsigned(&i.to_bytes_be()));
    let mut parts: Vec<&[u8]> = vec![&[0x02, 0x01, 0x00]];
    parts.extend(ints.iter().map(Vec::as_slice));
    let rsa = der::sequence(&parts);
    let algorithm = der::sequence(&[&der::RSA_ENCRYPTION, &der::NULL]);
    let info = der::sequence(&[&[0x02, 0x01, 0x00], &algorithm, &der::octets(&rsa)]);
    let cert = key
        .own_certificate
        .clone()
        .unwrap_or_else(|| key.self_signed(common_name, ""));
    Ok(format!(
        "AUTOMATA Android release key. Keep this file private and back it up:\n\
         updates of your game must be signed with it, and it can't be replaced.\n\n{}{}",
        pem("PRIVATE KEY", &info),
        pem("CERTIFICATE", &cert)
    ))
}

/// Makes a release key (RSA 2048) at `path`, with a certificate naming
/// `common_name`. Never overwrites a file.
///
/// # Errors
///
/// When `path` exists or can't be written.
pub fn create_release_key(path: &Path, common_name: &str) -> Result<RsaKey, String> {
    if path.exists() {
        return Err(format!(
            "{} already exists; not replacing a key (a lost key means no more updates)",
            path.display()
        ));
    }
    let key = RsaKey::generate(2048)?;
    let text = write_key(&key, common_name)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    key::write_private(path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    read_key(&text)
}

/// The release key file for a build, if one is set: `$AUTOMATA_ANDROID_RELEASE_KEY`,
/// else `configured` (`build.ron`'s `android.key`). `~/` is the home
/// folder; other relative paths are from the project folder.
#[must_use]
pub fn release_key_path(project: &Path, configured: &str) -> Option<PathBuf> {
    let env = std::env::var(RELEASE_KEY_ENV).ok();
    let set = env
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| Some(configured.trim()).filter(|s| !s.is_empty()))?;
    if let Some(rest) = set.strip_prefix("~/").or_else(|| set.strip_prefix("~\\")) {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(home) = home {
            return Some(PathBuf::from(home).join(rest));
        }
    }
    let path = PathBuf::from(set);
    Some(if path.is_absolute() {
        path
    } else {
        project.join(path)
    })
}

/// Where a new release key for `package` goes by default: AUTOMATA's
/// settings folder, outside every project.
#[must_use]
pub fn suggested_release_key_path(package: &str) -> PathBuf {
    let name: String = package
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    key::config_dir().join("release-keys").join(format!(
        "{}.pem",
        if name.is_empty() { "game" } else { &name }
    ))
}

/// Loads the release key at `path`.
///
/// # Errors
///
/// When the file is missing or holds no usable key.
pub fn load_release_key(path: &Path) -> Result<RsaKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "release key {}: {e} (make one with `automata-cli keygen {}`)",
            path.display(),
            path.display()
        )
    })?;
    read_key(&text).map_err(|e| format!("release key {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> RsaKey {
        RsaKey::generate(2048).unwrap()
    }

    #[test]
    fn a_written_key_reads_back_with_its_certificate() {
        let key = key();
        let text = write_key(&key, "Jane Doe").unwrap();
        assert!(text.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(text.contains("-----BEGIN CERTIFICATE-----"));
        assert!(text.lines().all(|l| l.len() <= 76));
        let back = read_key(&text).unwrap();
        assert_eq!((&back.n, &back.e, &back.d), (&key.n, &key.e, &key.d));
        assert_eq!(back.certificate(), key.self_signed("Jane Doe", ""));
        // The certificate is what's in the file, not the debug one.
        assert_ne!(back.certificate(), key.certificate());
        // Writing it again keeps that certificate.
        assert_eq!(
            read_key(&write_key(&back, "Other").unwrap())
                .unwrap()
                .certificate(),
            back.certificate()
        );
    }

    #[test]
    fn pkcs1_keys_and_keys_without_a_certificate_work() {
        let key = key();
        let (p, q) = key.primes.clone().unwrap();
        let one = BigUint::from(1u32);
        let ints = [
            &key.n,
            &key.e,
            &key.d,
            &p,
            &q,
            &(&key.d % (&p - &one)),
            &(&key.d % (&q - &one)),
            &q.modinv(&p).unwrap(),
        ]
        .map(|i| der::unsigned(&i.to_bytes_be()));
        let mut parts: Vec<&[u8]> = vec![&[0x02, 0x01, 0x00]];
        parts.extend(ints.iter().map(Vec::as_slice));
        let text = format!(
            "Bag Attributes\n    friendlyName: release\n{}",
            pem("RSA PRIVATE KEY", &der::sequence(&parts))
        );
        let back = read_key(&text).unwrap();
        assert_eq!(back.n, key.n);
        assert_eq!(back.own_certificate, None);
    }

    #[test]
    fn unusable_files_say_why() {
        let key = key();
        let other = RsaKey::generate(2048).unwrap();
        let text = write_key(&key, "A").unwrap();
        let cert = text.find("-----BEGIN CERTIFICATE").unwrap();
        let mismatched = format!(
            "{}{}",
            &text[..cert],
            pem("CERTIFICATE", &other.self_signed("B", ""))
        );
        assert!(read_key(&mismatched).unwrap_err().contains("another key"));
        assert!(read_key(&text[cert..])
            .unwrap_err()
            .contains("no private key"));
        let encrypted = pem("ENCRYPTED PRIVATE KEY", b"\x30\x00");
        assert!(read_key(&encrypted).unwrap_err().contains("encrypted"));
        let ec = pem("EC PRIVATE KEY", b"\x30\x00");
        assert!(read_key(&ec).unwrap_err().contains("only RSA"));
        assert!(read_key("hello").unwrap_err().contains("no key"));
        let small = RsaKey::generate(1024).unwrap();
        assert!(read_key(&write_key(&small, "S").unwrap())
            .unwrap_err()
            .contains("2048"));
    }

    #[test]
    fn release_keys_are_made_once_and_found() {
        let dir = std::env::temp_dir().join(format!("automata-pem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("keys").join("release.pem");
        let made = create_release_key(&path, "Studio").unwrap();
        assert!(create_release_key(&path, "Studio")
            .unwrap_err()
            .contains("already exists"));
        let loaded = load_release_key(&path).unwrap();
        assert_eq!(loaded.certificate(), made.certificate());
        assert!(load_release_key(&dir.join("none.pem"))
            .unwrap_err()
            .contains("keygen"));

        let project = Path::new("/games/coin_run");
        assert_eq!(release_key_path(project, ""), None);
        assert_eq!(
            release_key_path(project, "keys/r.pem"),
            Some(project.join("keys/r.pem"))
        );
        let absolute = std::env::temp_dir().join("r.pem");
        assert_eq!(
            release_key_path(project, &absolute.to_string_lossy()),
            Some(absolute)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
