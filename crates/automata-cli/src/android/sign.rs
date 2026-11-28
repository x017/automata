//! APK Signature Scheme v2: the signing block Android 7+ checks before it
//! installs an APK. See
//! <https://source.android.com/docs/security/features/apksigning/v2>.
//!
//! The signature covers the whole file in 1 MB chunks: the entries, the
//! central directory, and the end record (whose central directory offset
//! counts as if the signing block weren't there). The block goes between
//! the entries and the central directory.

use super::key::RsaKey;
use super::sha256::{sha256, Sha256};
use super::zip::{end_record, Written};

/// RSASSA-PKCS1-v1_5 with SHA2-256.
const RSA_PKCS1_SHA256: u32 = 0x0103;
/// The v2 block's ID in the signing block.
const V2_BLOCK_ID: u32 = 0x7109_871a;
const MAGIC: &[u8; 16] = b"APK Sig Block 42";
const CHUNK: usize = 1 << 20;

/// `data` with a little-endian u32 length before it.
fn prefixed(data: &[u8]) -> Vec<u8> {
    let mut out = super::n32(data.len()).to_le_bytes().to_vec();
    out.extend_from_slice(data);
    out
}

/// The v2 content digest of the three parts.
#[must_use]
pub fn content_digest(parts: &[&[u8]]) -> [u8; 32] {
    let chunks: Vec<&[u8]> = parts.iter().flat_map(|p| p.chunks(CHUNK)).collect();
    let mut top = Sha256::new();
    top.update(&[0x5a]);
    top.update(&super::n32(chunks.len()).to_le_bytes());
    for chunk in chunks {
        let mut h = Sha256::new();
        h.update(&[0xa5]);
        h.update(&super::n32(chunk.len()).to_le_bytes());
        h.update(chunk);
        top.update(&h.finish());
    }
    top.finish()
}

/// The signed-data structure for `digest` and the certificate.
fn signed_data(digest: &[u8; 32], certificate: &[u8]) -> Vec<u8> {
    let mut d = RSA_PKCS1_SHA256.to_le_bytes().to_vec();
    d.extend(prefixed(digest));
    let digests = prefixed(&prefixed(&d));
    let certificates = prefixed(&prefixed(certificate));
    let attributes = prefixed(&[]);
    [digests, certificates, attributes].concat()
}

/// The APK Signing Block for an APK whose parts digest to `digest`.
#[must_use]
pub fn signing_block(digest: &[u8; 32], key: &RsaKey) -> Vec<u8> {
    let data = signed_data(digest, &key.certificate());
    let mut sig = RSA_PKCS1_SHA256.to_le_bytes().to_vec();
    sig.extend(prefixed(&key.sign(&data)));
    let signer = [
        prefixed(&data),
        prefixed(&prefixed(&sig)),
        prefixed(&key.public_key_der()),
    ]
    .concat();
    let value = prefixed(&prefixed(&signer));

    let mut pair = (value.len() as u64 + 4).to_le_bytes().to_vec();
    pair.extend_from_slice(&V2_BLOCK_ID.to_le_bytes());
    pair.extend(value);
    let size = (pair.len() + 8 + MAGIC.len()) as u64;
    let mut block = size.to_le_bytes().to_vec();
    block.extend(pair);
    block.extend_from_slice(&size.to_le_bytes());
    block.extend_from_slice(MAGIC);
    block
}

/// The signed APK: the zip with a v2 signing block.
#[must_use]
pub fn sign(zip: &Written, key: &RsaKey) -> Vec<u8> {
    let digest = content_digest(&[&zip.entries, &zip.central, &zip.end]);
    let block = signing_block(&digest, key);
    let count = u16::from_le_bytes([zip.end[10], zip.end[11]]) as usize;
    let end = end_record(count, zip.central.len(), zip.entries.len() + block.len());
    [&zip.entries[..], &block, &zip.central, &end].concat()
}

/// Checks an APK's v2 signature against `key`, the way Android does:
/// finds the block before the central directory, checks the signature
/// over the signed data and the content digest. For tests and `check`.
///
/// # Errors
///
/// What doesn't match.
pub fn verify(apk: &[u8], key: &RsaKey) -> Result<(), String> {
    let end_at = apk.len() - 22;
    let end = &apk[end_at..];
    let central_at = u32::from_le_bytes([end[16], end[17], end[18], end[19]]) as usize;
    if apk.get(central_at - 16..central_at) != Some(&MAGIC[..]) {
        return Err("no APK Signing Block".into());
    }
    let mut size = [0u8; 8];
    size.copy_from_slice(&apk[central_at - 24..central_at - 16]);
    let block_at = central_at - super::to_usize(u64::from_le_bytes(size)) - 8;
    let block = &apk[block_at..central_at];
    let count = u16::from_le_bytes([end[10], end[11]]) as usize;
    let unsigned_end = end_record(count, end_at - central_at, block_at);
    let digest = content_digest(&[&apk[..block_at], &apk[central_at..end_at], &unsigned_end]);
    let expected = signing_block(&digest, key);
    if block != &expected[..] {
        return Err("the signing block doesn't match the content".into());
    }
    // The block holds the signed data; check its signature too.
    let data = signed_data(&digest, &key.certificate());
    let sig_at = block
        .windows(data.len())
        .position(|w| w == &data[..])
        .ok_or("no signed data in the block")?
        + data.len()
        + 16; // sequence, signature and byte lengths, and the algorithm
    let signature = &block[sig_at..sig_at + key.size()];
    if key.verify(&data, signature) {
        Ok(())
    } else {
        Err("bad signature".into())
    }
}

/// The SHA-256 of the signing certificate, as `apksigner` and Play show it
/// (colon-separated hex).
#[must_use]
pub fn certificate_fingerprint(key: &RsaKey) -> String {
    sha256(&key.certificate())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
