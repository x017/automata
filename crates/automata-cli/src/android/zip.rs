//! Reading an APK's entries and writing a new, aligned APK.
//!
//! An APK is a zip file. Entries copied from the template keep their
//! compressed bytes as they are; stored (uncompressed) entries are aligned
//! the way `zipalign` aligns them, since Android maps some of them straight
//! from the file: 4 bytes in general (`resources.arsc` must be), and 16 KB
//! pages for native libraries.

use std::io::{Read, Write};

use super::{n16, n32};

/// One file in a zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path inside the zip (`/`-separated).
    pub name: String,
    /// 0 (stored) or 8 (deflated).
    pub method: u16,
    /// CRC-32 of the uncompressed bytes.
    pub crc: u32,
    /// The uncompressed size.
    pub size: u32,
    /// The bytes as stored in the zip (compressed when `method` is 8).
    pub data: Vec<u8>,
}

impl Entry {
    /// A new entry for `bytes`, deflated unless that doesn't make it
    /// smaller.
    #[must_use]
    pub fn new(name: &str, bytes: &[u8]) -> Self {
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        let deflated = enc
            .write_all(bytes)
            .and_then(|()| enc.finish())
            .ok()
            .filter(|d| d.len() + 16 < bytes.len());
        let (method, data) = match deflated {
            Some(d) => (8, d),
            None => (0, bytes.to_vec()),
        };
        Self {
            name: name.to_string(),
            method,
            crc: crc32fast::hash(bytes),
            size: super::n32(bytes.len()),
            data,
        }
    }

    /// The uncompressed bytes.
    ///
    /// # Errors
    ///
    /// When the entry uses another compression or its data is damaged.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        let out = match self.method {
            0 => self.data.clone(),
            8 => {
                let mut out = Vec::with_capacity(self.size as usize);
                flate2::read::DeflateDecoder::new(&self.data[..])
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{}: {e}", self.name))?;
                out
            }
            m => return Err(format!("{}: unsupported compression {m}", self.name)),
        };
        if crc32fast::hash(&out) != self.crc {
            return Err(format!("{}: damaged (CRC mismatch)", self.name));
        }
        Ok(out)
    }
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "truncated zip".to_string())
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated zip".to_string())
}

/// Where the end of central directory record starts.
fn find_eocd(zip: &[u8]) -> Result<usize, String> {
    let lowest = zip.len().saturating_sub(22 + 0xffff);
    (lowest..=zip.len().saturating_sub(22))
        .rev()
        .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| "not a zip file (no end of central directory)".to_string())
}

