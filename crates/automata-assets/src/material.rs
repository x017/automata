//! Material files: `.mat.ron` assets that entities share.
//!
//! A [`MaterialFile`] is a colour, a surface (roughness, metalness, glow) and
//! up to four texture maps, with tiling. Entities name one by path
//! (`material: "materials/brick.mat.ron"` in a scene); every entity that
//! names the same file draws with one shared GPU material, so their draws
//! batch, and changing the file changes them all.
//!
//! Texture paths inside a material are relative to the material file's own
//! folder (like a glTF file's textures), so a material and its textures can
//! move together.
//!
//! [`load_material`] goes through a cache that notices when the file or one
//! of its textures changes on disk ([`crate::refresh`] reports it). Editors
//! can show unsaved edits everywhere at once with [`set_material_preview`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use automata_renderer::{MaterialParams, Renderer, Texture};
use serde::{Deserialize, Serialize};

use crate::Image;

/// What a material file's name ends with.
pub const MATERIAL_SUFFIX: &str = ".mat.ron";

/// Whether `path` names a material file (`*.mat.ron`, any case).
#[must_use]
pub fn is_material_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(MATERIAL_SUFFIX) && n.len() > 8)
}

/// The image file extensions a material's maps can use.
pub const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Whether `path` is an image a material can use as a map.
#[must_use]
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The four texture slots of a material, in [`MaterialFile::maps`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapSlot {
    /// Base colour (sRGB), multiplied with the colour.
    Base,
    /// Tangent-space normal map (OpenGL convention, green up).
    Normal,
    /// glTF roughness-metal map: green = roughness, blue = metalness.
    RoughnessMetal,
    /// Emission colour (sRGB), multiplied with the emission.
    Emissive,
}

impl MapSlot {
    /// Every slot, in file order.
    pub const ALL: [MapSlot; 4] = [
        MapSlot::Base,
        MapSlot::Normal,
        MapSlot::RoughnessMetal,
        MapSlot::Emissive,
    ];

    /// The slot's name in the editor.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            MapSlot::Base => "Base Color",
            MapSlot::Normal => "Normal",
            MapSlot::RoughnessMetal => "Rough / Metal",
            MapSlot::Emissive => "Emission",
        }
    }

    /// The slot's field in the file.
    #[must_use]
    pub fn field(self) -> &'static str {
        match self {
            MapSlot::Base => "base_map",
            MapSlot::Normal => "normal_map",
            MapSlot::RoughnessMetal => "roughness_metal_map",
            MapSlot::Emissive => "emissive_map",
        }
    }

    /// Whether the map holds colour (decoded as sRGB) rather than data.
    #[must_use]
    pub fn is_color(self) -> bool {
        matches!(self, MapSlot::Base | MapSlot::Emissive)
    }

    fn index(self) -> usize {
        match self {
            MapSlot::Base => 0,
            MapSlot::Normal => 1,
            MapSlot::RoughnessMetal => 2,
            MapSlot::Emissive => 3,
        }
    }
}

