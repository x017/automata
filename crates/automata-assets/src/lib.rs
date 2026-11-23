//! AUTOMATA assets: 3D models loaded from disk.
//!
//! [`load`] reads a model file (glTF 2.0 `.glb` / `.gltf`, or Wavefront
//! `.obj` with its `.mtl`) into a [`Model`]: one [`ModelPart`] per material,
//! with node transforms baked in so the whole file draws as one object.
//! Each part keeps its base color and color texture. Animated or skinned
//! glTF files also get a [`Rig`]: their parts keep bind-space vertices with
//! joint weights, and their clips pose the joints (see [`anim`]).
//!
//! Loaded models are cached by path and shared (`Arc`), so a hundred trees
//! read the file once and upload it to the GPU once. The cache notices when
//! a file (or a texture or buffer it uses) changes on disk: the next
//! [`load`] re-reads it, and [`refresh`] reports changes so editors can
//! redraw.
//!
//! Relative paths resolve against the asset root ([`set_root`], normally the
//! scene file's folder), then the working directory.

pub mod anim;
mod gltf;
pub mod import;
pub mod json;
mod obj;
pub mod springs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use automata_renderer::{Mesh, MeshData, Renderer, Texture};
use glam::Vec3;

pub use anim::{Clip, ClipEvent, ClipMotion, PartSkin, Pose, Rig};

/// File extensions [`load`] understands (lowercase, without the dot).
pub const MODEL_EXTENSIONS: [&str; 3] = ["glb", "gltf", "obj"];

/// The largest texture side kept after loading; bigger images are scaled
/// down (keeps memory sane and stays within every GPU's limits).
pub const MAX_TEXTURE_SIZE: u32 = 4096;

/// Whether `path` has a model file extension.
#[must_use]
pub fn is_model_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MODEL_EXTENSIONS.iter().any(|m| e.eq_ignore_ascii_case(m)))
}

/// Model loading failures.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// No file at the path (after resolving it against the asset root).
    #[error("model file not found: {0}")]
    NotFound(String),
    /// The file exists but could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        /// The file.
        path: String,
        /// The OS error.
        source: std::io::Error,
    },
    /// The file is not a valid model (or uses an unsupported feature).
    #[error("{path}: {message}")]
    Invalid {
        /// The file.
        path: String,
        /// What is wrong.
        message: String,
    },
    /// The extension is not one of [`MODEL_EXTENSIONS`].
    #[error("{0}: unsupported model format (use .glb, .gltf or .obj)")]
    Unsupported(String),
}

/// A decoded RGBA8 image (a model's color texture).
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels, 4 bytes each, row by row from the top.
    pub rgba: Vec<u8>,
}

