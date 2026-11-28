//! Editing the template's compiled `AndroidManifest.xml` (Android's binary
//! XML) and `resources.arsc`, so each game gets its own app ID, name and
//! version without the Android SDK.
//!
//! Binary XML is a string pool followed by chunks; elements point at
//! strings by index. The edit rewrites the pool (new strings go at the
//! end, so every existing index stays valid) and changes attribute values
//! in place, which keeps every other chunk's size.

/// What a game changes in the template's manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestPatch {
    /// The app ID (`com.example.game`).
    pub package: String,
    /// The name under the icon.
    pub label: String,
    /// The version Android compares for updates.
    pub version_code: u32,
    /// The version people see.
    pub version_name: String,
}

use super::{low_byte, n16, n32};

const RES_XML: u16 = 0x0003;
const RES_STRING_POOL: u16 = 0x0001;
const RES_XML_RESOURCE_MAP: u16 = 0x0180;
const RES_XML_START_ELEMENT: u16 = 0x0102;
const UTF8_FLAG: u32 = 0x100;
const ATTR_LABEL: u32 = 0x0101_0001;
const ATTR_NAME: u32 = 0x0101_0003;
const ATTR_PERMISSION: u32 = 0x0101_0006;
const ATTR_AUTHORITIES: u32 = 0x0101_0018;
const ATTR_VERSION_CODE: u32 = 0x0101_021b;
const ATTR_VERSION_NAME: u32 = 0x0101_021c;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;

fn u16_at(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "truncated binary XML".to_string())
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated binary XML".to_string())
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// A decoded string pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPool {
    /// The strings, by index.
    pub strings: Vec<String>,
    /// Whether they're stored as UTF-8 (else UTF-16).
    pub utf8: bool,
    /// The pool's flags.
    flags: u32,
}

impl StringPool {
    /// Reads the pool chunk at the start of `b`; returns it and its size.
    ///
    /// # Errors
    ///
    /// When the chunk isn't a string pool, has styles, or is damaged.
    pub fn read(b: &[u8]) -> Result<(Self, usize), String> {
        if u16_at(b, 0)? != RES_STRING_POOL {
            return Err("expected a string pool".into());
        }
        let size = u32_at(b, 4)? as usize;
        let count = u32_at(b, 8)? as usize;
        let styles = u32_at(b, 12)?;
        let flags = u32_at(b, 16)?;
        let start = u32_at(b, 20)? as usize;
        if styles != 0 {
            return Err("styled strings in the manifest aren't supported".into());
        }
        let header = u16_at(b, 2)? as usize;
        let utf8 = flags & UTF8_FLAG != 0;
        let mut strings = Vec::with_capacity(count);
        for i in 0..count {
            let at = start + u32_at(b, header + i * 4)? as usize;
            strings.push(if utf8 {
                read_utf8(b, at)?
            } else {
                read_utf16(b, at)?
            });
        }
        Ok((
            Self {
                strings,
                utf8,
                flags,
            },
            size,
        ))
    }

    /// The pool as a chunk.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut data = Vec::new();
        let mut offsets = Vec::with_capacity(self.strings.len());
        for s in &self.strings {
            offsets.push(n32(data.len()));
            if self.utf8 {
                write_utf8(&mut data, s);
            } else {
                write_utf16(&mut data, s);
            }
        }
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let start = 28 + 4 * self.strings.len();
        let mut out = Vec::with_capacity(start + data.len());
        out.extend_from_slice(&RES_STRING_POOL.to_le_bytes());
        out.extend_from_slice(&28u16.to_le_bytes());
        out.extend_from_slice(&n32(start + data.len()).to_le_bytes());
        out.extend_from_slice(&n32(self.strings.len()).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        // Not sorted any more: new strings go at the end.
        out.extend_from_slice(&(self.flags & UTF8_FLAG).to_le_bytes());
        out.extend_from_slice(&n32(start).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for o in offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend(data);
        out
    }

    /// The index of `s`, added at the end when it isn't there.
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(i) = self.strings.iter().position(|x| x == s) {
            return n32(i);
        }
        self.strings.push(s.to_string());
        n32(self.strings.len() - 1)
    }
}