/// A material file's contents. Every field is optional in the file;
/// left out, it takes the plain default (white, roughness 0.6, not metal,
/// no glow, no maps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaterialFile {
    /// Base colour, linear RGBA like a mesh's colour in a scene. Multiplied
    /// with the base colour map.
    pub color: [f32; 4],
    /// 0 = mirror-smooth, 1 = chalky. Multiplied with the map's green.
    #[serde(skip_serializing_if = "is_default_roughness")]
    pub roughness: f32,
    /// 0 = plastic, wood, stone; 1 = bare metal. Multiplied with the map's
    /// blue.
    #[serde(skip_serializing_if = "is_zero")]
    pub metallic: f32,
    /// Light the surface gives off, RGB 0..1 (times `emissive_strength`).
    /// Multiplied with the emission map.
    #[serde(skip_serializing_if = "is_black")]
    pub emissive: [f32; 3],
    /// Multiplier on `emissive`; above 1 it glows (and blooms on
    /// Beautiful).
    #[serde(skip_serializing_if = "is_one")]
    pub emissive_strength: f32,
    /// How much the glow lights the surroundings (1 like a real glowing
    /// object, 0 not at all).
    #[serde(skip_serializing_if = "is_one")]
    pub glow_light: f32,
    /// Whether objects block that light.
    #[serde(skip_serializing_if = "is_true")]
    pub glow_shadows: bool,
    /// Base colour map, relative to the material file's folder.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub base_map: String,
    /// Normal map (OpenGL convention: green up, as glTF uses).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub normal_map: String,
    /// How strongly the normal map bends the surface.
    #[serde(skip_serializing_if = "is_one")]
    pub normal_strength: f32,
    /// glTF roughness-metal map: green scales roughness, blue metalness.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub roughness_metal_map: String,
    /// Emission map (sRGB), multiplied with the emission colour.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub emissive_map: String,
    /// How many times the maps repeat across the mesh, in U and V.
    #[serde(skip_serializing_if = "is_unit")]
    pub tiling: [f32; 2],
    /// Shifts the maps across the mesh (in texture widths).
    #[serde(skip_serializing_if = "is_origin")]
    pub offset: [f32; 2],
    /// Sample the colour maps without smoothing (crisp pixel art).
    #[serde(skip_serializing_if = "is_false")]
    pub pixelated: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_default_roughness(v: &f32) -> bool {
    *v == automata_renderer::DEFAULT_ROUGHNESS
}
#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_black(v: &[f32; 3]) -> bool {
    *v == [0.0; 3]
}
#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_unit(v: &[f32; 2]) -> bool {
    *v == [1.0; 2]
}
#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_origin(v: &[f32; 2]) -> bool {
    *v == [0.0; 2]
}
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_true(v: &bool) -> bool {
    *v
}
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_false(v: &bool) -> bool {
    !*v
}

impl Default for MaterialFile {
    fn default() -> Self {
        Self {
            color: [1.0; 4],
            roughness: automata_renderer::DEFAULT_ROUGHNESS,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            glow_light: 1.0,
            glow_shadows: true,
            base_map: String::new(),
            normal_map: String::new(),
            normal_strength: 1.0,
            roughness_metal_map: String::new(),
            emissive_map: String::new(),
            tiling: [1.0; 2],
            offset: [0.0; 2],
            pixelated: false,
        }
    }
}

const HEADER: &str = "// A material. Entities that use it share its colour, surface and\n\
// textures: material: \"PATH\" on a Mesh Renderer. Texture paths are\n\
// relative to this file's folder.\n";

impl MaterialFile {
    /// Reads a material file's text.
    ///
    /// # Errors
    /// The text isn't a material (the message says where and why).
    pub fn parse(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    /// The file's text, with a header naming the file (`name`, as scenes
    /// refer to it).
    #[must_use]
    pub fn to_ron(&self, name: &str) -> String {
        let config = ron::ser::PrettyConfig::new()
            .indentor("    ".into())
            .separate_tuple_members(false);
        let body = ron::ser::to_string_pretty(self, config).unwrap_or_default();
        format!("{}{body}\n", HEADER.replace("PATH", name))
    }

    /// Writes the file (creating its folder).
    ///
    /// # Errors
    /// The file can't be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::fs::write(path, self.to_ron(&format!("materials/{name}")))
    }

    /// The map path in `slot` (empty: none).
    #[must_use]
    pub fn map(&self, slot: MapSlot) -> &str {
        match slot {
            MapSlot::Base => &self.base_map,
            MapSlot::Normal => &self.normal_map,
            MapSlot::RoughnessMetal => &self.roughness_metal_map,
            MapSlot::Emissive => &self.emissive_map,
        }
    }

    /// The map path in `slot`, to change.
    pub fn map_mut(&mut self, slot: MapSlot) -> &mut String {
        match slot {
            MapSlot::Base => &mut self.base_map,
            MapSlot::Normal => &mut self.normal_map,
            MapSlot::RoughnessMetal => &mut self.roughness_metal_map,
            MapSlot::Emissive => &mut self.emissive_map,
        }
    }

