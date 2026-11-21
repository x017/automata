//! Loads the sample models in `examples/models` (made by `make_models.py`).

use std::path::PathBuf;

fn example(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/models")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_sample_tree_loads_with_two_materials_and_an_embedded_texture() {
    let tree = automata_assets::load(&example("tree.glb")).expect("tree.glb loads");
    assert_eq!(tree.parts.len(), 2, "trunk and canopy");
    let bark = tree.parts.iter().find(|p| p.name == "Bark").unwrap();
    let leaves = tree.parts.iter().find(|p| p.name == "Leaves").unwrap();
    assert!(bark.texture.is_none());
    assert!((bark.color[0] - 0.42).abs() < 1e-3);
    let tex = leaves.texture.as_ref().expect("the canopy texture decodes");
    assert_eq!((tex.width, tex.height), (32, 32));
    // Stands on its origin: lowest point at y = 0, 2.4 m tall.
    assert!(tree.min.y.abs() < 1e-4 && (tree.max.y - 2.4).abs() < 1e-3);
    assert_eq!(tree.triangle_count(), 14 + 4 * 9);
}

#[test]
fn the_sample_crate_loads_its_mtl_and_texture() {
    let path = example("crate.obj");
    let crate_ = automata_assets::load(&path).expect("crate.obj loads");
    assert_eq!(crate_.parts.len(), 1);
    assert_eq!(crate_.triangle_count(), 12);
    let tex = crate_.parts[0].texture.as_ref().expect("map_Kd decodes");
    assert_eq!((tex.width, tex.height), (64, 64));
    assert!((crate_.size().x - 0.9).abs() < 1e-4);
    // Its companions are what an import copies alongside it.
    let deps: Vec<String> = automata_assets::dependencies(std::path::Path::new(&path))
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(deps, ["crate.mtl", "crate.png"]);
}