fn read_len8(b: &[u8], at: &mut usize) -> Result<usize, String> {
    let first = *b.get(*at).ok_or("truncated string")? as usize;
    *at += 1;
    if first & 0x80 == 0 {
        return Ok(first);
    }
    let second = *b.get(*at).ok_or("truncated string")? as usize;
    *at += 1;
    Ok(((first & 0x7f) << 8) | second)
}

fn read_utf8(b: &[u8], mut at: usize) -> Result<String, String> {
    read_len8(b, &mut at)?; // UTF-16 length
    let len = read_len8(b, &mut at)?;
    let bytes = b.get(at..at + len).ok_or("truncated string")?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn read_utf16(b: &[u8], mut at: usize) -> Result<String, String> {
    let mut len = u16_at(b, at)? as usize;
    at += 2;
    if len & 0x8000 != 0 {
        len = ((len & 0x7fff) << 16) | u16_at(b, at)? as usize;
        at += 2;
    }
    let units = (0..len)
        .map(|i| u16_at(b, at + i * 2))
        .collect::<Result<Vec<u16>, String>>()?;
    Ok(String::from_utf16_lossy(&units))
}

fn write_len8(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(low_byte(len));
    } else {
        out.push(0x80 | low_byte(len >> 8));
        out.push(low_byte(len));
    }
}