    /// The emitted light: `emissive × emissive_strength`.
    #[must_use]
    pub fn glow(&self) -> [f32; 3] {
        let k = self.emissive_strength.max(0.0);
        self.emissive.map(|c| c.max(0.0) * k)
    }

    /// The renderer's parameters (values clamped to their ranges).
    /// `emissive_coverage` scales the light the glow throws: how much of
    /// the emission map is lit (1 without one).
    #[must_use]
    pub fn params(&self, emissive_coverage: f32) -> MaterialParams {
        let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
        let tile = |v: f32| {
            let v = finite(v, 1.0);
            if v.abs() < 1e-4 {
                1.0
            } else {
                v
            }
        };
        MaterialParams {
            color: self.color.map(|c| finite(c, 1.0).max(0.0)),
            roughness: finite(self.roughness, automata_renderer::DEFAULT_ROUGHNESS).clamp(0.0, 1.0),
            metallic: finite(self.metallic, 0.0).clamp(0.0, 1.0),
            emissive: self.glow().map(|c| finite(c, 0.0)),
            normal_scale: finite(self.normal_strength, 1.0),
            glow_light: finite(self.glow_light, 1.0).max(0.0) * emissive_coverage,
            glow_shadows: self.glow_shadows,
            uv_scale: [tile(self.tiling[0]), tile(self.tiling[1])],
            uv_offset: [finite(self.offset[0], 0.0), finite(self.offset[1], 0.0)],
        }
    }

    /// The material a primitive with this colour and surface draws with:
    /// the starting point of "New Material from this surface".
    #[must_use]
    pub fn from_params(p: &MaterialParams) -> Self {
        Self {
            color: p.color,
            roughness: p.roughness,
            metallic: p.metallic,
            emissive: p.emissive,
            glow_light: p.glow_light,
            glow_shadows: p.glow_shadows,
            ..Self::default()
        }
    }
}

/// Material loading failures.
#[derive(Debug, Clone, thiserror::Error)]
pub enum MaterialError {
    /// No file at the path.
    #[error("material file not found: {0}")]
    NotFound(String),
    /// The file isn't a material.
    #[error("{path}: {message}")]
    Invalid {
        /// The file.
        path: String,
        /// What is wrong.
        message: String,
    },
}

/// A loaded material: the file, its decoded maps, and its shared GPU
/// material.
#[derive(Debug)]
pub struct MaterialAsset {
    /// The file it was loaded from.
    pub path: PathBuf,
    /// What the file says.
    pub file: MaterialFile,
    /// The decoded maps, in [`MapSlot::ALL`] order.
    pub maps: [Option<Arc<Image>>; 4],
    /// Maps that couldn't be read, with why ("textures/x.png: not found").
    /// The material still draws, without them.
    pub problems: Vec<String>,
    gpu: OnceLock<automata_renderer::Material>,
}

impl MaterialAsset {
    /// Builds an asset from a parsed file: reads its maps (relative to
    /// `path`'s folder) through the image cache.
    #[must_use]
    pub fn new(path: PathBuf, file: MaterialFile) -> Self {
        let mut problems = Vec::new();
        let maps = MapSlot::ALL.map(|slot| {
            let rel = file.map(slot).trim();
            if rel.is_empty() {
                return None;
            }
            let image_path = map_path(&path, rel);
            match load_image(&image_path) {
                Ok(img) => Some(img),
                Err(e) => {
                    problems.push(format!("{} map {rel}: {e}", slot.label()));
                    None
                }
            }
        });
        Self {
            path,
            file,
            maps,
            problems,
            gpu: OnceLock::new(),
        }
    }

    /// The renderer parameters, the glow light following how much of the
    /// emission map is lit.
    #[must_use]
    pub fn params(&self) -> MaterialParams {
        let coverage = self.maps[MapSlot::Emissive.index()]
            .as_ref()
            .map_or(1.0, |img| img.mean_linear());
        self.file.params(coverage)
    }