/// Reads every entry of `zip`.
///
/// # Errors
///
/// When it isn't a zip this reader handles (ZIP64, multi-disk, damaged).
pub fn read(zip: &[u8]) -> Result<Vec<Entry>, String> {
    let eocd = find_eocd(zip)?;
    let count = u16_at(zip, eocd + 10)? as usize;
    let mut at = u32_at(zip, eocd + 16)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(zip, at)? != 0x0201_4b50 {
            return Err("damaged central directory".into());
        }
        let method = u16_at(zip, at + 10)?;
        let crc = u32_at(zip, at + 16)?;
        let csize = u32_at(zip, at + 20)?;
        let size = u32_at(zip, at + 24)?;
        let name_len = u16_at(zip, at + 28)? as usize;
        let extra_len = u16_at(zip, at + 30)? as usize;
        let comment_len = u16_at(zip, at + 32)? as usize;
        let local = u32_at(zip, at + 42)? as usize;
        if csize == u32::MAX || size == u32::MAX || local == u32::MAX as usize {
            return Err("ZIP64 archives aren't supported".into());
        }
        let name = zip
            .get(at + 46..at + 46 + name_len)
            .ok_or("truncated zip")?;
        let name = String::from_utf8_lossy(name).into_owned();
        if u32_at(zip, local)? != 0x0403_4b50 {
            return Err(format!("{name}: damaged local header"));
        }
        let start =
            local + 30 + u16_at(zip, local + 26)? as usize + u16_at(zip, local + 28)? as usize;
        let data = zip
            .get(start..start + csize as usize)
            .ok_or_else(|| format!("{name}: truncated"))?
            .to_vec();
        entries.push(Entry {
            name,
            method,
            crc,
            size,
            data,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// A written zip, in the three parts an APK signature covers.
#[derive(Debug, Clone, Default)]
pub struct Written {
    /// The local headers and data.
    pub entries: Vec<u8>,
    /// The central directory.
    pub central: Vec<u8>,
    /// The end of central directory record.
    pub end: Vec<u8>,
}

impl Written {
    /// The whole file (unsigned).
    #[must_use]
    pub fn concat(&self) -> Vec<u8> {
        [&self.entries[..], &self.central, &self.end].concat()
    }
}

/// The alignment of a stored entry's data.
fn alignment(name: &str) -> usize {
    if std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("so"))
    {
        16384
    } else {
        4
    }
}

/// Writes `entries` as a zip, aligning stored entries.
#[must_use]
pub fn write(entries: &[Entry]) -> Written {
    // 1980-01-01 00:00: builds of the same files are byte-for-byte equal.
    const DATE: u16 = 0x0021;
    let mut out = Written::default();
    for e in entries {
        let offset = n32(out.entries.len());
        let name = e.name.as_bytes();
        let mut extra = Vec::new();
        if e.method == 0 {
            let align = alignment(&e.name);
            let data_at = out.entries.len() + 30 + name.len();
            let mut pad = (align - data_at % align) % align;
            if pad > 0 {
                // An alignment extra field (as apksigner writes): id, size,
                // alignment, then zeros.
                while pad < 6 {
                    pad += align;
                }
                extra.extend_from_slice(&0xd935u16.to_le_bytes());
                extra.extend_from_slice(&n16(pad - 4).to_le_bytes());
                extra.extend_from_slice(&n16(align).to_le_bytes());
                extra.resize(pad, 0);
            }
        }
        let common = |v: &mut Vec<u8>| {
            v.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
            v.extend_from_slice(&e.method.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
            v.extend_from_slice(&DATE.to_le_bytes());
            v.extend_from_slice(&e.crc.to_le_bytes());
            v.extend_from_slice(&n32(e.data.len()).to_le_bytes());
            v.extend_from_slice(&e.size.to_le_bytes());
            v.extend_from_slice(&n16(name.len()).to_le_bytes());
        };
        let l = &mut out.entries;
        l.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        l.extend_from_slice(&20u16.to_le_bytes());
        common(l);
        l.extend_from_slice(&n16(extra.len()).to_le_bytes());
        l.extend_from_slice(name);
        l.extend_from_slice(&extra);
        l.extend_from_slice(&e.data);

        let c = &mut out.central;
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        common(c);
        c.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name);
    }
    out.end = end_record(entries.len(), out.central.len(), out.entries.len());
    out
}

/// The end of central directory record.
#[must_use]
pub fn end_record(count: usize, central_len: usize, central_at: usize) -> Vec<u8> {
    let mut e = Vec::with_capacity(22);
    e.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    e.extend_from_slice(&[0; 4]);
    e.extend_from_slice(&n16(count).to_le_bytes());
    e.extend_from_slice(&n16(count).to_le_bytes());
    e.extend_from_slice(&n32(central_len).to_le_bytes());
    e.extend_from_slice(&n32(central_at).to_le_bytes());
    e.extend_from_slice(&0u16.to_le_bytes());
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_and_stored_ones_are_aligned() {
        let text = b"hello hello hello hello hello hello hello hello".repeat(20);
        let entries = vec![
            Entry::new("AndroidManifest.xml", &text),
            Entry::new("a.bin", &[1, 2, 3]),
            Entry::new("lib/x86_64/libx.so", &[9; 10]),
            Entry::new("resources.arsc", &[7; 5]),
        ];
        assert_eq!(entries[0].method, 8);
        assert_eq!(entries[1].method, 0);
        let zip = write(&entries).concat();
        let back = read(&zip).unwrap();
        assert_eq!(back, entries);
        assert_eq!(back[0].bytes().unwrap(), text);
        for e in &back[1..] {
            let at = zip
                .windows(e.data.len())
                .position(|w| w == &e.data[..])
                .unwrap();
            assert_eq!(at % alignment(&e.name), 0, "{}", e.name);
        }
    }

    #[test]
    fn not_a_zip() {
        assert!(read(b"nope").is_err());
        assert!(read(&[0; 100]).is_err());
    }
}
