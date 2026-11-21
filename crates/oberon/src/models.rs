//! 3D models in the editor: importing files into the project, listing the
//! project's models, and describing a loaded model for the Inspector.
//!
//! Importing copies a model (plus the buffers, textures and `.mtl` it
//! uses) into the project's `models/` folder, so the scene keeps a short
//! relative path and exported games carry the files along. Files already
//! inside the project are used where they are.

use std::path::{Path, PathBuf};

use automata_assets::{is_model_path, Model};

/// The project folder models are imported into.
pub const MODELS_DIR: &str = "models";

/// Model files under `root` (skipping hidden, build and virtualenv
/// folders), at most `limit`, sorted by path.
#[must_use]
pub fn project_models(root: &Path, limit: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, limit: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if out.len() >= limit {
                return;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "venv" | "target" | "node_modules" | "__pycache__" | "vendor" | "build"
                )
            {
                continue;
            }
            if path.is_dir() {
                if depth < 6 {
                    walk(&path, depth + 1, out, limit);
                }
            } else if is_model_path(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out, limit);
    out
}

/// Whether `path` is inside `dir` (after resolving `..` and links where
/// possible).
fn is_inside(path: &Path, dir: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(path).starts_with(canon(dir))
}

/// `dir/name`, or `dir/stem 2.ext`, `dir/stem 3.ext`, … whichever is free.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..10_000)
        .map(|i| dir.join(format!("{stem} {i}{ext}")))
        .find(|c| !c.exists())
        .unwrap_or(first)
}

/// Copies a model into `project/models/` and returns where it landed.
///
/// A self-contained file (`.glb`, or a `.gltf` / `.obj` that uses no other
/// files) is copied as `models/<name>`. A model with companion files gets
/// its own folder, `models/<stem>/`, with the companions at the same
/// relative paths, so its internal references keep working. A model that
/// is already inside the project is returned unchanged.
///
/// # Errors
///
/// Fails when `src` isn't a model file or copying fails.
pub fn import_model(src: &Path, project: &Path) -> Result<PathBuf, String> {
    if !is_model_path(src) {
        return Err(format!(
            "{} isn't a model (use .glb, .gltf or .obj)",
            src.display()
        ));
    }
    if !src.is_file() {
        return Err(format!("{} doesn't exist", src.display()));
    }
    if is_inside(src, project) {
        return Ok(src.to_path_buf());
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("the model has no file name")?;
    let models = project.join(MODELS_DIR);
    let src_dir = src.parent().unwrap_or(Path::new("."));
    let deps: Vec<PathBuf> = automata_assets::dependencies(src)
        .into_iter()
        .filter(|d| d.is_file())
        .collect();
    let copy = |from: &Path, to: &Path| -> Result<(), String> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| format!("copying {}: {e}", from.display()))
    };
    if deps.is_empty() {
        let dest = free_name(&models, &name);
        copy(src, &dest)?;
        return Ok(dest);
    }
    let stem = Path::new(&name)
        .file_stem()
        .map_or_else(|| "model".to_string(), |s| s.to_string_lossy().into_owned());
    let folder = free_name(&models, &stem);
    let dest = folder.join(&name);
    copy(src, &dest)?;
    for dep in deps {
        // Companions keep their path relative to the model. One outside the
        // model's folder (`../textures/x.png`) can't keep it; it goes beside
        // the model, and the reference may need fixing in a modelling tool.
        let rel = dep.strip_prefix(src_dir).map_or_else(
            |_| dep.file_name().map(PathBuf::from).unwrap_or_default(),
            Path::to_path_buf,
        );
        copy(&dep, &folder.join(rel))?;
    }
    Ok(dest)
}

/// An entity name for a model file: its stem with `_` and `-` as spaces,
/// first letter capitalized (`oak_tree.glb` → `Oak tree`).
#[must_use]
pub fn entity_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map_or_else(|| "Model".to_string(), |s| s.to_string_lossy().into_owned());
    let spaced: String = stem
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let trimmed = spaced.trim();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Model".to_string(),
    }
}

/// One line for the Inspector: parts, triangles and size.
#[must_use]
pub fn describe(model: &Model) -> String {
    let s = model.size();
    let parts = model.parts.len();
    let textured = model.parts.iter().filter(|p| p.texture.is_some()).count();
    let mut line = format!(
        "{parts} part{} · {} tris · {:.2} × {:.2} × {:.2}",
        if parts == 1 { "" } else { "s" },
        group_thousands(model.triangle_count()),
        s.x,
        s.y,
        s.z
    );
    if textured > 0 {
        line.push_str(" · textured");
    }
    line
}

fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon-models-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const TRI: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    #[test]
    fn a_self_contained_model_is_copied_into_models() {
        let outside = temp("src1");
        let project = temp("proj1");
        let src = outside.join("rock.obj");
        std::fs::write(&src, TRI).unwrap();
        let dest = import_model(&src, &project).unwrap();
        assert_eq!(dest, project.join("models/rock.obj"));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), TRI);
        // A second import with the same name doesn't overwrite.
        let again = import_model(&src, &project).unwrap();
        assert_eq!(again, project.join("models/rock 2.obj"));
        // A file already in the project stays put.
        assert_eq!(import_model(&dest, &project).unwrap(), dest);
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn companions_travel_with_the_model() {
        let outside = temp("src2");
        let project = temp("proj2");
        std::fs::create_dir_all(outside.join("tex")).unwrap();
        std::fs::write(
            outside.join("crate.mtl"),
            "newmtl Wood\nmap_Kd tex/wood.png\n",
        )
        .unwrap();
        std::fs::write(outside.join("tex/wood.png"), b"png").unwrap();
        let src = outside.join("crate.obj");
        std::fs::write(&src, format!("mtllib crate.mtl\nusemtl Wood\n{TRI}")).unwrap();
        let dest = import_model(&src, &project).unwrap();
        assert_eq!(dest, project.join("models/crate/crate.obj"));
        assert!(project.join("models/crate/crate.mtl").is_file());
        assert!(project.join("models/crate/tex/wood.png").is_file());
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn non_models_are_refused() {
        let project = temp("proj3");
        let err = import_model(Path::new("/tmp/notes.txt"), &project).unwrap_err();
        assert!(err.contains("isn't a model"), "{err}");
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn project_models_are_listed_and_names_read_well() {
        let project = temp("proj4");
        std::fs::create_dir_all(project.join("models/trees")).unwrap();
        std::fs::create_dir_all(project.join("build/linux")).unwrap();
        std::fs::write(project.join("models/trees/oak_tree.glb"), b"").unwrap();
        std::fs::write(project.join("models/rock.OBJ"), b"").unwrap();
        std::fs::write(project.join("build/linux/copy.glb"), b"").unwrap();
        std::fs::write(project.join("level.ron"), b"").unwrap();
        let found = project_models(&project, 100);
        assert_eq!(
            found,
            [
                project.join("models/rock.OBJ"),
                project.join("models/trees/oak_tree.glb")
            ]
        );
        assert_eq!(entity_name(&found[1]), "Oak tree");
        assert_eq!(entity_name(Path::new("x/-.glb")), "Model");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        let _ = std::fs::remove_dir_all(&project);
    }
}
