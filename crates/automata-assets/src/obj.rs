//! Wavefront OBJ with MTL materials.
//!
//! Supported: `v` / `vt` / `vn`, polygon faces (fan-triangulated) in every
//! `v`, `v/vt`, `v//vn`, `v/vt/vn` form with negative (relative) indices,
//! `usemtl` groups, and `mtllib` files with `Kd` (diffuse color), `d` / `Tr`
//! (opacity) and `map_Kd` (color texture). Faces without normals get flat
//! normals. Texture V is flipped (OBJ's origin is bottom-left).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use automata_renderer::{MeshData, Vertex};
use glam::{Vec2, Vec3};

use crate::{unweld_flat, Image, Loaded, ModelPart};

#[derive(Debug, Clone)]
struct Mtl {
    color: [f32; 4],
    texture: Option<PathBuf>,
}

impl Default for Mtl {
    fn default() -> Self {
        Self {
            color: [1.0; 4],
            texture: None,
        }
    }
}

fn parse_floats<const N: usize>(rest: &str, default: f32) -> [f32; N] {
    let mut out = [default; N];
    for (slot, v) in out.iter_mut().zip(rest.split_whitespace()) {
        *slot = v.parse().unwrap_or(default);
    }
    out
}

/// The file name at the end of a `map_Kd` line (options like `-s 1 1 1`
/// come first; names may contain spaces).
fn map_file(rest: &str) -> &str {
    let mut tokens: Vec<&str> = rest.split_whitespace().collect();
    // Drop leading `-option value…` groups.
    while tokens.first().is_some_and(|t| t.starts_with('-')) {
        let takes = match tokens[0] {
            "-o" | "-s" | "-t" => 4,
            "-mm" => 3,
            _ => 2,
        };
        // Option values are numbers (or on/off); stop at the first token
        // that isn't, which starts the file name.
        let mut n = 1;
        while n < takes
            && tokens
                .get(n)
                .is_some_and(|t| t.parse::<f32>().is_ok() || matches!(*t, "on" | "off"))
        {
            n += 1;
        }
        tokens.drain(..n.min(tokens.len()));
    }
    let first = tokens.first().copied().unwrap_or("");
    rest.find(first).map_or("", |at| rest[at..].trim())
}

fn parse_mtl(text: &str, dir: &Path) -> HashMap<String, Mtl> {
    let mut out = HashMap::new();
    let mut name: Option<String> = None;
    let mut cur = Mtl::default();
    for line in text.lines() {
        let line = line.trim();
        let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match key {
            "newmtl" => {
                if let Some(n) = name.take() {
                    out.insert(n, std::mem::take(&mut cur));
                }
                name = Some(rest.to_string());
            }
            "Kd" => {
                let [r, g, b] = parse_floats::<3>(rest, 1.0);
                cur.color = [r, g, b, cur.color[3]];
            }
            "d" => cur.color[3] = parse_floats::<1>(rest, 1.0)[0],
            "Tr" => cur.color[3] = 1.0 - parse_floats::<1>(rest, 0.0)[0],
            "map_Kd" => {
                let file = map_file(rest);
                if !file.is_empty() {
                    cur.texture = Some(crate::join_relative(dir, file));
                }
            }
            _ => {}
        }
    }
    if let Some(n) = name {
        out.insert(n, cur);
    }
    out
}

/// The `.mtl` files and textures an OBJ uses.
pub(crate) fn references(path: &Path, bytes: &[u8]) -> Vec<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    for lib in mtllibs(&text) {
        let file = crate::join_relative(dir, &lib);
        if let Ok(mtl) = std::fs::read_to_string(&file) {
            let lib_dir = file.parent().unwrap_or(dir).to_path_buf();
            for m in parse_mtl(&mtl, &lib_dir).into_values() {
                if let Some(t) = m.texture {
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
            }
        }
        if !out.contains(&file) {
            out.insert(0, file);
        }
    }
    out
}