fn write_utf8(out: &mut Vec<u8>, s: &str) {
    write_len8(out, s.encode_utf16().count());
    write_len8(out, s.len());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

fn write_utf16(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() > 0x7fff {
        out.extend_from_slice(&n16(0x8000 | (units.len() >> 16)).to_le_bytes());
    }
    out.extend_from_slice(&n16(units.len() & 0xffff).to_le_bytes());
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
}

/// Where an attribute's 20 bytes start, with its resource ID (0 for none),
/// its name and its element's name (string indices).
struct Attr {
    at: usize,
    id: u32,
    name: u32,
    element: u32,
}

/// Every element attribute in the chunks after the pool.
fn attributes(rest: &[u8]) -> Result<Vec<Attr>, String> {
    let mut ids: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= rest.len() {
        let kind = u16_at(rest, at)?;
        let header = u16_at(rest, at + 2)? as usize;
        let size = u32_at(rest, at + 4)? as usize;
        if size < 8 || at + size > rest.len() {
            return Err("damaged binary XML chunk".into());
        }
        if kind == RES_XML_RESOURCE_MAP {
            ids = (at + header..at + size)
                .step_by(4)
                .map(|i| u32_at(rest, i))
                .collect::<Result<_, _>>()?;
        } else if kind == RES_XML_START_ELEMENT {
            let ext = at + header;
            let element = u32_at(rest, ext + 4)?;
            let start = u16_at(rest, ext + 8)? as usize;
            let step = u16_at(rest, ext + 10)? as usize;
            let count = u16_at(rest, ext + 12)? as usize;
            for i in 0..count {
                let a = ext + start + i * step;
                let name = u32_at(rest, a + 4)?;
                let id = ids.get(name as usize).copied().unwrap_or(0);
                out.push(Attr {
                    at: a,
                    id,
                    name,
                    element,
                });
            }
        }
        at += size;
    }
    Ok(out)
}

/// Whether `a` names something the app owns rather than a class: a
/// permission (declared or required) or a provider's authorities.
fn owned_name(pool: &StringPool, a: &Attr) -> bool {
    let element = pool
        .strings
        .get(a.element as usize)
        .map_or("", String::as_str);
    match a.id {
        ATTR_AUTHORITIES | ATTR_PERMISSION => true,
        ATTR_NAME => matches!(
            element,
            "permission" | "uses-permission" | "permission-tree" | "permission-group"
        ),
        _ => false,
    }
}

fn set_value(rest: &mut [u8], at: usize, raw: u32, kind: u8, data: u32) {
    put_u32(rest, at + 8, raw);
    rest[at + 12..at + 16].copy_from_slice(&[8, 0, 0, kind]);
    put_u32(rest, at + 16, data);
}

/// Applies `patch` to a binary manifest. Returns the new manifest and the
/// template's package name.
///
/// # Errors
///
/// When the file isn't binary XML this understands or has no package.
pub fn patch_manifest(xml: &[u8], patch: &ManifestPatch) -> Result<(Vec<u8>, String), String> {
    if u16_at(xml, 0)? != RES_XML {
        return Err("AndroidManifest.xml isn't binary XML".into());
    }
    let header = u16_at(xml, 2)? as usize;
    let (mut pool, pool_size) = StringPool::read(&xml[header..])?;
    let mut rest = xml[header + pool_size..].to_vec();
    let attrs = attributes(&rest)?;
    let package_attr = attrs
        .iter()
        .find(|a| {
            a.id == 0
                && pool
                    .strings
                    .get(a.name as usize)
                    .is_some_and(|n| n == "package")
        })
        .ok_or("the manifest has no package")?;
    let old_index = u32_at(&rest, package_attr.at + 8)? as usize;
    let old = pool
        .strings
        .get(old_index)
        .cloned()
        .ok_or("damaged package attribute")?;

    let label = pool.intern(&patch.label);
    let version_name = pool.intern(&patch.version_name);
    let package = pool.intern(&patch.package);
    set_value(&mut rest, package_attr.at, package, TYPE_STRING, package);
    let dotted = format!("{old}.");
    for a in &attrs {
        match a.id {
            ATTR_LABEL => set_value(&mut rest, a.at, label, TYPE_STRING, label),
            ATTR_VERSION_NAME => {
                set_value(&mut rest, a.at, version_name, TYPE_STRING, version_name);
            }
            ATTR_VERSION_CODE => {
                set_value(&mut rest, a.at, u32::MAX, TYPE_INT_DEC, patch.version_code);
            }
            _ if owned_name(&pool, a) => {
                // Permissions and authorities named after the package must
                // be the game's own (two apps can't both declare one).
                // Class names stay: the code doesn't move.
                let raw = u32_at(&rest, a.at + 8)? as usize;
                let renamed = pool
                    .strings
                    .get(raw)
                    .and_then(|v| v.strip_prefix(&dotted))
                    .map(|tail| format!("{}.{tail}", patch.package));
                if let Some(renamed) = renamed {
                    let i = pool.intern(&renamed);
                    set_value(&mut rest, a.at, i, TYPE_STRING, i);
                }
            }
            _ => {}
        }
    }

    let new_pool = pool.write();
    let mut out = xml[..header].to_vec();
    out.extend(new_pool);
    out.extend(rest);
    let len = n32(out.len());
    put_u32(&mut out, 4, len);
    Ok((out, old))
}

/// Renames the app's package in `resources.arsc` (a fixed 128-character
/// field), which Android matches against the app ID when code looks
/// resources up by name.
///
/// # Errors
///
/// When the table is damaged, has no app package, or the name is too long.
pub fn rename_resource_package(arsc: &mut [u8], package: &str) -> Result<(), String> {
    const RES_TABLE: u16 = 0x0002;
    const RES_TABLE_PACKAGE: u16 = 0x0200;
    if u16_at(arsc, 0)? != RES_TABLE {
        return Err("resources.arsc isn't a resource table".into());
    }
    let units: Vec<u16> = package.encode_utf16().collect();
    if units.len() >= 128 {
        return Err("the Android package name is too long".into());
    }
    let mut at = u16_at(arsc, 2)? as usize;
    let mut renamed = false;
    while at + 8 <= arsc.len() {
        let kind = u16_at(arsc, at)?;
        let size = u32_at(arsc, at + 4)? as usize;
        if size < 8 {
            return Err("damaged resources.arsc".into());
        }
        if kind == RES_TABLE_PACKAGE && u32_at(arsc, at + 8)? == 0x7f {
            let field = arsc
                .get_mut(at + 12..at + 12 + 256)
                .ok_or("damaged resources.arsc")?;
            field.fill(0);
            for (i, u) in units.iter().enumerate() {
                field[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
            renamed = true;
        }
        at += size;
    }
    if renamed {
        Ok(())
    } else {
        Err("resources.arsc has no app package".into())
    }
}

/// The template's app package name in `resources.arsc`.
///
/// # Errors
///
/// When the table is damaged or has no app package.
pub fn resource_package(arsc: &[u8]) -> Result<String, String> {
    let mut at = u16_at(arsc, 2)? as usize;
    while at + 8 <= arsc.len() {
        let size = u32_at(arsc, at + 4)? as usize;
        if size < 8 {
            break;
        }
        if u16_at(arsc, at)? == 0x0200 && u32_at(arsc, at + 8)? == 0x7f {
            let units = (0..128)
                .map(|i| u16_at(arsc, at + 12 + i * 2))
                .collect::<Result<Vec<u16>, String>>()?;
            let end = units.iter().position(|u| *u == 0).unwrap_or(128);
            return Ok(String::from_utf16_lossy(&units[..end]));
        }
        at += size;
    }
    Err("resources.arsc has no app package".into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn chunk(kind: u16, header: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = kind.to_le_bytes().to_vec();
        out.extend_from_slice(&n16(8 + header.len()).to_le_bytes());
        out.extend_from_slice(&n32(8 + header.len() + body.len()).to_le_bytes());
        out.extend_from_slice(header);
        out.extend_from_slice(body);
        out
    }

    fn attr(name: u32, raw: u32, kind: u8, data: u32) -> Vec<u8> {
        let mut a = u32::MAX.to_le_bytes().to_vec();
        a.extend_from_slice(&name.to_le_bytes());
        a.extend_from_slice(&raw.to_le_bytes());
        a.extend_from_slice(&[8, 0, 0, kind]);
        a.extend_from_slice(&data.to_le_bytes());
        a
    }

    fn element(name: u32, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut header = 1u32.to_le_bytes().to_vec(); // line
        header.extend_from_slice(&u32::MAX.to_le_bytes()); // comment
        let mut body = u32::MAX.to_le_bytes().to_vec(); // namespace
        body.extend_from_slice(&name.to_le_bytes());
        for v in [20u16, 20, n16(attrs.len()), 0, 0, 0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for a in attrs {
            body.extend_from_slice(a);
        }
        chunk(RES_XML_START_ELEMENT, &header, &body)
    }

    /// A small manifest like aapt2's: label, versionCode and versionName
    /// with resource IDs, `package`, and a permission named after it.
    pub(crate) fn sample_manifest(utf8: bool) -> Vec<u8> {
        let pool = StringPool {
            strings: [
                "label",
                "versionCode",
                "versionName",
                "name",
                "package",
                "manifest",
                "org.automata.template",
                "0.1.0",
                "application",
                "org.automata.template.PERMISSION",
                "org.automata.AutomataActivity",
                "uses-permission",
                "org.automata.template.Service",
            ]
            .map(String::from)
            .to_vec(),
            utf8,
            flags: if utf8 { UTF8_FLAG } else { 0 },
        };
        let mut ids = Vec::new();
        for id in [
            ATTR_LABEL,
            ATTR_VERSION_CODE,
            ATTR_VERSION_NAME,
            0x0101_0003,
        ] {
            ids.extend_from_slice(&id.to_le_bytes());
        }
        let manifest = element(
            5,
            &[
                attr(1, u32::MAX, TYPE_INT_DEC, 7),
                attr(2, 7, TYPE_STRING, 7),
                attr(4, 6, TYPE_STRING, 6),
            ],
        );
        let app = element(
            8,
            &[
                attr(0, u32::MAX, 0x01, 0x7f05_0000),
                attr(3, 12, TYPE_STRING, 12),
            ],
        );
        let permission = element(11, &[attr(3, 9, TYPE_STRING, 9)]);
        let body = [
            pool.write(),
            chunk(RES_XML_RESOURCE_MAP, &[], &ids),
            manifest,
            app,
            permission,
        ]
        .concat();
        chunk(RES_XML, &[], &body)
    }

    /// A resource table with one app package called `name`.
    pub(crate) fn sample_arsc(name: &str) -> Vec<u8> {
        let mut header = 0x7fu32.to_le_bytes().to_vec();
        let mut field = [0u8; 256];
        for (i, u) in name.encode_utf16().enumerate() {
            field[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        header.extend_from_slice(&field);
        let package = chunk(0x0200, &header, &[1, 2, 3, 4]);
        chunk(0x0002, &1u32.to_le_bytes(), &package)
    }

    fn patch() -> ManifestPatch {
        ManifestPatch {
            package: "com.example.coinrun".into(),
            label: "Coin Run".into(),
            version_code: 42,
            version_name: "1.2.3".into(),
        }
    }

    #[test]
    fn pools_round_trip_in_both_encodings() {
        for utf8 in [true, false] {
            let pool = StringPool {
                strings: vec!["a".into(), "Persian فارسی".into(), "x".repeat(300)],
                utf8,
                flags: if utf8 { UTF8_FLAG } else { 0 },
            };
            let (back, size) = StringPool::read(&pool.write()).unwrap();
            assert_eq!(back, pool);
            assert_eq!(size, pool.write().len());
            assert_eq!(size % 4, 0);
        }
    }

    #[test]
    fn the_manifest_gets_the_games_package_name_and_version() {
        for utf8 in [true, false] {
            let (out, old) = patch_manifest(&sample_manifest(utf8), &patch()).unwrap();
            assert_eq!(old, "org.automata.template");
            assert_eq!(u32_at(&out, 4).unwrap() as usize, out.len());
            let (pool, size) = StringPool::read(&out[8..]).unwrap();
            let rest = &out[8 + size..];
            let attrs = attributes(rest).unwrap();
            let value_of = |a: &Attr| (rest[a.at + 15], u32_at(rest, a.at + 16).unwrap());
            let value = |id: u32| value_of(attrs.iter().find(|a| a.id == id).unwrap());
            let named = |element: &str| {
                let a = attrs
                    .iter()
                    .find(|a| pool.strings[a.element as usize] == element && a.id == ATTR_NAME)
                    .unwrap();
                pool.strings[value_of(a).1 as usize].clone()
            };
            let package = attrs.iter().find(|a| a.id == 0).unwrap();
            assert_eq!(
                pool.strings[value_of(package).1 as usize],
                "com.example.coinrun"
            );
            assert_eq!(named("uses-permission"), "com.example.coinrun.PERMISSION");
            assert_eq!(
                named("application"),
                "org.automata.template.Service",
                "classes stay"
            );
            assert_eq!(value(ATTR_VERSION_CODE), (TYPE_INT_DEC, 42));
            let (kind, label) = value(ATTR_LABEL);
            assert_eq!(kind, TYPE_STRING);
            assert_eq!(pool.strings[label as usize], "Coin Run");
            let (_, name) = value(ATTR_VERSION_NAME);
            assert_eq!(pool.strings[name as usize], "1.2.3");
        }
    }

    #[test]
    fn the_resource_table_package_is_renamed() {
        let mut arsc = sample_arsc("org.automata.template");
        assert_eq!(resource_package(&arsc).unwrap(), "org.automata.template");
        rename_resource_package(&mut arsc, "com.example.coinrun").unwrap();
        assert_eq!(resource_package(&arsc).unwrap(), "com.example.coinrun");
        assert!(rename_resource_package(&mut arsc, &"a".repeat(130)).is_err());
        assert!(patch_manifest(b"nope", &patch()).is_err());
    }
}