    /// The decoded map in `slot`, if it has one that loaded.
    #[must_use]
    pub fn map(&self, slot: MapSlot) -> Option<&Arc<Image>> {
        self.maps[slot.index()].as_ref()
    }

    /// The GPU material every entity using this file shares, created on
    /// first use. Textures are uploaded once per image file, even across
    /// materials.
    pub fn gpu(&self, renderer: &Renderer) -> automata_renderer::Material {
        self.gpu
            .get_or_init(|| {
                let tex = |slot: MapSlot| {
                    let img = self.map(slot)?;
                    let pixelated = self.file.pixelated && slot.is_color();
                    Some(gpu_texture(renderer, img, slot, pixelated))
                };
                renderer.create_pbr_material(
                    &self.params(),
                    automata_renderer::MaterialTextures {
                        base: tex(MapSlot::Base),
                        normal: tex(MapSlot::Normal),
                        metallic_roughness: tex(MapSlot::RoughnessMetal),
                        emissive: tex(MapSlot::Emissive),
                    },
                )
            })
            .clone()
    }
}

/// Where a map path in the material at `material` points: relative paths
/// against the material file's folder.
#[must_use]
pub fn map_path(material: &Path, rel: &str) -> PathBuf {
    if Path::new(rel).is_absolute() {
        return PathBuf::from(rel);
    }
    let dir = material.parent().unwrap_or_else(|| Path::new(""));
    // `..` resolved by name, so a missing file still reads cleanly.
    let mut out = PathBuf::new();
    for part in crate::join_relative(dir, rel).components() {
        match part {
            std::path::Component::ParentDir
                if matches!(
                    out.components().next_back(),
                    Some(std::path::Component::Normal(_))
                ) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The path to write in a material at `material` for the image at `image`:
/// relative to the material's folder, `/`-separated (`../textures/a.png`
/// when it's elsewhere in the project).
#[must_use]
pub fn relative_map_path(material: &Path, image: &Path) -> String {
    let dir = material.parent().unwrap_or_else(|| Path::new(""));
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (dir, image) = (canon(dir), canon(image));
    let from: Vec<_> = dir.components().collect();
    let to: Vec<_> = image.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return image.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// The image files a material uses (missing ones included), for exports
/// to check. Empty when the file doesn't read.
#[must_use]
pub fn material_dependencies(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(file) = MaterialFile::parse(&text) else {
        return Vec::new();
    };
    MapSlot::ALL
        .iter()
        .map(|&s| file.map(s).trim())
        .filter(|rel| !rel.is_empty())
        .map(|rel| map_path(path, rel))
        .collect()
}

// ---------------------------------------------------------------------------
// The caches.

struct MatEntry {
    /// The material file and its maps, with their modification times.
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    result: Result<Arc<MaterialAsset>, String>,
}

impl MatEntry {
    fn stale(&self) -> bool {
        self.stamps.iter().any(|(p, t)| crate::modified(p) != *t)
    }
}

type ImageEntry = (Option<SystemTime>, Result<Arc<Image>, String>);

static MATERIALS: Mutex<BTreeMap<PathBuf, MatEntry>> = Mutex::new(BTreeMap::new());
static PREVIEWS: Mutex<BTreeMap<PathBuf, Arc<MaterialAsset>>> = Mutex::new(BTreeMap::new());
static IMAGES: Mutex<BTreeMap<PathBuf, ImageEntry>> = Mutex::new(BTreeMap::new());
/// Uploaded maps by image (its address, kept alive by the entry) and how
/// they were uploaded.
type GpuKey = (usize, bool, bool);
static TEXTURES: Mutex<Vec<(GpuKey, Arc<Image>, Texture)>> = Mutex::new(Vec::new());

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Reads an image through the cache (re-read once the file changes).
fn load_image(path: &Path) -> Result<Arc<Image>, String> {
    let key = canonical(path);
    let stamp = crate::modified(&key);
    if stamp.is_none() {
        return Err("not found".into());
    }
    {
        let images = IMAGES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((t, result)) = images.get(&key) {
            if *t == stamp {
                return result.clone();
            }
        }
    }
    let result = std::fs::read(&key)
        .map_err(|e| e.to_string())
        .and_then(|bytes| Image::decode(&bytes))
        .map(Arc::new);
    IMAGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, (stamp, result.clone()));
    result
}

fn gpu_texture(renderer: &Renderer, img: &Arc<Image>, slot: MapSlot, pixelated: bool) -> Texture {
    let key = (Arc::as_ptr(img) as usize, slot.is_color(), pixelated);
    let mut textures = TEXTURES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, _, t)) = textures.iter().find(|(k, _, _)| *k == key) {
        return t.clone();
    }
    let t = if slot.is_color() {
        renderer.create_texture_rgba_mipmapped(img.width, img.height, &img.rgba, pixelated)
    } else {
        renderer.create_data_texture(img.width, img.height, &img.rgba)
    };
    textures.push((key, img.clone(), t.clone()));
    t
}

/// Loads a material through the shared cache: relative paths resolve
/// like model paths (against the asset root, then the working directory).
/// An editor's unsaved version ([`set_material_preview`]) wins over the
/// file. A file that changed on disk is read again.
///
/// # Errors
/// The file is missing or isn't a material. (Maps that don't load are
/// [`MaterialAsset::problems`], not errors.)
pub fn load_material(path: &str) -> Result<Arc<MaterialAsset>, MaterialError> {
    let file = crate::resolve(path);
    let key = canonical(&file);
    if let Some(asset) = PREVIEWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return Ok(asset.clone());
    }
    {
        let cache = MATERIALS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = cache.get(&key) {
            if !entry.stale() {
                return entry
                    .result
                    .clone()
                    .map_err(|message| MaterialError::Invalid {
                        path: path.to_string(),
                        message,
                    });
            }
        }
    }
    let text = match std::fs::read_to_string(&key) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(MaterialError::NotFound(path.to_string()));
        }
        Err(e) => {
            return Err(MaterialError::Invalid {
                path: path.to_string(),
                message: e.to_string(),
            })
        }
    };
    let mut stamps = vec![(key.clone(), crate::modified(&key))];
    let result = MaterialFile::parse(&text).map(|parsed| {
        for slot in MapSlot::ALL {
            let rel = parsed.map(slot).trim();
            if !rel.is_empty() {
                let p = canonical(&map_path(&key, rel));
                let t = crate::modified(&p);
                stamps.push((p, t));
            }
        }
        let asset = Arc::new(MaterialAsset::new(key.clone(), parsed));
        for problem in &asset.problems {
            tracing::warn!("material {path}: {problem}");
        }
        asset
    });
    MATERIALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            key,
            MatEntry {
                stamps,
                result: result.clone(),
            },
        );
    result.map_err(|message| MaterialError::Invalid {
        path: path.to_string(),
        message,
    })
}

