//! The RSA key that signs Android builds, and its certificate.
//!
//! Like Android Studio's `debug.keystore`, each machine makes its own
//! debug key the first time it builds for Android and keeps it, so a new
//! build of a game installs over the last one (Android only accepts
//! updates signed by the same certificate). The certificate is derived
//! from the key, so it stays the same too.

use std::path::{Path, PathBuf};

use num_bigint::BigUint;
use num_traits::{One, Zero};

use super::der;
use super::sha256::sha256;

/// The `DigestInfo` prefix of a SHA-256 digest (PKCS #1).
const SHA256_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

/// An RSA key pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsaKey {
    /// The modulus.
    pub n: BigUint,
    /// The public exponent.
    pub e: BigUint,
    /// The private exponent.
    pub d: BigUint,
}

impl RsaKey {
    /// Makes a new key with a `bits`-bit modulus (2048 for real use).
    ///
    /// # Errors
    ///
    /// When the system has no random numbers to give.
    pub fn generate(bits: u64) -> Result<Self, String> {
        let e = BigUint::from(65_537u32);
        loop {
            let p = random_prime(bits / 2, &e)?;
            let q = random_prime(bits - bits / 2, &e)?;
            if p == q {
                continue;
            }
            let n = &p * &q;
            if n.bits() != bits {
                continue;
            }
            let phi = (&p - 1u32) * (&q - 1u32);
            if let Some(d) = e.modinv(&phi) {
                return Ok(Self { n, e, d });
            }
        }
    }

    /// The modulus length in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        super::to_usize(self.n.bits().div_ceil(8))
    }

    /// An RSASSA-PKCS1-v1_5 signature of `message` with SHA-256.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let k = self.size();
        let digest = sha256(message);
        let mut em = vec![0xffu8; k];
        em[0] = 0;
        em[1] = 1;
        let t = k - SHA256_PREFIX.len() - digest.len();
        em[t - 1] = 0;
        em[t..t + SHA256_PREFIX.len()].copy_from_slice(&SHA256_PREFIX);
        em[t + SHA256_PREFIX.len()..].copy_from_slice(&digest);
        let s = BigUint::from_bytes_be(&em).modpow(&self.d, &self.n);
        left_pad(&s.to_bytes_be(), k)
    }

    /// Whether `signature` is this key's signature of `message`.
    #[must_use]
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        let m = BigUint::from_bytes_be(signature).modpow(&self.e, &self.n);
        let k = self.size();
        let em = left_pad(&m.to_bytes_be(), k);
        let digest = sha256(message);
        em[..2] == [0, 1] && em.ends_with(&digest) && em[..k - 32].ends_with(&SHA256_PREFIX)
    }

    /// The public key as a DER `SubjectPublicKeyInfo`.
    #[must_use]
    pub fn public_key_der(&self) -> Vec<u8> {
        let rsa = der::sequence(&[
            &der::unsigned(&self.n.to_bytes_be()),
            &der::unsigned(&self.e.to_bytes_be()),
        ]);
        let algorithm = der::sequence(&[&der::RSA_ENCRYPTION, &der::NULL]);
        der::sequence(&[&algorithm, &der::bit_string(&rsa)])
    }

    /// A self-signed X.509 certificate for the key (DER). It only depends
    /// on the key, so it's the same every build.
    #[must_use]
    pub fn certificate(&self) -> Vec<u8> {
        let algorithm = der::sequence(&[&der::SHA256_WITH_RSA, &der::NULL]);
        let mut serial = sha256(&self.n.to_bytes_be())[..8].to_vec();
        serial[0] = (serial[0] & 0x7f) | 0x01;
        let version = der::tlv(0xa0, &der::unsigned(&[2]));
        let name = der::name("AUTOMATA Android Debug", "AUTOMATA");
        let validity = der::sequence(&[
            &der::tlv(0x17, b"260101000000Z"),
            &der::tlv(0x18, b"20760101000000Z"),
        ]);
        let tbs = der::sequence(&[
            &version,
            &der::unsigned(&serial),
            &algorithm,
            &name,
            &validity,
            &name,
            &self.public_key_der(),
        ]);
        let signature = self.sign(&tbs);
        der::sequence(&[&tbs, &algorithm, &der::bit_string(&signature)])
    }

    /// The key as text (the key file's format).
    #[must_use]
    pub fn to_text(&self) -> String {
        format!(
            "# AUTOMATA Android debug key (RSA). Keep this file: a game signed with\n# another key can't install over the builds signed with this one.\nn={}\ne={}\nd={}\n",
            self.n.to_str_radix(16),
            self.e.to_str_radix(16),
            self.d.to_str_radix(16)
        )
    }

    /// Reads a key written by [`RsaKey::to_text`].
    ///
    /// # Errors
    ///
    /// When a number is missing or malformed.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let field = |name: &str| -> Result<BigUint, String> {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('='))
                .and_then(|v| BigUint::parse_bytes(v.trim().as_bytes(), 16))
                .ok_or_else(|| format!("no `{name}` in the key file"))
        };
        let key = Self {
            n: field("n")?,
            e: field("e")?,
            d: field("d")?,
        };
        if key.n.bits() < 1024 || key.e.is_zero() || key.d.is_zero() {
            return Err("the key file holds no usable key".into());
        }
        Ok(key)
    }
}