fn mtllibs(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("mtllib"))
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map(|rest| rest.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Geometry gathered for one `usemtl` material.
#[derive(Default)]
struct Group {
    vertices: Vec<Vertex>,
    /// Corners with normals, deduplicated by `(v, vt, vn)`.
    smooth: Vec<u32>,
    /// Corners of faces without normals (normals are generated).
    flat_vertices: Vec<Vertex>,
    /// Triangles over `flat_vertices`.
    flat: Vec<u32>,
    lookup: HashMap<(usize, usize, usize), u32>,
}

fn index(token: &str, len: usize) -> Option<usize> {
    let i: i64 = token.parse().ok()?;
    let len = i64::try_from(len).ok()?;
    let resolved = if i < 0 { len + i } else { i - 1 };
    usize::try_from(resolved)
        .ok()
        .filter(|&r| i64::try_from(r).is_ok_and(|r| r < len))
}

#[allow(clippy::too_many_lines)]
pub(crate) fn load(path: &Path, bytes: &[u8]) -> Result<Loaded, String> {
    let text = String::from_utf8_lossy(bytes);
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut deps = Vec::new();
    let mut materials: HashMap<String, Mtl> = HashMap::new();
    for lib in mtllibs(&text) {
        let file = crate::join_relative(&dir, &lib);
        match std::fs::read_to_string(&file) {
            Ok(mtl) => {
                let lib_dir = file.parent().unwrap_or(&dir).to_path_buf();
                materials.extend(parse_mtl(&mtl, &lib_dir));
                deps.push(file);
            }
            Err(e) => tracing::warn!("material library {} not loaded: {e}", file.display()),
        }
    }

    let mut positions: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut current = String::from("Default");

    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match key {
            "v" => positions.push(Vec3::from(parse_floats::<3>(rest, 0.0))),
            "vt" => {
                let [u, v] = parse_floats::<2>(rest, 0.0);
                uvs.push(Vec2::new(u, 1.0 - v));
            }
            "vn" => normals.push(Vec3::from(parse_floats::<3>(rest, 0.0)).normalize_or(Vec3::Y)),
            "usemtl" => current = rest.trim().to_string(),
            "f" => {
                let mut corners: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
                for token in rest.split_whitespace() {
                    let mut it = token.split('/');
                    let v = it
                        .next()
                        .and_then(|t| index(t, positions.len()))
                        .ok_or(format!("line {}: bad face index {token:?}", line_no + 1))?;
                    let vt = it
                        .next()
                        .filter(|t| !t.is_empty())
                        .and_then(|t| index(t, uvs.len()));
                    let vn = it
                        .next()
                        .filter(|t| !t.is_empty())
                        .and_then(|t| index(t, normals.len()));
                    corners.push((v, vt, vn));
                }
                if corners.len() < 3 {
                    continue;
                }
                if !groups.contains_key(&current) {
                    order.push(current.clone());
                }
                let group = groups.entry(current.clone()).or_default();
                let has_normals = corners.iter().all(|c| c.2.is_some());
                let mut ids = Vec::with_capacity(corners.len());
                for &(v, vt, vn) in &corners {
                    let uv = vt.map_or([0.0, 0.0], |t| uvs[t].into());
                    if has_normals {
                        let key = (v, vt.unwrap_or(usize::MAX), vn.unwrap_or(0));
                        let next =
                            u32::try_from(group.vertices.len()).map_err(|_| "too many vertices")?;
                        let id = *group.lookup.entry(key).or_insert_with(|| {
                            group.vertices.push(Vertex::new(
                                positions[v].into(),
                                normals[vn.unwrap_or(0)].into(),
                                uv,
                            ));
                            next
                        });
                        ids.push(id);
                    } else {
                        let id = u32::try_from(group.flat_vertices.len())
                            .map_err(|_| "too many vertices")?;
                        group.flat_vertices.push(Vertex::new(
                            positions[v].into(),
                            [0.0, 1.0, 0.0],
                            uv,
                        ));
                        ids.push(id);
                    }
                }
                let target = if has_normals {
                    &mut group.smooth
                } else {
                    &mut group.flat
                };
                for i in 1..ids.len() - 1 {
                    target.extend([ids[0], ids[i], ids[i + 1]]);
                }
            }
            _ => {}
        }
    }

    let mut textures: HashMap<PathBuf, Option<Arc<Image>>> = HashMap::new();
    let mut parts = Vec::new();
    for name in order {
        let Some(group) = groups.remove(&name) else {
            continue;
        };
        let mtl = materials.get(&name).cloned().unwrap_or_default();
        let texture = mtl.texture.as_ref().and_then(|file| {
            textures
                .entry(file.clone())
                .or_insert_with(|| {
                    let img = std::fs::read(file)
                        .map_err(|e| e.to_string())
                        .and_then(|b| Image::decode(&b));
                    match img {
                        Ok(img) => {
                            deps.push(file.clone());
                            Some(Arc::new(img))
                        }
                        Err(e) => {
                            tracing::warn!("texture {} not loaded: {e}", file.display());
                            None
                        }
                    }
                })
                .clone()
        });
        let mut mesh = MeshData::new(group.vertices, group.smooth);
        if !group.flat.is_empty() {
            let flat = unweld_flat(&group.flat_vertices, &group.flat);
            let base = u32::try_from(mesh.vertices.len()).map_err(|_| "too many vertices")?;
            mesh.vertices.extend(flat.vertices);
            mesh.indices.extend(flat.indices.iter().map(|i| i + base));
        }
        parts.push(ModelPart {
            pixelated: false,
            name,
            mesh,
            color: mtl.color,
            texture,
            skin: None,
            surface: crate::PartSurface::default(),
        });
    }
    Ok(Loaded {
        parts,
        rig: None,
        deps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE_FACE: &str = "\
# a square and a triangle in two materials
mtllib parts.mtl
v -1 -1 0
v 1 -1 0
v 1 1 0
v -1 1 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vn 0 0 1
usemtl Green
f 1/1/1 2/2/1 3/3/1 4/4/1
usemtl Blue
f -4 -3 -2
";

    #[test]
    fn faces_triangulate_into_one_part_per_material() {
        let loaded = load(Path::new("/nowhere/x.obj"), CUBE_FACE.as_bytes()).unwrap();
        assert_eq!(loaded.parts.len(), 2);
        let green = &loaded.parts[0];
        assert_eq!(green.name, "Green");
        assert_eq!(green.mesh.indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(green.mesh.vertices.len(), 4);
        // V is flipped: vt 0 0 (bottom-left) becomes uv (0, 1).
        assert_eq!(green.mesh.vertices[0].uv, [0.0, 1.0]);
        let blue = &loaded.parts[1];
        assert_eq!(blue.mesh.indices.len(), 3);
        assert_eq!(blue.mesh.vertices.len(), 3);
        assert!(blue
            .mesh
            .vertices
            .iter()
            .all(|v| (Vec3::from(v.normal) - Vec3::Z).length() < 1e-6));
    }

    #[test]
    fn mtl_colors_and_textures_are_read() {
        let dir = std::env::temp_dir().join(format!("automata-obj-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("tex")).unwrap();
        std::fs::write(
            dir.join("parts.mtl"),
            "newmtl Green\nKd 0 1 0\nd 0.5\nmap_Kd -s 2 2 1 tex/my grass.png\nnewmtl Blue\nKd 0 0 1\n",
        )
        .unwrap();
        // A 1x1 PNG so the texture decodes.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.join("tex/my grass.png"), png).unwrap();
        let obj = dir.join("x.obj");
        std::fs::write(&obj, CUBE_FACE).unwrap();

        let loaded = load(&obj, CUBE_FACE.as_bytes()).unwrap();
        assert_eq!(loaded.parts[0].color, [0.0, 1.0, 0.0, 0.5]);
        let tex = loaded.parts[0].texture.as_ref().expect("texture loaded");
        assert_eq!((tex.width, tex.height), (1, 1));
        assert_eq!(loaded.parts[1].color, [0.0, 0.0, 1.0, 1.0]);
        assert!(loaded.parts[1].texture.is_none());
        assert_eq!(
            references(&obj, CUBE_FACE.as_bytes()),
            [dir.join("parts.mtl"), dir.join("tex/my grass.png")]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn faces_without_normals_get_flat_ones() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 0 -1\nf 1 2 3\n";
        let loaded = load(Path::new("t.obj"), obj.as_bytes()).unwrap();
        let mesh = &loaded.parts[0].mesh;
        assert_eq!(mesh.indices.len(), 3);
        for v in &mesh.vertices {
            assert!(
                (Vec3::from(v.normal) - Vec3::Y).length() < 1e-5,
                "{:?}",
                v.normal
            );
        }
    }

    #[test]
    fn bad_indices_are_reported_with_the_line() {
        let err = load(Path::new("b.obj"), b"v 0 0 0\nf 1 2 3\n")
            .err()
            .unwrap();
        assert!(err.contains("line 2"), "{err}");
    }

    #[test]
    fn map_options_are_skipped() {
        assert_eq!(map_file("-s 1 1 1 -bm 0.5 wood.png"), "wood.png");
        assert_eq!(map_file("my tex.png"), "my tex.png");
        assert_eq!(map_file("-clamp on a.jpg"), "a.jpg");
    }
}