/// Shows `file` instead of what's on disk for the material at `path`
/// (an editor's unsaved changes), or goes back to the disk with `None`.
/// Returns whether anything changed, so the caller knows to rebuild what
/// uses it.
pub fn set_material_preview(path: &Path, file: Option<&MaterialFile>) -> bool {
    let key = canonical(path);
    let mut previews = PREVIEWS.lock().unwrap_or_else(PoisonError::into_inner);
    match file {
        None => previews.remove(&key).is_some(),
        Some(file) => {
            if previews.get(&key).is_some_and(|a| a.file == *file) {
                return false;
            }
            // Unchanged on disk too: no preview needed.
            let on_disk = std::fs::read_to_string(&key)
                .ok()
                .and_then(|t| MaterialFile::parse(&t).ok());
            if on_disk.as_ref() == Some(file) {
                return previews.remove(&key).is_some();
            }
            previews.insert(key.clone(), Arc::new(MaterialAsset::new(key, file.clone())));
            true
        }
    }
}

/// The materials showing an editor's unsaved version.
#[must_use]
pub fn material_previews() -> Vec<PathBuf> {
    PREVIEWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect()
}

/// Drops cached materials whose file or maps changed on disk (and images
/// and uploads no material uses any more). Returns whether any did.
pub(crate) fn refresh_materials() -> bool {
    let changed = {
        let mut cache = MATERIALS.lock().unwrap_or_else(PoisonError::into_inner);
        let before = cache.len();
        cache.retain(|_, entry| !entry.stale());
        cache.len() != before
    };
    // A preview whose maps changed on disk re-reads them.
    let mut previews_changed = false;
    {
        let mut previews = PREVIEWS.lock().unwrap_or_else(PoisonError::into_inner);
        for asset in previews.values_mut() {
            let stale = MapSlot::ALL.iter().any(|&slot| {
                let rel = asset.file.map(slot).trim();
                !rel.is_empty() && {
                    let p = canonical(&map_path(&asset.path, rel));
                    let cached = IMAGES
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&p)
                        .map(|(t, _)| *t);
                    cached != Some(crate::modified(&p))
                }
            });
            if stale {
                *asset = Arc::new(MaterialAsset::new(asset.path.clone(), asset.file.clone()));
                previews_changed = true;
            }
        }
    }
    if changed || previews_changed {
        prune_textures();
    }
    changed || previews_changed
}