/// Where the debug key lives: `$AUTOMATA_ANDROID_KEY`, else
/// `android-debug.key` in `~/.config/automata` (Linux, macOS) or
/// `%APPDATA%\automata` (Windows).
#[must_use]
pub fn key_path() -> PathBuf {
    if let Some(p) = std::env::var_os("AUTOMATA_ANDROID_KEY") {
        return PathBuf::from(p);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
    .unwrap_or_else(std::env::temp_dir);
    base.join("automata").join("android-debug.key")
}

/// Loads the key at `path`, making (and saving) a new one when there's
/// none. Returns the key and whether it's new.
///
/// # Errors
///
/// When the file can't be read, holds no key, or can't be written.
pub fn load_or_create(path: &Path) -> Result<(RsaKey, bool), String> {
    match std::fs::read_to_string(path) {
        Ok(text) => RsaKey::from_text(&text)
            .map(|k| (k, false))
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = RsaKey::generate(2048)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            write_private(path, &key.to_text()).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((key, true))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}

fn left_pad(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut out = vec![0; len.saturating_sub(bytes.len())];
    out.extend_from_slice(bytes);
    out
}

fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| format!("no random numbers: {e}"))?;
    Ok(buf)
}

/// The odd primes below 2000, to rule out most candidates quickly.
fn small_primes() -> Vec<u32> {
    let mut sieve = vec![true; 2000];
    let mut primes = Vec::new();
    for i in 3..2000 {
        if sieve[i] {
            primes.push(super::n32(i));
            for j in (i * i..2000).step_by(i) {
                sieve[j] = false;
            }
        }
    }
    primes
}

/// A random `bits`-bit prime p with gcd(p - 1, e) = 1 and its top two
/// bits set (so two of them make a modulus of the full size).
fn random_prime(bits: u64, e: &BigUint) -> Result<BigUint, String> {
    let small = small_primes();
    let bytes = super::to_usize(bits.div_ceil(8));
    loop {
        let mut raw = random_bytes(bytes)?;
        let extra = bytes as u64 * 8 - bits;
        raw[0] &= 0xff >> extra;
        raw[0] |= 0xc0 >> extra;
        if extra == 7 {
            raw[1] |= 0x80;
        }
        raw[bytes - 1] |= 1;
        let candidate = BigUint::from_bytes_be(&raw);
        if small
            .iter()
            .any(|&p| (&candidate % p).is_zero() && candidate != BigUint::from(p))
        {
            continue;
        }
        if ((&candidate - 1u32) % e).is_zero() {
            continue;
        }
        if probably_prime(&candidate, 24)? {
            return Ok(candidate);
        }
    }
}

/// Miller-Rabin with `rounds` random bases.
#[allow(clippy::many_single_char_names)] // the algorithm's usual names
fn probably_prime(n: &BigUint, rounds: usize) -> Result<bool, String> {
    let one = BigUint::one();
    let two = BigUint::from(2u32);
    if *n < BigUint::from(4u32) {
        return Ok(*n >= two);
    }
    let n1 = n - &one;
    let s = n1.trailing_zeros().unwrap_or(0);
    let d = &n1 >> s;
    let len = super::to_usize(n.bits().div_ceil(8));
    'rounds: for _ in 0..rounds {
        let a = BigUint::from_bytes_be(&random_bytes(len + 8)?) % (n - 3u32) + &two;
        let mut x = a.modpow(&d, n);
        if x == one || x == n1 {
            continue;
        }
        for _ in 1..s {
            x = x.modpow(&two, n);
            if x == n1 {
                continue 'rounds;
            }
        }
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primes_are_told_apart() {
        for p in [5u32, 7, 65_537, 2_147_483_647] {
            assert!(probably_prime(&BigUint::from(p), 16).unwrap(), "{p}");
        }
        for c in [9u32, 561, 41_041, 2_147_483_649] {
            assert!(!probably_prime(&BigUint::from(c), 16).unwrap(), "{c}");
        }
    }

    #[test]
    fn keys_sign_verify_and_round_trip() {
        let key = RsaKey::generate(1024).unwrap();
        assert_eq!(key.n.bits(), 1024);
        let sig = key.sign(b"hello");
        assert_eq!(sig.len(), 128);
        assert!(key.verify(b"hello", &sig));
        assert!(!key.verify(b"hellp", &sig));
        let back = RsaKey::from_text(&key.to_text()).unwrap();
        assert_eq!(back, key);
        assert_eq!(key.certificate(), back.certificate());
        assert_eq!(key.certificate()[0], 0x30);
    }

    #[test]
    fn a_key_file_is_made_once_and_then_reused() {
        let dir = std::env::temp_dir().join(format!("automata-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("debug.key");
        let (first, new) = load_or_create(&path).unwrap();
        assert!(new);
        assert_eq!(first.n.bits(), 2048);
        let (again, new) = load_or_create(&path).unwrap();
        assert!(!new);
        assert_eq!(again, first);
        std::fs::write(&path, "n=zz").unwrap();
        assert!(load_or_create(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