impl Image {
    /// The mean linear brightness of the pixels, reading them as sRGB:
    /// 0 for black, 1 for white. Large images are sampled on a grid.
    #[must_use]
    pub fn mean_linear(&self) -> f32 {
        let pixels = self.rgba.len() / 4;
        if pixels == 0 {
            return 0.0;
        }
        // About 64k samples at most.
        let step = (pixels / 65_536).max(1);
        let mut lut = [0.0_f32; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            let c = f32::from(u8::try_from(i).unwrap_or(u8::MAX)) / 255.0;
            *v = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        let (mut sum, mut count) = (0.0_f64, 0_u32);
        for px in self.rgba.chunks_exact(4).step_by(step) {
            let v =
                (lut[usize::from(px[0])] + lut[usize::from(px[1])] + lut[usize::from(px[2])]) / 3.0;
            sum += f64::from(v);
            count += 1;
        }
        #[allow(clippy::cast_possible_truncation)] // a mean of 0..1 values
        let mean = (sum / f64::from(count.max(1))) as f32;
        mean
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

impl Image {
    /// Decodes a PNG or JPEG, scaling it down to [`MAX_TEXTURE_SIZE`].
    ///
    /// # Errors
    ///
    /// Returns the decoder's message when the bytes are not an image.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut img = image::load_from_memory(bytes)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        let (w, h) = img.dimensions();
        if w > MAX_TEXTURE_SIZE || h > MAX_TEXTURE_SIZE {
            let scale = MAX_TEXTURE_SIZE as f32 / w.max(h) as f32;
            let fit = |v: u32| {
                (f64::from(v) * f64::from(scale))
                    .round()
                    .clamp(1.0, f64::from(MAX_TEXTURE_SIZE))
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (nw, nh) = (fit(w) as u32, fit(h) as u32);
            img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
        }
        let (width, height) = img.dimensions();
        Ok(Self {
            width,
            height,
            rgba: img.into_raw(),
        })
    }
}

/// One drawable piece of a model: geometry with a single material.
#[derive(Debug, Clone)]
pub struct ModelPart {
    /// The material (or mesh) name from the file.
    pub name: String,
    /// Geometry in model space (node transforms already applied).
    pub mesh: MeshData,
    /// Base color, RGBA (multiplied with the texture).
    pub color: [f32; 4],
    /// Color texture, shared between parts that use the same image.
    pub texture: Option<Arc<Image>>,
    /// Whether the texture is sampled without smoothing when magnified (the
    /// file asked for nearest filtering: palette atlases, pixel art).
    pub pixelated: bool,
    /// How the part follows the model's [`Rig`]: bind-space vertices plus
    /// joints and weights. `mesh` then holds the rest pose.
    pub skin: Option<PartSkin>,
    /// Roughness, metalness, glow and the extra PBR maps.
    pub surface: PartSurface,
}

/// A part's surface beyond its base color: glTF's metallic-roughness
/// parameters and maps. The default is the engine's plain look (slightly
/// rough, non-metal, no glow, no maps), which formats without PBR data
/// (OBJ) get.
#[derive(Debug, Clone, PartialEq)]
pub struct PartSurface {
    /// 0 (mirror) to 1 (chalk).
    pub roughness: f32,
    /// 0 for dielectrics, 1 for bare metal.
    pub metallic: f32,
    /// Emitted light, linear RGB (already scaled by any emissive strength).
    pub emissive: [f32; 3],
    /// Normal map strength.
    pub normal_scale: f32,
    /// Tangent-space normal map (data, not color).
    pub normal_map: Option<Arc<Image>>,
    /// G = roughness, B = metallic (data, not color).
    pub metallic_roughness_map: Option<Arc<Image>>,
    /// Emissive color map (sRGB).
    pub emissive_map: Option<Arc<Image>>,
    /// How much of the emissive map glows: its mean linear brightness, 0
    /// (black) to 1 (white), and 1 without a map. Scales the light a
    /// glowing part throws on its surroundings (a few lit windows light
    /// less than a whole glowing wall).
    pub emissive_coverage: f32,
}

impl Default for PartSurface {
    fn default() -> Self {
        Self {
            roughness: automata_renderer::DEFAULT_ROUGHNESS,
            metallic: 0.0,
            emissive: [0.0; 3],
            normal_scale: 1.0,
            normal_map: None,
            metallic_roughness_map: None,
            emissive_map: None,
            emissive_coverage: 1.0,
        }
    }
}

// The roles an image can be uploaded for as a PBR map.
const MAP_NORMAL: u8 = 0;
const MAP_METAL_ROUGH: u8 = 1;
const MAP_EMISSIVE: u8 = 2;

/// A part's GPU resources (see [`Model::gpu`]).
#[derive(Debug, Clone)]
pub struct GpuPart {
    /// The uploaded geometry.
    pub mesh: Mesh,
    /// The uploaded color texture, if the part has one.
    pub texture: Option<Texture>,
    /// The uploaded normal, metallic-roughness and emissive maps.
    pub maps: automata_renderer::MaterialTextures,
    /// The bind-space geometry with joints and weights, for parts that
    /// follow the model's rig (drawn with a joint palette).
    pub skinned: Option<Mesh>,
}

/// A loaded model: its parts plus model-space bounds.
#[derive(Debug)]
pub struct Model {
    /// The file it was loaded from.
    pub path: PathBuf,
    /// Drawable parts, one per material.
    pub parts: Vec<ModelPart>,
    /// Bounding box minimum corner (model space).
    pub min: Vec3,
    /// Bounding box maximum corner (model space).
    pub max: Vec3,
    /// The node hierarchy, joints and animations of an animated or skinned
    /// file.
    pub rig: Option<Arc<Rig>>,
    gpu: OnceLock<Vec<GpuPart>>,
}

impl Model {
    /// Builds a model from parts, computing its bounds. Parts without
    /// triangles are dropped.
    #[must_use]
    pub fn new(path: PathBuf, parts: Vec<ModelPart>) -> Self {
        let parts: Vec<ModelPart> = parts
            .into_iter()
            .filter(|p| !p.mesh.indices.is_empty() && !p.mesh.vertices.is_empty())
            .collect();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in parts.iter().flat_map(|p| &p.mesh.vertices) {
            let p = Vec3::from(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        if !min.is_finite() || !max.is_finite() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        Self {
            path,
            parts,
            min,
            max,
            rig: None,
            gpu: OnceLock::new(),
        }
    }

    /// Attaches a rig (see [`Model::rig`]).
    #[must_use]
    pub fn with_rig(mut self, rig: Option<Rig>) -> Self {
        self.rig = rig.map(Arc::new);
        self
    }

    /// The names of the model's animations, in file order.
    #[must_use]
    pub fn clip_names(&self) -> Vec<&str> {
        self.rig
            .as_ref()
            .map(|r| r.clip_names())
            .unwrap_or_default()
    }

    /// The bounding box centre (model space).
    #[must_use]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// The bounding box size (model space).
    #[must_use]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// Every part's geometry merged into one triangle soup (model space,
    /// vertices scaled by `scale`), for building collision shapes.
    /// Triangles that point at missing vertices are skipped.
    #[must_use]
    pub fn triangles(&self, scale: Vec3) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let total: usize = self.parts.iter().map(|p| p.mesh.vertices.len()).sum();
        let mut points = Vec::with_capacity(total);
        let mut tris = Vec::new();
        for part in &self.parts {
            let base = u32::try_from(points.len()).unwrap_or(u32::MAX);
            let count = part.mesh.vertices.len();
            points.extend(
                part.mesh
                    .vertices
                    .iter()
                    .map(|v| Vec3::from(v.position) * scale),
            );
            for t in part.mesh.indices.chunks_exact(3) {
                if t.iter().all(|&i| (i as usize) < count) {
                    tris.push([base + t[0], base + t[1], base + t[2]]);
                }
            }
        }
        (points, tris)
    }

    /// Total triangles over all parts.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.indices.len() / 3).sum()
    }

    /// Total vertices over all parts.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.vertices.len()).sum()
    }

    /// The model's GPU resources, uploaded on first use and shared by every
    /// entity that shows this model. Textures used by several parts are
    /// uploaded once.
    pub fn gpu(&self, renderer: &Renderer) -> &[GpuPart] {
        self.gpu.get_or_init(|| {
            let mut uploaded: Vec<((*const Image, bool), Texture)> = Vec::new();
            // Images used as PBR maps, by image and role (the same image as
            // color and as data needs two uploads: sRGB vs linear).
            let mut maps: Vec<((*const Image, u8), Texture)> = Vec::new();
            self.parts
                .iter()
                .map(|part| {
                    let texture = part.texture.as_ref().map(|img| {
                        let key = (Arc::as_ptr(img), part.pixelated);
                        if let Some((_, t)) = uploaded.iter().find(|(k, _)| *k == key) {
                            return t.clone();
                        }
                        let t = renderer.create_texture_rgba_mipmapped(
                            img.width,
                            img.height,
                            &img.rgba,
                            part.pixelated,
                        );
                        uploaded.push((key, t.clone()));
                        t
                    });
                    let skinned = part.skin.as_ref().map(|skin| {
                        let bind = MeshData::new(skin.bind.clone(), part.mesh.indices.clone());
                        renderer.create_skinned_mesh(&bind, &skin.skin)
                    });
                    let surface = &part.surface;
                    let mut upload_map = |img: &Option<Arc<Image>>, kind: u8| {
                        let img = img.as_ref()?;
                        let key = (Arc::as_ptr(img), kind);
                        if let Some((_, t)) = maps.iter().find(|(k, _)| *k == key) {
                            return Some(t.clone());
                        }
                        let t = if kind == MAP_EMISSIVE {
                            renderer.create_texture_rgba_mipmapped(
                                img.width, img.height, &img.rgba, false,
                            )
                        } else {
                            renderer.create_data_texture(img.width, img.height, &img.rgba)
                        };
                        maps.push((key, t.clone()));
                        Some(t)
                    };
                    let maps = automata_renderer::MaterialTextures {
                        base: None,
                        normal: upload_map(&surface.normal_map, MAP_NORMAL),
                        metallic_roughness: upload_map(
                            &surface.metallic_roughness_map,
                            MAP_METAL_ROUGH,
                        ),
                        emissive: upload_map(&surface.emissive_map, MAP_EMISSIVE),
                    };
                    GpuPart {
                        mesh: renderer.create_mesh(&part.mesh),
                        texture,
                        maps,
                        skinned,
                    }
                })
                .collect()
        })
    }
}

/// What a loader produced: the parts plus every extra file it read
/// (buffers, textures, `.mtl`), which the cache watches for changes.
pub(crate) struct Loaded {
    pub parts: Vec<ModelPart>,
    pub rig: Option<Rig>,
    pub deps: Vec<PathBuf>,
}

/// Loads a model file without the cache.
///
/// # Errors
///
/// Fails when the file is missing, unreadable, not a supported format, or
/// malformed.
pub fn load_file(path: &Path) -> Result<Model, ModelError> {
    load_uncached(path).map(|(model, _)| model)
}

fn load_uncached(path: &Path) -> Result<(Model, Vec<PathBuf>), ModelError> {
    let shown = path.display().to_string();
    if !path.is_file() {
        return Err(ModelError::NotFound(shown));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|source| ModelError::Io {
        path: shown.clone(),
        source,
    })?;
    let loaded = match ext.as_str() {
        "glb" | "gltf" => gltf::load(path, &bytes),
        "obj" => obj::load(path, &bytes),
        _ => return Err(ModelError::Unsupported(shown)),
    }
    .map_err(|message| ModelError::Invalid {
        path: shown.clone(),
        message,
    })?;
    let mut parts = loaded.parts;
    let mut rig = loaded.rig;
    let settings = import::read_settings(path);
    let import = settings.apply(&mut parts);
    // Clip files are watched even when they fail to load.
    let clip_files: Vec<PathBuf> = settings.clips.iter().map(|c| c.path(path)).collect();
    if let Some(rig) = &mut rig {
        rig.import = import;
        for source in &settings.clips {
            if let Err(why) = add_clip_file(path, source, rig) {
                tracing::warn!("{}: clips from {}: {why}", path.display(), source.file);
            }
        }
        for mark in &settings.events {
            if let Some(clip) = rig.clips.iter_mut().find(|c| c.name == mark.clip) {
                clip.add_event(mark.time, mark.name.clone());
            } else {
                tracing::warn!(
                    "{}: event {:?} is on clip {:?}, which the model doesn't have",
                    path.display(),
                    mark.name,
                    mark.clip
                );
            }
        }
        rig.springs = settings.springs;
    } else if let Some(source) = settings.clips.first() {
        tracing::warn!(
            "{}: can't add clips from {}: the model has no skeleton",
            path.display(),
            source.file
        );
    }
    let model = Model::new(path.to_path_buf(), parts).with_rig(rig);
    if model.parts.is_empty() {
        return Err(ModelError::Invalid {
            path: shown,
            message: "the file has no triangle meshes".into(),
        });
    }
    // The settings file is watched even before it exists, so creating it
    // reloads the model.
    let mut deps = loaded.deps;
    deps.push(import::settings_path(path));
    deps.extend(clip_files);
    Ok((model, deps))
}

/// Adds the clips of `source` (a glTF for the same skeleton) to `rig`,
/// matching bones by name. Returns how many were added.
fn add_clip_file(model: &Path, source: &import::ClipFile, rig: &mut Rig) -> Result<usize, String> {
    let file = source.path(model);
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ext != "glb" && ext != "gltf" {
        return Err("only glTF files (.glb, .gltf) hold animations".into());
    }
    let bytes = std::fs::read(&file).map_err(|_| "file not found".to_string())?;
    let other = gltf::load(&file, &bytes)?
        .rig
        .filter(|r| !r.clips.is_empty())
        .ok_or("it has no animations")?;
    let clip_names: Vec<&str> = other.clips.iter().map(|c| c.name.as_str()).collect();
    let names = source.clip_names(&clip_names);
    let mut added = 0;
    for (clip, name) in other.clips.iter().zip(names) {
        if rig.clip(&name).is_some() {
            tracing::warn!("{}: a clip called {name:?} already exists", model.display());
            continue;
        }
        if let Some(clip) = clip.retarget(&other.nodes, rig, name) {
            rig.clips.push(clip);
            added += 1;
        }
    }
    if added == 0 {
        return Err("none of its animated bones are in the model".into());
    }
    Ok(added)
}

/// The files a model reads besides itself (external buffers, textures, the
/// `.mtl` of an OBJ). Importers copy them along with the model; exporters
/// check they exist. Missing files are still listed.
#[must_use]
pub fn dependencies(path: &Path) -> Vec<PathBuf> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut deps = match ext.as_str() {
        "gltf" | "glb" => gltf::references(path, &bytes),
        "obj" => obj::references(path, &bytes),
        _ => Vec::new(),
    };
    let settings = import::settings_path(path);
    if settings.is_file() {
        deps.extend(
            import::read_settings(path)
                .clips
                .iter()
                .map(|c| c.path(path)),
        );
        deps.push(settings);
    }
    deps
}

// ---------------------------------------------------------------------------
// The shared cache.

struct Entry {
    /// Every file read, with its modification time when loaded.
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    /// The model, or why it failed (kept so a broken file isn't re-parsed
    /// on every call; it is retried once the file changes).
    result: Result<Arc<Model>, String>,
}

impl Entry {
    fn stale(&self) -> bool {
        self.stamps.iter().any(|(p, t)| modified(p) != *t)
    }
}

static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
static CACHE: Mutex<BTreeMap<PathBuf, Entry>> = Mutex::new(BTreeMap::new());

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Sets the folder relative model paths resolve against (normally the open
/// scene's folder). `None` falls back to the working directory only.
pub fn set_root(dir: Option<&Path>) {
    *ROOT.lock().unwrap_or_else(PoisonError::into_inner) = dir.map(Path::to_path_buf);
}

/// The current asset root, if one was set.
#[must_use]
pub fn root() -> Option<PathBuf> {
    ROOT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Joins a `/`- or `\`-separated relative path onto `base` component by
/// component (so it also works under Windows verbatim `\\?\` bases).
#[must_use]
pub fn join_relative(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
    {
        out.push(part);
    }
    out
}

/// Resolves a model path: absolute paths as-is, relative ones against the
/// asset root, then the working directory. Returns the first existing file,
/// or the root-relative candidate when none exists.
#[must_use]
pub fn resolve(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let mut candidates = Vec::new();
    if let Some(root) = root() {
        candidates.push(join_relative(&root, path));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(join_relative(&cwd, path));
    }
    candidates
        .iter()
        .find(|c| c.is_file())
        .or(candidates.first())
        .cloned()
        .unwrap_or_else(|| p.to_path_buf())
}

/// Loads a model through the shared cache (see the crate docs). A file that
/// changed on disk since it was cached is read again.
///
/// # Errors
///
/// See [`load_file`].
pub fn load(path: &str) -> Result<Arc<Model>, ModelError> {
    let file = resolve(path);
    {
        let cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = cache.get(&file) {
            if !entry.stale() {
                return entry.result.clone().map_err(|message| ModelError::Invalid {
                    path: file.display().to_string(),
                    message,
                });
            }
        }
    }
    let (model, deps) = match load_uncached(&file) {
        Ok(loaded) => loaded,
        Err(ModelError::Invalid { message, .. }) => {
            CACHE.lock().unwrap_or_else(PoisonError::into_inner).insert(
                file.clone(),
                Entry {
                    stamps: vec![(file.clone(), modified(&file))],
                    result: Err(message.clone()),
                },
            );
            return Err(ModelError::Invalid {
                path: file.display().to_string(),
                message,
            });
        }
        Err(e) => return Err(e),
    };
    let model = Arc::new(model);
    let mut stamps = vec![(file.clone(), modified(&file))];
    stamps.extend(deps.into_iter().map(|d| {
        let t = modified(&d);
        (d, t)
    }));
    tracing::info!(
        "loaded model {} ({} parts, {} triangles)",
        file.display(),
        model.parts.len(),
        model.triangle_count()
    );
    CACHE.lock().unwrap_or_else(PoisonError::into_inner).insert(
        file,
        Entry {
            stamps,
            result: Ok(model.clone()),
        },
    );
    Ok(model)
}

/// Drops cached models whose files changed on disk. Returns true when any
/// did, so callers (editors) know to reload the scene's visuals.
pub fn refresh() -> bool {
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    let before = cache.len();
    cache.retain(|_, entry| !entry.stale());
    cache.len() != before
}

/// Empties the cache (the next [`load`] of every model reads the file).
pub fn clear_cache() {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner).clear();
}

// ---------------------------------------------------------------------------
// Helpers shared by the loaders.

/// Decodes standard or URL-safe base64, ignoring whitespace and padding.
pub(crate) fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0_u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return Err(format!("invalid base64 character {:?}", c as char)),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

/// Decodes `%20`-style escapes in a URI.
pub(crate) fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..=i + 2]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Flat normals for a triangle list: every triangle gets its own three
/// vertices facing the triangle's normal.
pub(crate) fn unweld_flat(vertices: &[automata_renderer::Vertex], indices: &[u32]) -> MeshData {
    let mut out_v = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| vertices[i as usize]);
        let n = (Vec3::from(b.position) - Vec3::from(a.position))
            .cross(Vec3::from(c.position) - Vec3::from(a.position))
            .normalize_or(Vec3::Y);
        for mut v in [a, b, c] {
            v.normal = n.into();
            out_v.push(v);
        }
    }
    let count = u32::try_from(out_v.len()).unwrap_or(u32::MAX);
    MeshData::new(out_v, (0..count).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_standard_and_url_safe() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64_decode("+/8=").unwrap(), [0xfb, 0xff]);
        assert_eq!(base64_decode("-_8").unwrap(), [0xfb, 0xff]);
        assert!(base64_decode("a$b").is_err());
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("my%20tex.png"), "my tex.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }

    #[test]
    fn model_extensions_are_case_insensitive() {
        assert!(is_model_path(Path::new("a/Tree.GLB")));
        assert!(is_model_path(Path::new("house.gltf")));
        assert!(is_model_path(Path::new("rock.obj")));
        assert!(!is_model_path(Path::new("level.ron")));
    }

    #[test]
    fn the_cache_shares_models_and_reloads_changed_files() {
        let dir = std::env::temp_dir().join(format!("automata-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tri.obj");
        std::fs::write(&file, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        let path = file.to_str().unwrap();
        let a = load(path).unwrap();
        let b = load(path).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "the second load is a cache hit");
        assert!(!refresh());

        // Rewrite with a newer timestamp: refresh notices, load re-reads.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, "v 0 0 0\nv 2 0 0\nv 0 2 0\nf 1 2 3\n").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        let f = std::fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(later).unwrap();
        drop(f);
        assert!(refresh());
        let c = load(path).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        assert!((c.max.x - 2.0).abs() < 1e-6);

        // A broken file fails (and keeps failing) without panicking.
        std::fs::write(&file, "v 0 0 0\nf 1 2 9\n").unwrap();
        let f = std::fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(later + std::time::Duration::from_secs(2))
            .unwrap();
        drop(f);
        assert!(matches!(load(path), Err(ModelError::Invalid { .. })));
        assert!(matches!(load(path), Err(ModelError::Invalid { .. })));
        assert!(matches!(
            load(dir.join("missing.glb").to_str().unwrap()),
            Err(ModelError::NotFound(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clip_files_add_clips_matched_by_bone_name() {
        use crate::gltf::tests::waving_gltf;
        let dir = std::env::temp_dir().join(format!("clip-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("anims")).unwrap();
        let model = dir.join("robot.gltf");
        std::fs::write(&model, waving_gltf()).unwrap();
        // The same two nodes listed the other way round, so node 1 is
        // `body` here: the clip must follow the name, not the index.
        let swapped = waving_gltf()
            .replace(r#""name":"body""#, r#""name":"TMP""#)
            .replace(r#""name":"arm""#, r#""name":"body""#)
            .replace(r#""name":"TMP""#, r#""name":"arm""#)
            .replace(r#""name":"wave""#, r#""name":"mixamo.com""#);
        std::fs::write(dir.join("anims/Shrug.gltf"), &swapped).unwrap();
        let two = waving_gltf().replace(
            r#""animations":[{"name":"wave","#,
            r#""animations":[{"name":"Hi","channels":[{"sampler":0,"target":{"node":1,"path":"rotation"}}],
              "samplers":[{"input":2,"output":3,"interpolation":"LINEAR"}]},{"name":"Bye","#,
        );
        std::fs::write(dir.join("anims/greet.gltf"), &two).unwrap();
        std::fs::write(dir.join("anims/broken.gltf"), "not json").unwrap();
        let settings = import::ImportSettings {
            clips: [
                "anims/Shrug.gltf",
                "anims/greet.gltf",
                "anims/broken.gltf",
                "nope.glb",
            ]
            .into_iter()
            .map(import::ClipFile::new)
            .collect(),
            events: vec![
                import::EventMark::new("greet:Bye", 0.75, "wave"),
                import::EventMark::new("wave", 0.25, "lift"),
                import::EventMark::new("Missing", 0.1, "lost"),
            ],
            ..import::ImportSettings::default()
        };
        import::write_settings(&model, &settings).unwrap();

        let (loaded, deps) = load_uncached(&model).unwrap();
        let rig = loaded.rig.as_ref().unwrap();
        // Broken and missing files are skipped with a warning.
        assert_eq!(rig.clip_names(), ["wave", "Shrug", "greet:Hi", "greet:Bye"]);
        let shrug = rig.clip("Shrug").unwrap();
        assert_eq!(shrug.channels.len(), 1);
        assert_eq!(shrug.channels[0].node, rig.node("body").unwrap());
        assert_eq!(
            rig.clip("wave").unwrap().channels[0].node,
            rig.node("arm").unwrap()
        );
        // Events land on clips by exact name, borrowed ones included.
        let events = |clip: &str| -> Vec<(f32, String)> {
            rig.clip(clip)
                .unwrap()
                .events
                .iter()
                .map(|e| (e.time, e.name.clone()))
                .collect()
        };
        assert_eq!(events("greet:Bye"), [(0.75, "wave".to_string())]);
        assert_eq!(events("wave"), [(0.25, "lift".to_string())]);
        assert_eq!(events("greet:Hi"), []);
        // Every clip file is watched, even the broken and missing ones.
        for file in [
            "anims/Shrug.gltf",
            "anims/greet.gltf",
            "anims/broken.gltf",
            "nope.glb",
        ] {
            assert!(deps.contains(&dir.join(file)), "{file} isn't watched");
        }
        let listed = dependencies(&model);
        assert!(listed.contains(&dir.join("anims/greet.gltf")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relative_paths_join_per_component() {
        let base = Path::new("/game");
        assert_eq!(
            join_relative(base, "models\\trees/oak.glb"),
            Path::new("/game/models/trees/oak.glb")
        );
        assert_eq!(join_relative(base, "./a.obj"), Path::new("/game/a.obj"));
    }
}