/// Forgets uploads of images that only the caches still hold.
fn prune_textures() {
    let mut images = IMAGES.lock().unwrap_or_else(PoisonError::into_inner);
    images.retain(|p, (t, _)| crate::modified(p) == *t);
    drop(images);
    // Kept while the image cache or a material still holds the image.
    TEXTURES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|(_, img, _)| Arc::strong_count(img) > 1);
}

/// Empties the material, image and texture caches (previews stay).
pub(crate) fn clear_materials() {
    MATERIALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    IMAGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    TEXTURES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "automata-mat-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(path: &Path, rgba: [u8; 4]) {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        img.save(path).unwrap();
    }

    #[test]
    fn material_paths_are_recognised() {
        assert!(is_material_path(Path::new("materials/Brick.MAT.RON")));
        assert!(!is_material_path(Path::new("level.ron")));
        assert!(!is_material_path(Path::new(".mat.ron")));
        assert!(is_image_path(Path::new("t/a.JPG")));
        assert!(!is_image_path(Path::new("t/a.gif")));
    }

    #[test]
    fn files_stay_short_and_round_trip() {
        let plain = MaterialFile::default();
        let text = plain.to_ron("materials/plain.mat.ron");
        assert!(text.starts_with("// A material."), "{text}");
        assert!(text.contains("materials/plain.mat.ron"));
        assert!(!text.contains("roughness"), "defaults are left out: {text}");
        assert_eq!(MaterialFile::parse(&text).unwrap(), plain);
        let brick = MaterialFile {
            color: [0.8, 0.4, 0.3, 1.0],
            roughness: 0.9,
            base_map: "../textures/brick.png".into(),
            tiling: [4.0, 2.0],
            ..MaterialFile::default()
        };
        let text = brick.to_ron("materials/brick.mat.ron");
        assert!(text.contains("tiling: (4.0, 2.0)"), "{text}");
        assert_eq!(MaterialFile::parse(&text).unwrap(), brick);
        // Everything is optional, and typos are caught.
        assert_eq!(
            MaterialFile::parse("(metallic: 1.0)").unwrap().metallic,
            1.0
        );
        let err = MaterialFile::parse("(roughnes: 0.2)").unwrap_err();
        assert!(err.contains("roughnes"), "{err}");
    }

    #[test]
    fn params_clamp_and_tile() {
        let file = MaterialFile {
            roughness: 3.0,
            metallic: -1.0,
            emissive: [1.0, 0.5, 0.0],
            emissive_strength: 4.0,
            tiling: [0.0, 3.0],
            offset: [0.25, 0.5],
            ..MaterialFile::default()
        };
        let p = file.params(0.5);
        assert_eq!(p.roughness, 1.0);
        assert_eq!(p.metallic, 0.0);
        assert_eq!(p.emissive, [4.0, 2.0, 0.0]);
        assert_eq!(p.glow_light, 0.5);
        assert_eq!(
            p.uv_scale,
            [1.0, 3.0],
            "a zero tiling would collapse the map"
        );
        assert_eq!(p.uv_offset, [0.25, 0.5]);
    }

    #[test]
    fn map_paths_are_relative_to_the_material() {
        let mat = Path::new("/game/materials/brick.mat.ron");
        assert_eq!(
            map_path(mat, "../textures/brick.png"),
            PathBuf::from("/game/textures/brick.png")
        );
        assert_eq!(
            map_path(mat, "brick.png"),
            PathBuf::from("/game/materials/brick.png")
        );
        let dir = temp_dir("rel");
        png(&dir.join("textures/a.png"), [255; 4]);
        std::fs::create_dir_all(dir.join("materials")).unwrap();
        let m = dir.join("materials/a.mat.ron");
        assert_eq!(
            relative_map_path(&m, &dir.join("textures/a.png")),
            "../textures/a.png"
        );
        assert_eq!(
            relative_map_path(&dir.join("a.mat.ron"), &dir.join("textures/a.png")),
            "textures/a.png"
        );
    }

    #[test]
    fn materials_load_share_and_reload() {
        let _serial = crate::REFRESH_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = temp_dir("load");
        png(&dir.join("textures/red.png"), [255, 0, 0, 255]);
        let path = dir.join("materials/red.mat.ron");
        MaterialFile {
            base_map: "../textures/red.png".into(),
            normal_map: "../textures/missing.png".into(),
            ..MaterialFile::default()
        }
        .save(&path)
        .unwrap();
        let rel = path.to_string_lossy().into_owned();
        let a = load_material(&rel).unwrap();
        let b = load_material(&rel).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "one asset for every user");
        assert_eq!(a.map(MapSlot::Base).unwrap().rgba[..4], [255, 0, 0, 255]);
        assert!(a.map(MapSlot::Normal).is_none());
        assert_eq!(a.problems.len(), 1, "{:?}", a.problems);
        assert!(a.problems[0].contains("missing.png"), "{:?}", a.problems);
        assert_eq!(
            material_dependencies(&path),
            vec![
                dir.join("textures/red.png"),
                dir.join("textures/missing.png")
            ]
        );

        // An editor's unsaved version wins, until it's dropped.
        let edited = MaterialFile {
            metallic: 1.0,
            ..a.file.clone()
        };
        assert!(set_material_preview(&path, Some(&edited)));
        assert!(!set_material_preview(&path, Some(&edited)), "unchanged");
        assert_eq!(load_material(&rel).unwrap().file.metallic, 1.0);
        assert!(material_previews().contains(&canonical(&path)));
        assert!(set_material_preview(&path, None));
        assert_eq!(load_material(&rel).unwrap().file.metallic, 0.0);

        // A changed file is read again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "(roughness: 0.1)").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(later).unwrap();
        assert!(crate::refresh());
        let c = load_material(&rel).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        assert!((c.file.roughness - 0.1).abs() < 1e-6);

        // Missing and broken files are errors.
        let missing = dir.join("nope.mat.ron").to_string_lossy().into_owned();
        assert!(matches!(
            load_material(&missing),
            Err(MaterialError::NotFound(_))
        ));
        std::fs::write(dir.join("bad.mat.ron"), "(colour: 1)").unwrap();
        let bad = dir.join("bad.mat.ron").to_string_lossy().into_owned();
        assert!(matches!(
            load_material(&bad),
            Err(MaterialError::Invalid { .. })
        ));
    }
}
