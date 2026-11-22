//! glTF 2.0 (`.gltf` + buffers, or binary `.glb`).
//!
//! Supported: the default scene's node tree (matrix or TRS transforms),
//! triangle / strip / fan primitives, `POSITION` / `NORMAL` / `TEXCOORD_n`
//! (float or normalized integer, interleaved or not, sparse accessors),
//! `u8` / `u16` / `u32` indices, buffers in the GLB chunk, beside the file
//! or as `data:` URIs, and metallic-roughness (or the older
//! specular-glossiness) base color factors and textures (PNG / JPEG),
//! roughness / metallic factors and maps, normal maps, and emission
//! (`KHR_materials_emissive_strength` too).
//! Primitives sharing a material are merged into one part.
//!
//! Skins (`JOINTS_0` / `WEIGHTS_0`, inverse bind matrices) and animations
//! (translation / rotation / scale; linear, step and cubic-spline) become
//! the model's [`Rig`]. In a rigged file, meshes that aren't skinned follow
//! their node as a single joint, so node animation (a robot's arm) and
//! skinning share one path.
//!
//! Not supported: Draco / meshopt compression (an error says so), morph
//! targets, vertex colors, occlusion maps, and the other material
//! extensions (clearcoat, transmission, ...).

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use automata_renderer::{MeshData, SkinVertex, Vertex};
use glam::{Mat3, Mat4, Quat, Vec3};

use crate::anim::{
    Channel, Clip, Interpolation, PartSkin, Property, Rig, RigNode, Trs, MAX_JOINTS,
};
use crate::json::Json;
use crate::PartSurface;
use crate::{base64_decode, percent_decode, unweld_flat, Image, Loaded, ModelPart};

const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Splits a GLB into its JSON text and binary chunk.
fn split_glb(bytes: &[u8]) -> Result<(&str, Option<&[u8]>), String> {
    let version = u32_at(bytes, 4).ok_or("truncated GLB header")?;
    if version != 2 {
        return Err(format!(
            "GLB version {version} isn't supported; re-export as glTF 2.0"
        ));
    }
    let total = (u32_at(bytes, 8).ok_or("truncated GLB header")? as usize).min(bytes.len());
    let mut at = 12;
    let mut json = None;
    let mut bin = None;
    while at + 8 <= total {
        let len = u32_at(bytes, at).ok_or("truncated GLB chunk")? as usize;
        let kind = u32_at(bytes, at + 4).ok_or("truncated GLB chunk")?;
        let data = bytes
            .get(at + 8..at + 8 + len)
            .ok_or("a GLB chunk runs past the end of the file")?;
        match kind {
            CHUNK_JSON if json.is_none() => {
                json = Some(std::str::from_utf8(data).map_err(|_| "the GLB JSON isn't UTF-8")?);
            }
            CHUNK_BIN if bin.is_none() => bin = Some(data),
            _ => {}
        }
        at += 8 + len.next_multiple_of(4);
    }
    Ok((json.ok_or("the GLB has no JSON chunk")?, bin))
}

fn json_of(bytes: &[u8]) -> Result<(Json, Option<&[u8]>), String> {
    let (text, bin) = if bytes.starts_with(GLB_MAGIC) {
        split_glb(bytes)?
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| "the glTF JSON isn't UTF-8")?;
        (text.trim_start_matches('\u{feff}'), None)
    };
    Ok((
        Json::parse(text).map_err(|e| format!("invalid JSON: {e}"))?,
        bin,
    ))
}

/// An accessor's `componentType` (0 when missing or negative).
fn ctype_of(j: Option<&Json>, default: u64) -> u64 {
    j.and_then(Json::as_i64)
        .and_then(|v| u64::try_from(v).ok())
        .unwrap_or(default)
}

fn usize_of(j: Option<&Json>) -> Option<usize> {
    j.and_then(Json::as_i64)
        .and_then(|v| usize::try_from(v).ok())
}

fn floats<const N: usize>(j: Option<&Json>, default: [f32; N]) -> [f32; N] {
    j.and_then(Json::as_floats::<N>).unwrap_or(default)
}

/// A single number (glTF scalars such as `roughnessFactor`).
#[allow(clippy::cast_possible_truncation)]
fn number(j: Option<&Json>, default: f32) -> f32 {
    j.and_then(Json::as_f64).map_or(default, |v| v as f32)
}

fn arr<'j>(j: &'j Json, key: &str) -> &'j [Json] {
    j.get(key).and_then(Json::as_arr).unwrap_or(&[])
}

/// Every external file the glTF references (buffers and images that are not
/// embedded), resolved next to the file.
pub(crate) fn references(path: &Path, bytes: &[u8]) -> Vec<PathBuf> {
    let Ok((doc, _)) = json_of(bytes) else {
        return Vec::new();
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out: Vec<PathBuf> = Vec::new();
    for item in arr(&doc, "buffers").iter().chain(arr(&doc, "images")) {
        if let Some(uri) = item.get("uri").and_then(Json::as_str) {
            if !uri.starts_with("data:") {
                let p = crate::join_relative(dir, &percent_decode(uri));
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

struct Doc<'a> {
    json: Json,
    dir: PathBuf,
    buffers: Vec<Cow<'a, [u8]>>,
    /// Decoded images by index; `Tried(None)` remembers a failed decode.
    images: Vec<ImageSlot>,
    deps: Vec<PathBuf>,
}

/// An image's decode state.
#[derive(Clone)]
enum ImageSlot {
    Untried,
    Tried(Option<Arc<Image>>),
}

/// A typed view into a buffer.
struct View<'b> {
    data: &'b [u8],
    offset: usize,
    stride: usize,
    count: usize,
    comps: usize,
    ctype: u64,
    normalized: bool,
}

fn comp_size(ctype: u64) -> Result<usize, String> {
    Ok(match ctype {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        other => return Err(format!("unknown accessor component type {other}")),
    })
}

fn comp_count(kind: &str) -> Result<usize, String> {
    Ok(match kind {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" | "MAT2" => 4,
        "MAT3" => 9,
        "MAT4" => 16,
        other => return Err(format!("unknown accessor type {other:?}")),
    })
}

impl View<'_> {
    fn read(&self, element: usize, comp: usize) -> f32 {
        let size = comp_size(self.ctype).unwrap_or(4);
        let at = self.offset + element * self.stride + comp * size;
        let b = &self.data[at..at + size];
        let n = self.normalized;
        match self.ctype {
            5120 => {
                let v = f32::from(b[0].cast_signed_compat());
                if n {
                    (v / 127.0).max(-1.0)
                } else {
                    v
                }
            }
            5121 => {
                let v = f32::from(b[0]);
                if n {
                    v / 255.0
                } else {
                    v
                }
            }
            5122 => {
                let v = f32::from(i16::from_le_bytes([b[0], b[1]]));
                if n {
                    (v / 32767.0).max(-1.0)
                } else {
                    v
                }
            }
            5123 => {
                let v = f32::from(u16::from_le_bytes([b[0], b[1]]));
                if n {
                    v / 65535.0
                } else {
                    v
                }
            }
            5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
            _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        }
    }

    fn read_u32(&self, element: usize) -> u32 {
        let size = comp_size(self.ctype).unwrap_or(4);
        let at = self.offset + element * self.stride;
        let b = &self.data[at..at + size];
        match self.ctype {
            5121 | 5120 => u32::from(b[0]),
            5123 | 5122 => u32::from(u16::from_le_bytes([b[0], b[1]])),
            _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        }
    }
}

/// `i8` from a byte (MSRV-friendly `cast_signed`).
trait CastSignedCompat {
    fn cast_signed_compat(self) -> i8;
}

impl CastSignedCompat for u8 {
    fn cast_signed_compat(self) -> i8 {
        i8::from_le_bytes([self])
    }
}

impl<'a> Doc<'a> {
    fn new(path: &Path, bytes: &'a [u8]) -> Result<Self, String> {
        let (json, bin) = json_of(bytes)?;
        if let Some(v) = json
            .get("asset")
            .and_then(|a| a.get("version"))
            .and_then(Json::as_str)
        {
            if !v.starts_with('2') {
                return Err(format!("glTF {v} isn't supported; re-export as glTF 2.0"));
            }
        }
        for ext in arr(&json, "extensionsRequired") {
            match ext.as_str() {
                Some("KHR_draco_mesh_compression" | "EXT_meshopt_compression") => {
                    return Err(format!(
                        "the file uses {} mesh compression, which isn't supported; export it again without compression",
                        ext.as_str().unwrap_or_default()
                    ));
                }
                Some(other) => tracing::debug!("glTF extension {other} is ignored"),
                None => {}
            }
        }
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut deps = Vec::new();
        let mut buffers = Vec::new();
        for (i, buffer) in arr(&json, "buffers").iter().enumerate() {
            let data: Cow<'a, [u8]> = match buffer.get("uri").and_then(Json::as_str) {
                None => Cow::Borrowed(bin.ok_or(format!(
                    "buffer {i} has no URI and there is no GLB binary chunk"
                ))?),
                Some(uri) if uri.starts_with("data:") => Cow::Owned(data_uri(uri)?),
                Some(uri) => {
                    let file = crate::join_relative(&dir, &percent_decode(uri));
                    let data = std::fs::read(&file)
                        .map_err(|e| format!("buffer {}: {e}", file.display()))?;
                    deps.push(file);
                    Cow::Owned(data)
                }
            };
            buffers.push(data);
        }
        let images = vec![ImageSlot::Untried; arr(&json, "images").len()];
        Ok(Self {
            json,
            dir,
            buffers,
            images,
            deps,
        })
    }

    /// The bytes of buffer view `index`, plus its stride.
    fn buffer_view(&self, index: usize) -> Result<(&[u8], Option<usize>), String> {
        let view = arr(&self.json, "bufferViews")
            .get(index)
            .ok_or(format!("buffer view {index} doesn't exist"))?;
        let buffer = usize_of(view.get("buffer")).ok_or("buffer view without a buffer")?;
        let data = self
            .buffers
            .get(buffer)
            .ok_or(format!("buffer {buffer} doesn't exist"))?;
        let offset = usize_of(view.get("byteOffset")).unwrap_or(0);
        let len = usize_of(view.get("byteLength")).ok_or("buffer view without a length")?;
        let bytes = data
            .get(offset..offset + len)
            .ok_or(format!("buffer view {index} runs past its buffer"))?;
        Ok((bytes, usize_of(view.get("byteStride")).filter(|s| *s > 0)))
    }

    /// Reads accessor `index` as floats (`count × comps`, row by row).
    fn floats(&self, index: usize) -> Result<(Vec<f32>, usize), String> {
        let acc = arr(&self.json, "accessors")
            .get(index)
            .ok_or(format!("accessor {index} doesn't exist"))?;
        let count = usize_of(acc.get("count")).ok_or("accessor without a count")?;
        let comps = comp_count(acc.get("type").and_then(Json::as_str).unwrap_or(""))?;
        let ctype = ctype_of(acc.get("componentType"), 0);
        let normalized = acc
            .get("normalized")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        let mut out = vec![0.0; count * comps];
        if let Some(view_index) = usize_of(acc.get("bufferView")) {
            let view = self.view(
                view_index,
                usize_of(acc.get("byteOffset")).unwrap_or(0),
                count,
                comps,
                ctype,
                normalized,
            )?;
            for e in 0..count {
                for c in 0..comps {
                    out[e * comps + c] = view.read(e, c);
                }
            }
        }
        if let Some(sparse) = acc.get("sparse") {
            let n = usize_of(sparse.get("count")).unwrap_or(0);
            let idx = sparse
                .get("indices")
                .ok_or("sparse accessor without indices")?;
            let vals = sparse
                .get("values")
                .ok_or("sparse accessor without values")?;
            let idx_view = self.view(
                usize_of(idx.get("bufferView")).ok_or("sparse indices without a view")?,
                usize_of(idx.get("byteOffset")).unwrap_or(0),
                n,
                1,
                ctype_of(idx.get("componentType"), 5125),
                false,
            )?;
            let val_view = self.view(
                usize_of(vals.get("bufferView")).ok_or("sparse values without a view")?,
                usize_of(vals.get("byteOffset")).unwrap_or(0),
                n,
                comps,
                ctype,
                normalized,
            )?;
            for s in 0..n {
                let e = idx_view.read_u32(s) as usize;
                if e < count {
                    for c in 0..comps {
                        out[e * comps + c] = val_view.read(s, c);
                    }
                }
            }
        }
        Ok((out, comps))
    }

    fn view(
        &self,
        view_index: usize,
        offset: usize,
        count: usize,
        comps: usize,
        ctype: u64,
        normalized: bool,
    ) -> Result<View<'_>, String> {
        let (data, stride) = self.buffer_view(view_index)?;
        let elem = comps * comp_size(ctype)?;
        let stride = stride.unwrap_or(elem);
        if count > 0 && offset + (count - 1) * stride + elem > data.len() {
            return Err(format!("an accessor runs past buffer view {view_index}"));
        }
        Ok(View {
            data,
            offset,
            stride,
            count,
            comps,
            ctype,
            normalized,
        })
    }

    fn indices(&self, index: usize) -> Result<Vec<u32>, String> {
        let acc = arr(&self.json, "accessors")
            .get(index)
            .ok_or(format!("accessor {index} doesn't exist"))?;
        let ctype = ctype_of(acc.get("componentType"), 0);
        if !matches!(ctype, 5121 | 5123 | 5125) {
            // Unusual but legal-looking: read through the float path.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            return Ok(self
                .floats(index)?
                .0
                .iter()
                .map(|v| v.max(0.0) as u32)
                .collect());
        }
        let count = usize_of(acc.get("count")).ok_or("accessor without a count")?;
        let Some(view_index) = usize_of(acc.get("bufferView")) else {
            return Ok(vec![0; count]);
        };
        let view = self.view(
            view_index,
            usize_of(acc.get("byteOffset")).unwrap_or(0),
            count,
            1,
            ctype,
            false,
        )?;
        debug_assert_eq!(view.comps, 1);
        Ok((0..view.count).map(|i| view.read_u32(i)).collect())
    }

    /// Image `index`, decoded once (failures are logged and drawn untextured).
    fn image(&mut self, index: usize) -> Option<Arc<Image>> {
        if let Some(ImageSlot::Tried(cached)) = self.images.get(index) {
            return cached.clone();
        }
        let decoded = self.decode_image(index).map_or_else(
            |e| {
                tracing::warn!("texture {index} not loaded: {e}");
                None
            },
            |img| Some(Arc::new(img)),
        );
        if let Some(slot) = self.images.get_mut(index) {
            *slot = ImageSlot::Tried(decoded.clone());
        }
        decoded
    }

    fn decode_image(&mut self, index: usize) -> Result<Image, String> {
        let img = arr(&self.json, "images")
            .get(index)
            .ok_or("image doesn't exist")?
            .clone();
        if let Some(uri) = img.get("uri").and_then(Json::as_str) {
            if uri.starts_with("data:") {
                return Image::decode(&data_uri(uri)?);
            }
            let file = crate::join_relative(&self.dir, &percent_decode(uri));
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            self.deps.push(file);
            return Image::decode(&bytes);
        }
        let view = usize_of(img.get("bufferView")).ok_or("image without a URI or buffer view")?;
        let (bytes, _) = self.buffer_view(view)?;
        Image::decode(bytes)
    }

    /// What a part needs from material `index`.
    fn material(&mut self, index: Option<usize>) -> Material {
        let Some(mat) = index
            .and_then(|i| arr(&self.json, "materials").get(i))
            .cloned()
        else {
            return Material {
                name: "Default".into(),
                color: [1.0; 4],
                texture: None,
                texcoord: 0,
                uv_transform: None,
                pixelated: false,
                surface: PartSurface::default(),
            };
        };
        let name = mat.get("name").and_then(Json::as_str).map_or_else(
            || format!("Material {}", index.unwrap_or(0)),
            str::to_string,
        );
        let pbr = mat.get("pbrMetallicRoughness");
        let spec_gloss = mat
            .get("extensions")
            .and_then(|e| e.get("KHR_materials_pbrSpecularGlossiness"));
        let (color, tex) = match (pbr, spec_gloss) {
            (_, Some(sg)) => (
                floats(sg.get("diffuseFactor"), [1.0; 4]),
                sg.get("diffuseTexture"),
            ),
            (Some(pbr), None) => (
                floats(pbr.get("baseColorFactor"), [1.0; 4]),
                pbr.get("baseColorTexture"),
            ),
            (None, None) => ([1.0; 4], None),
        };
        let (uv_transform, texcoord_override) = uv_transform(tex);
        let texcoord = texcoord_override
            .or_else(|| usize_of(tex.and_then(|t| t.get("texCoord"))))
            .unwrap_or(0);
        let texture_def = usize_of(tex.and_then(|t| t.get("index")))
            .and_then(|t| arr(&self.json, "textures").get(t))
            .cloned();
        let source = texture_def.as_ref().and_then(|t| usize_of(t.get("source")));
        // magFilter 9728 = NEAREST.
        let pixelated = texture_def
            .as_ref()
            .and_then(|t| usize_of(t.get("sampler")))
            .and_then(|s| arr(&self.json, "samplers").get(s))
            .and_then(|s| s.get("magFilter"))
            .and_then(Json::as_i64)
            == Some(9728);
        let texture = source.and_then(|s| self.image(s));
        let surface = self.surface(&mat, pbr, spec_gloss);
        Material {
            name,
            color,
            texture,
            texcoord,
            uv_transform,
            pixelated,
            surface,
        }
    }

    /// The image a texture reference (`{"index": n, ...}`) points at.
    fn texture_image(&mut self, tex: Option<&Json>) -> Option<Arc<Image>> {
        let source = usize_of(tex?.get("index"))
            .and_then(|t| arr(&self.json, "textures").get(t))
            .and_then(|t| usize_of(t.get("source")))?;
        self.image(source)
    }

    /// Roughness, metalness, emission and the PBR maps of a material. The
    /// glTF defaults apply (a material without factors is rough metal);
    /// specular-glossiness materials map glossiness to roughness.
    fn surface(
        &mut self,
        mat: &Json,
        pbr: Option<&Json>,
        spec_gloss: Option<&Json>,
    ) -> PartSurface {
        let (roughness, metallic, metallic_roughness_map) = match (pbr, spec_gloss) {
            (_, Some(sg)) => (1.0 - number(sg.get("glossinessFactor"), 1.0), 0.0, None),
            (Some(pbr), None) => (
                number(pbr.get("roughnessFactor"), 1.0),
                number(pbr.get("metallicFactor"), 1.0),
                self.texture_image(pbr.get("metallicRoughnessTexture")),
            ),
            (None, None) => (1.0, 1.0, None),
        };
        let normal = mat.get("normalTexture");
        let normal_scale = number(normal.and_then(|n| n.get("scale")), 1.0);
        let normal_map = self.texture_image(normal);
        let strength = number(
            mat.get("extensions")
                .and_then(|e| e.get("KHR_materials_emissive_strength"))
                .and_then(|e| e.get("emissiveStrength")),
            1.0,
        );
        let emissive = floats(mat.get("emissiveFactor"), [0.0; 3]).map(|c| c * strength);
        let emissive_map = if emissive.iter().all(|c| *c <= 0.0) {
            None // a map times a zero factor is no glow
        } else {
            self.texture_image(mat.get("emissiveTexture"))
        };
        let emissive_coverage = emissive_map.as_ref().map_or(1.0, |m| m.mean_linear());
        PartSurface {
            roughness: roughness.clamp(0.0, 1.0),
            metallic: metallic.clamp(0.0, 1.0),
            emissive,
            normal_scale,
            normal_map,
            metallic_roughness_map,
            emissive_map,
            emissive_coverage,
        }
    }
}

fn data_uri(uri: &str) -> Result<Vec<u8>, String> {
    let (head, data) = uri.split_once(',').ok_or("malformed data URI")?;
    if head.ends_with(";base64") {
        base64_decode(data)
    } else {
        Ok(percent_decode(data).into_bytes())
    }
}

fn node_matrix(node: &Json) -> Mat4 {
    if let Some(m) = node.get("matrix").and_then(Json::as_floats::<16>) {
        return Mat4::from_cols_array(&m);
    }
    let t = floats(node.get("translation"), [0.0; 3]);
    let r = floats(node.get("rotation"), [0.0, 0.0, 0.0, 1.0]);
    let s = floats(node.get("scale"), [1.0; 3]);
    let q = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
    let q = if q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    Mat4::from_scale_rotation_translation(Vec3::from(s), q, Vec3::from(t))
}

/// Geometry gathered for one material.
struct PartBuilder {
    key: Option<usize>,
    name: String,
    color: [f32; 4],
    texture: Option<Arc<Image>>,
    pixelated: bool,
    surface: PartSurface,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    /// Joints and weights per vertex (rigged files only).
    skin: Vec<SkinVertex>,
}

/// What a part needs from its glTF material.
struct Material {
    name: String,
    color: [f32; 4],
    texture: Option<Arc<Image>>,
    /// The `TEXCOORD_n` set the color texture reads.
    texcoord: usize,
    /// `KHR_texture_transform`, as a 2×3 matrix applied to the UVs.
    uv_transform: Option<[f32; 6]>,
    /// The sampler magnifies with nearest filtering.
    pixelated: bool,
    surface: PartSurface,
}

/// The `KHR_texture_transform` of a texture reference, as a row-major 2×3
/// matrix `[a, b, tx, c, d, ty]` (u' = a·u + b·v + tx, v' = c·u + d·v + ty),
/// or `None` when absent or the identity. Same convention as three.js.
fn uv_transform(tex: Option<&Json>) -> (Option<[f32; 6]>, Option<usize>) {
    let Some(ext) = tex
        .and_then(|t| t.get("extensions"))
        .and_then(|e| e.get("KHR_texture_transform"))
    else {
        return (None, None);
    };
    let [ox, oy] = floats(ext.get("offset"), [0.0, 0.0]);
    let [sx, sy] = floats(ext.get("scale"), [1.0, 1.0]);
    let angle = number(ext.get("rotation"), 0.0);
    let (sin, cos) = angle.sin_cos();
    let matrix = [sx * cos, sx * sin, ox, -sy * sin, sy * cos, oy];
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let changed = matrix
        .iter()
        .zip(identity)
        .any(|(a, b)| (a - b).abs() > 1e-6);
    (changed.then_some(matrix), usize_of(ext.get("texCoord")))
}

/// Triangle-list indices for a primitive `mode`, or `None` for points/lines.
fn triangles(mode: i64, idx: &[u32]) -> Option<Vec<u32>> {
    match mode {
        4 => Some(idx[..idx.len() - idx.len() % 3].to_vec()),
        5 => Some(
            (2..idx.len())
                .flat_map(|i| {
                    if i % 2 == 0 {
                        [idx[i - 2], idx[i - 1], idx[i]]
                    } else {
                        [idx[i - 1], idx[i - 2], idx[i]]
                    }
                })
                .collect(),
        ),
        6 => Some(
            (2..idx.len())
                .flat_map(|i| [idx[0], idx[i - 1], idx[i]])
                .collect(),
        ),
        _ => None,
    }
}

/// A node's local transform as translation, rotation and scale.
fn node_trs(node: &Json) -> Trs {
    if node.get("matrix").is_some() {
        return Trs::from_matrix(node_matrix(node));
    }
    let t = floats(node.get("translation"), [0.0; 3]);
    let r = floats(node.get("rotation"), [0.0, 0.0, 0.0, 1.0]);
    let s = floats(node.get("scale"), [1.0; 3]);
    let q = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
    Trs {
        translation: Vec3::from(t),
        rotation: if q.length_squared() > 1e-12 {
            q.normalize()
        } else {
            Quat::IDENTITY
        },
        scale: Vec3::from(s),
    }
}

/// A node of the scene tree, in traversal order (parents first).
struct TreeNode {
    /// The node's index in the file.
    index: usize,
    /// The parent's position in the traversal.
    parent: Option<usize>,
    /// The rest-pose transform relative to the model.
    world: Mat4,
}

/// Walks the default scene's node tree depth first.
fn scene_tree(json: &Json, nodes: &[Json]) -> Result<Vec<TreeNode>, String> {
    // Root nodes: the default scene's, else every node nobody parents.
    let scene = usize_of(json.get("scene")).unwrap_or(0);
    let roots: Vec<usize> = if let Some(s) = arr(json, "scenes").get(scene) {
        arr(s, "nodes")
            .iter()
            .filter_map(|n| usize_of(Some(n)))
            .collect()
    } else {
        let children: Vec<usize> = nodes
            .iter()
            .flat_map(|n| arr(n, "children").iter().filter_map(|c| usize_of(Some(c))))
            .collect();
        (0..nodes.len()).filter(|i| !children.contains(i)).collect()
    };
    let mut tree = Vec::new();
    let mut stack: Vec<(usize, Option<usize>, usize)> =
        roots.iter().rev().map(|&r| (r, None, 0)).collect();
    while let Some((index, parent, depth)) = stack.pop() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        if depth > 64 {
            return Err("the node tree is too deep (or has a cycle)".into());
        }
        let parent_world = parent.map_or(Mat4::IDENTITY, |p: usize| tree_world(&tree, p));
        let at = tree.len();
        tree.push(TreeNode {
            index,
            parent,
            world: parent_world * node_matrix(node),
        });
        for child in arr(node, "children").iter().rev() {
            if let Some(c) = usize_of(Some(child)) {
                stack.push((c, Some(at), depth + 1));
            }
        }
    }
    Ok(tree)
}

fn tree_world(tree: &[TreeNode], at: usize) -> Mat4 {
    tree.get(at).map_or(Mat4::IDENTITY, |n| n.world)
}

/// The palette index of `(node, inverse_bind)`, adding it when new.
fn palette_joint(joints: &mut Vec<(usize, Mat4)>, node: usize, inverse_bind: Mat4) -> u16 {
    let at = joints
        .iter()
        .position(|(n, m)| *n == node && m.abs_diff_eq(inverse_bind, 1e-6))
        .unwrap_or_else(|| {
            joints.push((node, inverse_bind));
            joints.len() - 1
        });
    u16::try_from(at).unwrap_or(u16::MAX)
}

/// A skin's joints as palette indices.
fn skin_palette(
    doc: &Doc<'_>,
    skin: &Json,
    tree_of: &dyn Fn(usize) -> Option<usize>,
    fallback: usize,
    joints: &mut Vec<(usize, Mat4)>,
) -> Result<Vec<u16>, String> {
    let ibms = match usize_of(skin.get("inverseBindMatrices")) {
        Some(acc) => {
            let (m, comps) = doc.floats(acc)?;
            if comps != 16 {
                return Err("inverseBindMatrices must be MAT4".into());
            }
            m
        }
        None => Vec::new(),
    };
    Ok(arr(skin, "joints")
        .iter()
        .enumerate()
        .map(|(i, j)| {
            let node = usize_of(Some(j)).and_then(tree_of).unwrap_or(fallback);
            let ibm = ibms
                .get(i * 16..i * 16 + 16)
                .map_or(Mat4::IDENTITY, Mat4::from_cols_slice);
            palette_joint(joints, node, ibm)
        })
        .collect())
}

/// The file's animations, targeting nodes by traversal position.
fn read_clips(doc: &Doc<'_>, tree_of: &dyn Fn(usize) -> Option<usize>) -> Vec<Clip> {
    let mut clips = Vec::new();
    for (i, anim) in arr(&doc.json, "animations").iter().enumerate() {
        let samplers = arr(anim, "samplers");
        let mut channels = Vec::new();
        for ch in arr(anim, "channels") {
            let target = ch.get("target");
            let Some(node) = usize_of(target.and_then(|t| t.get("node"))).and_then(tree_of) else {
                continue;
            };
            let property = match target.and_then(|t| t.get("path")).and_then(Json::as_str) {
                Some("translation") => Property::Translation,
                Some("rotation") => Property::Rotation,
                Some("scale") => Property::Scale,
                _ => continue, // morph target weights aren't supported
            };
            let Some(sampler) = usize_of(ch.get("sampler")).and_then(|s| samplers.get(s)) else {
                continue;
            };
            let interpolation = match sampler.get("interpolation").and_then(Json::as_str) {
                Some("STEP") => Interpolation::Step,
                Some("CUBICSPLINE") => Interpolation::CubicSpline,
                _ => Interpolation::Linear,
            };
            let read = |key: &str| usize_of(sampler.get(key)).and_then(|a| doc.floats(a).ok());
            let (Some((times, 1)), Some((values, comps))) = (read("input"), read("output")) else {
                continue;
            };
            let width = if property == Property::Rotation { 4 } else { 3 };
            let per_key = if interpolation == Interpolation::CubicSpline {
                3
            } else {
                1
            };
            if comps != width || values.len() != times.len() * width * per_key || times.is_empty() {
                continue;
            }
            channels.push(Channel {
                node,
                property,
                interpolation,
                times,
                values,
            });
        }
        if channels.is_empty() {
            continue;
        }
        let duration = channels
            .iter()
            .filter_map(|c| c.times.last().copied())
            .fold(0.0_f32, f32::max);
        let name = anim
            .get("name")
            .and_then(Json::as_str)
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("Animation {i}"), str::to_string);
        clips.push(Clip {
            name,
            duration,
            channels,
        });
    }
    clips
}

/// Per-vertex joints and weights from `JOINTS_0` / `WEIGHTS_0`, mapped
/// through the skin's palette; `None` when the attributes are missing.
fn read_skin(
    doc: &Doc<'_>,
    attrs: Option<&Json>,
    palette: &[u16],
    count: usize,
) -> Result<Option<Vec<SkinVertex>>, String> {
    let get = |name: &str| usize_of(attrs.and_then(|a| a.get(name)));
    let (Some(j_acc), Some(w_acc)) = (get("JOINTS_0"), get("WEIGHTS_0")) else {
        return Ok(None);
    };
    let (joints, jc) = doc.floats(j_acc)?;
    let (weights, wc) = doc.floats(w_acc)?;
    if jc != 4 || wc != 4 || joints.len() < count * 4 || weights.len() < count * 4 {
        return Err("JOINTS_0 and WEIGHTS_0 must be VEC4 for every vertex".into());
    }
    Ok(Some(
        (0..count)
            .map(|i| {
                let mut skin = SkinVertex::default();
                let mut total = 0.0;
                for c in 0..4 {
                    let w = weights[i * 4 + c].max(0.0);
                    // Joint ids are small integers stored exactly as floats.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let local = joints[i * 4 + c] as usize;
                    skin.joints[c] = palette.get(local).copied().unwrap_or(0);
                    skin.weights[c] = w;
                    total += w;
                }
                if total > 1e-6 {
                    skin.weights = skin.weights.map(|w| w / total);
                } else {
                    skin.weights = [1.0, 0.0, 0.0, 0.0];
                }
                skin
            })
            .collect(),
    ))
}

#[allow(clippy::too_many_lines)]
pub(crate) fn load(path: &Path, bytes: &[u8]) -> Result<Loaded, String> {
    let mut doc = Doc::new(path, bytes)?;
    let nodes = arr(&doc.json, "nodes").to_vec();
    let meshes = arr(&doc.json, "meshes").to_vec();
    let skins = arr(&doc.json, "skins").to_vec();
    let tree = scene_tree(&doc.json, &nodes)?;
    // A node's traversal position (its first, if it's instanced twice).
    let tree_of = |index: usize| tree.iter().position(|n| n.index == index);

    // Animated or skinned files keep a rig: the geometry stays in bind
    // space and follows joints. Others are baked into model space.
    let clips = read_clips(&doc, &tree_of);
    let rigged = !clips.is_empty() || !skins.is_empty();
    let mut joints: Vec<(usize, Mat4)> = Vec::new();
    let mut skin_palettes: Vec<Option<Vec<u16>>> = vec![None; skins.len()];

    // (mesh index, traversal position) for every mesh instance.
    let mut instances: Vec<(usize, Option<usize>)> = tree
        .iter()
        .enumerate()
        .filter_map(|(at, n)| usize_of(nodes[n.index].get("mesh")).map(|m| (m, Some(at))))
        .collect();
    if nodes.is_empty() {
        instances = (0..meshes.len()).map(|m| (m, None)).collect();
    }

    let mut parts: Vec<PartBuilder> = Vec::new();
    for (mesh_index, at) in instances {
        let Some(mesh) = meshes.get(mesh_index) else {
            continue;
        };
        let world = at.map_or(Mat4::IDENTITY, |at| tree_world(&tree, at));
        // A glTF skin replaces the node's transform with its joints.
        let skin_index = at
            .and_then(|at| usize_of(nodes[tree[at].index].get("skin")))
            .filter(|s| *s < skins.len() && rigged);
        let palette = match skin_index {
            Some(s) => {
                if skin_palettes[s].is_none() {
                    let p = skin_palette(&doc, &skins[s], &tree_of, at.unwrap_or(0), &mut joints)?;
                    skin_palettes[s] = Some(p);
                }
                skin_palettes[s].clone()
            }
            None => None,
        };
        for prim in arr(mesh, "primitives") {
            let attrs = prim.get("attributes");
            let Some(pos_acc) = usize_of(attrs.and_then(|a| a.get("POSITION"))) else {
                continue;
            };
            let mode = prim.get("mode").and_then(Json::as_i64).unwrap_or(4);
            let material = usize_of(prim.get("material"));
            let (positions, pc) = doc.floats(pos_acc)?;
            if pc < 3 {
                return Err("POSITION must be a VEC3".into());
            }
            let count = positions.len() / pc;
            let normals = match usize_of(attrs.and_then(|a| a.get("NORMAL"))) {
                Some(acc) => Some(doc.floats(acc)?),
                None => None,
            };
            let skinned = match &palette {
                Some(p) => read_skin(&doc, attrs, p, count)?,
                None => None,
            };
            // Skinned vertices are already in bind space.
            let place = if skinned.is_some() {
                Mat4::IDENTITY
            } else {
                world
            };
            let normal_matrix = Mat3::from_mat4(place).inverse().transpose();
            let flip = place.determinant() < 0.0;

            // Find (or start) the part for this material.
            let mat = doc.material(material);
            let slot = if let Some(i) = parts.iter().position(|p| p.key == material) {
                i
            } else {
                parts.push(PartBuilder {
                    key: material,
                    name: mat.name.clone(),
                    color: mat.color,
                    texture: mat.texture.clone(),
                    pixelated: mat.pixelated,
                    surface: mat.surface.clone(),
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    skin: Vec::new(),
                });
                parts.len() - 1
            };
            let texcoord = mat.texcoord;
            let uvs = match usize_of(attrs.and_then(|a| a.get(&format!("TEXCOORD_{texcoord}")))) {
                Some(acc) => Some(doc.floats(acc)?),
                None => None,
            };

            let raw: Vec<u32> = match usize_of(prim.get("indices")) {
                Some(acc) => doc.indices(acc)?,
                None => (0..u32::try_from(count).map_err(|_| "too many vertices")?).collect(),
            };
            let Some(mut tris) = triangles(mode, &raw) else {
                continue; // points and lines aren't drawn
            };
            tris.retain(|&i| (i as usize) < count);
            tris.truncate(tris.len() - tris.len() % 3);
            if flip {
                for t in tris.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }

            let mut verts = Vec::with_capacity(count);
            for i in 0..count {
                let p = place.transform_point3(Vec3::new(
                    positions[i * pc],
                    positions[i * pc + 1],
                    positions[i * pc + 2],
                ));
                let n = normals
                    .as_ref()
                    .filter(|(_, c)| *c >= 3)
                    .map_or(Vec3::Y, |(n, c)| {
                        (normal_matrix * Vec3::new(n[i * c], n[i * c + 1], n[i * c + 2]))
                            .normalize_or(Vec3::Y)
                    });
                let mut uv = uvs
                    .as_ref()
                    .filter(|(_, c)| *c >= 2)
                    .map_or([0.0, 0.0], |(t, c)| [t[i * c], t[i * c + 1]]);
                if let Some(m) = mat.uv_transform {
                    uv = [
                        m[0] * uv[0] + m[1] * uv[1] + m[2],
                        m[3] * uv[0] + m[4] * uv[1] + m[5],
                    ];
                }
                verts.push(Vertex::new(p.into(), n.into(), uv));
            }
            let skin = skinned.unwrap_or_else(|| {
                // Everything that isn't skinned follows its node rigidly.
                let rigid_joint = match at {
                    Some(at) if rigged => palette_joint(&mut joints, at, world.inverse()),
                    _ => 0,
                };
                let rigid = SkinVertex {
                    joints: [rigid_joint, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                };
                vec![rigid; count]
            });
            let (mesh, skin) = if normals.is_some() {
                (MeshData::new(verts, tris), skin)
            } else {
                // Flat normals give every triangle its own vertices.
                let unwelded: Vec<SkinVertex> = tris.iter().map(|&i| skin[i as usize]).collect();
                (unweld_flat(&verts, &tris), unwelded)
            };
            let part = &mut parts[slot];
            let base = u32::try_from(part.vertices.len()).map_err(|_| "too many vertices")?;
            part.vertices.extend(mesh.vertices);
            part.indices.extend(mesh.indices.iter().map(|i| i + base));
            if rigged {
                part.skin.extend(skin);
            }
        }
    }

    let rig = rigged.then(|| Rig {
        nodes: tree
            .iter()
            .map(|n| RigNode {
                name: nodes[n.index]
                    .get("name")
                    .and_then(Json::as_str)
                    .filter(|s| !s.is_empty())
                    .map_or_else(|| format!("Node {}", n.index), str::to_string),
                parent: n.parent,
                rest: node_trs(&nodes[n.index]),
            })
            .collect(),
        joints,
        import: Mat4::IDENTITY,
        clips,
    });
    // Parts draw their rest pose until something animates them; a rig with
    // too many joints for the GPU keeps only that.
    let rest_palette = rig.as_ref().map(Rig::rest_palette);
    let rig = rig.filter(|r| {
        let fits = r.joints.len() <= MAX_JOINTS;
        if !fits {
            tracing::warn!(
                "{}: {} joints is more than the {MAX_JOINTS} supported; it is drawn in its rest pose",
                path.display(),
                r.joints.len()
            );
        }
        fits
    });

    Ok(Loaded {
        parts: parts
            .into_iter()
            .map(|p| {
                let (vertices, skin) = match &rest_palette {
                    Some(palette) => {
                        let skin = PartSkin {
                            bind: p.vertices,
                            skin: p.skin,
                        };
                        (skin.posed(palette), rig.is_some().then_some(skin))
                    }
                    None => (p.vertices, None),
                };
                ModelPart {
                    name: p.name,
                    mesh: MeshData::new(vertices, p.indices),
                    color: p.color,
                    texture: p.texture,
                    pixelated: p.pixelated,
                    skin,
                    surface: p.surface,
                }
            })
            .collect(),
        rig,
        deps: doc.deps,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A GLB with one quad (two triangles) per entry of `nodes`, each placed
    /// by its translation, all using one red material. `normals` adds a
    /// NORMAL accessor.
    pub(crate) fn quad_glb(nodes: &[[f32; 3]], normals: bool) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        for p in [
            [-0.5_f32, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ] {
            for v in p {
                bin.extend(v.to_le_bytes());
            }
        }
        for _ in 0..4 {
            for v in [0.0_f32, 0.0, 1.0] {
                bin.extend(v.to_le_bytes());
            }
        }
        for i in [0_u16, 1, 2, 0, 2, 3] {
            bin.extend(i.to_le_bytes());
        }
        let node_json: Vec<String> = nodes
            .iter()
            .map(|t| format!(r#"{{"mesh":0,"translation":[{},{},{}]}}"#, t[0], t[1], t[2]))
            .collect();
        let node_ids: Vec<String> = (0..nodes.len()).map(|i| i.to_string()).collect();
        let normal_attr = if normals { r#","NORMAL":1"# } else { "" };
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[{ids}]}}],
            "nodes":[{nodes}],
            "meshes":[{{"name":"Quad","primitives":[{{"attributes":{{"POSITION":0{normal_attr}}},"indices":2,"material":0}}]}}],
            "materials":[{{"name":"Red","pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}}}}],
            "buffers":[{{"byteLength":{len}}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":48}},{{"buffer":0,"byteOffset":96,"byteLength":12}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[-0.5,-0.5,0],"max":[0.5,0.5,0]}},
              {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},
              {{"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
            ids = node_ids.join(","),
            nodes = node_json.join(","),
            len = bin.len(),
        );
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2_u32.to_le_bytes());
        out.extend(u32::try_from(total).unwrap().to_le_bytes());
        out.extend(u32::try_from(json.len()).unwrap().to_le_bytes());
        out.extend(CHUNK_JSON.to_le_bytes());
        out.extend(&json);
        out.extend(u32::try_from(bin.len()).unwrap().to_le_bytes());
        out.extend(CHUNK_BIN.to_le_bytes());
        out.extend(&bin);
        out
    }

    #[test]
    fn a_glb_loads_with_node_transforms_baked_and_materials_merged() {
        let bytes = quad_glb(&[[0.0, 0.0, 0.0], [3.0, 1.0, 0.0]], true);
        let loaded = load(Path::new("quad.glb"), &bytes).unwrap();
        assert_eq!(loaded.parts.len(), 1, "both nodes share one material");
        let part = &loaded.parts[0];
        assert_eq!(part.name, "Red");
        assert_eq!(part.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(part.mesh.indices.len(), 12);
        assert_eq!(part.mesh.vertices.len(), 8);
        let max_x = part
            .mesh
            .vertices
            .iter()
            .map(|v| v.position[0])
            .fold(f32::MIN, f32::max);
        assert!(
            (max_x - 3.5).abs() < 1e-5,
            "second node moved by +3: {max_x}"
        );
        assert!(part
            .mesh
            .vertices
            .iter()
            .all(|v| (Vec3::from(v.normal) - Vec3::Z).length() < 1e-6));
    }

    #[test]
    fn missing_normals_are_generated_flat() {
        let bytes = quad_glb(&[[0.0, 0.0, 0.0]], false);
        let loaded = load(Path::new("quad.glb"), &bytes).unwrap();
        let part = &loaded.parts[0];
        assert_eq!(part.mesh.vertices.len(), 6, "unwelded per triangle");
        for v in &part.mesh.vertices {
            assert!((Vec3::from(v.normal) - Vec3::Z).length() < 1e-5);
        }
    }

    #[test]
    fn mirrored_nodes_keep_their_faces_outward() {
        let mut bytes = quad_glb(&[[0.0, 0.0, 0.0]], true);
        // Swap the translation for a mirroring scale (same JSON length is
        // not required: rebuild through the text).
        let (json, _) = split_glb(&bytes).unwrap();
        let mirrored = json.replace(r#""translation":[0,0,0]"#, r#""scale":[-1,1,1]"#);
        assert_ne!(mirrored, json);
        let doc = Json::parse(&mirrored).unwrap();
        let node = &arr(&doc, "nodes")[0];
        assert!(node_matrix(node).determinant() < 0.0);
        // A full load of the edited file flips the winding.
        bytes = rebuild_glb(&bytes, &mirrored);
        let loaded = load(Path::new("m.glb"), &bytes).unwrap();
        let m = &loaded.parts[0].mesh;
        let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(m.vertices[m.indices[i] as usize].position));
        let face = (b - a).cross(c - a);
        assert!(face.z > 0.0, "the face still points +Z after mirroring");
    }

    fn rebuild_glb(original: &[u8], json: &str) -> Vec<u8> {
        let (_, bin) = split_glb(original).unwrap();
        let bin = bin.unwrap().to_vec();
        let mut json = json.as_bytes().to_vec();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2_u32.to_le_bytes());
        out.extend(u32::try_from(total).unwrap().to_le_bytes());
        out.extend(u32::try_from(json.len()).unwrap().to_le_bytes());
        out.extend(CHUNK_JSON.to_le_bytes());
        out.extend(&json);
        out.extend(u32::try_from(bin.len()).unwrap().to_le_bytes());
        out.extend(CHUNK_BIN.to_le_bytes());
        out.extend(&bin);
        out
    }

    #[test]
    fn strips_and_fans_become_triangle_lists() {
        assert_eq!(triangles(5, &[0, 1, 2, 3]).unwrap(), [0, 1, 2, 2, 1, 3]);
        assert_eq!(triangles(6, &[0, 1, 2, 3]).unwrap(), [0, 1, 2, 0, 2, 3]);
        assert!(triangles(1, &[0, 1]).is_none());
    }

    #[test]
    fn compressed_files_explain_themselves() {
        let json =
            r#"{"asset":{"version":"2.0"},"extensionsRequired":["KHR_draco_mesh_compression"]}"#;
        let err = load(Path::new("d.gltf"), json.as_bytes()).err().unwrap();
        assert!(err.contains("without compression"), "{err}");
        let old = r#"{"asset":{"version":"1.0"}}"#;
        assert!(load(Path::new("o.gltf"), old.as_bytes())
            .err()
            .unwrap()
            .contains("2.0"));
    }

    #[test]
    fn embedded_data_uris_and_external_references() {
        // A .gltf with its buffer embedded as base64 and one external image.
        let glb = quad_glb(&[[0.0, 0.0, 0.0]], true);
        let (json, bin) = split_glb(&glb).unwrap();
        let b64 = encode_base64(bin.unwrap());
        let json = json.replace(
            r#""buffers":[{"byteLength""#,
            &format!(r#""images":[{{"uri":"tex%20a.png"}}],"buffers":[{{"uri":"data:application/octet-stream;base64,{b64}","byteLength""#),
        );
        let loaded = load(Path::new("/nowhere/e.gltf"), json.as_bytes()).unwrap();
        assert_eq!(loaded.parts[0].mesh.indices.len(), 6);
        assert_eq!(
            references(Path::new("/m/e.gltf"), json.as_bytes()),
            [PathBuf::from("/m/tex a.png")]
        );
    }

    #[test]
    fn pbr_factors_maps_and_emission_load() {
        let glb = quad_glb(&[[0.0, 0.0, 0.0]], true);
        let (json, bin) = split_glb(&glb).unwrap();
        let b64 = encode_base64(bin.unwrap());
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 255, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let png = encode_base64(&png);
        let json = json
            .replace(
                r#""materials":[{"name":"Red","pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1]}}]"#,
                r#""materials":[{"name":"Steel","pbrMetallicRoughness":{"roughnessFactor":0.25,
                    "metallicFactor":0.75,"metallicRoughnessTexture":{"index":0}},
                    "normalTexture":{"index":0,"scale":0.5},
                    "emissiveFactor":[1,0.5,0],"emissiveTexture":{"index":0},
                    "extensions":{"KHR_materials_emissive_strength":{"emissiveStrength":4}}}],
                    "textures":[{"source":0}]"#,
            )
            .replace(
                r#""buffers":[{"byteLength""#,
                &format!(r#""images":[{{"uri":"data:image/png;base64,{png}"}}],"buffers":[{{"uri":"data:application/octet-stream;base64,{b64}","byteLength""#),
            );
        assert!(
            json.contains("Steel") && json.contains("image/png"),
            "{json}"
        );
        let loaded = load(Path::new("/nowhere/m.gltf"), json.as_bytes()).unwrap();
        let s = &loaded.parts[0].surface;
        assert_eq!(loaded.parts[0].name, "Steel");
        assert!((s.roughness - 0.25).abs() < 1e-6 && (s.metallic - 0.75).abs() < 1e-6);
        assert_eq!(s.emissive, [4.0, 2.0, 0.0], "factor x strength");
        assert!((s.normal_scale - 0.5).abs() < 1e-6);
        let normal = s.normal_map.as_ref().expect("normal map");
        assert_eq!(
            (normal.width, &normal.rgba[..4]),
            (2, &[128, 128, 255, 255][..])
        );
        assert!(s.metallic_roughness_map.is_some() && s.emissive_map.is_some());
        // The same image as all three maps is decoded once.
        assert!(Arc::ptr_eq(normal, s.emissive_map.as_ref().unwrap()));

        // No factors at all: glTF's default rough metal. No material: the
        // engine's plain surface.
        let bare = json.replace(
            r#""roughnessFactor":0.25,
                    "metallicFactor":0.75,"#,
            "",
        );
        let s = &load(Path::new("/nowhere/b.gltf"), bare.as_bytes())
            .unwrap()
            .parts[0]
            .surface;
        assert!((s.roughness - 1.0).abs() < 1e-6 && (s.metallic - 1.0).abs() < 1e-6);
        let plain = load(Path::new("q.glb"), &glb).unwrap();
        assert!(
            (plain.parts[0].surface.metallic - 1.0).abs() < 1e-6,
            "Red has no factors"
        );
        assert_eq!(PartSurface::default().metallic, 0.0);
    }

    /// A `.gltf` (buffer as a data URI) with two quads sharing a material:
    /// a static `body` at the origin and an `arm` at x = 2 whose `wave` clip
    /// turns it 90° about Z over one second.
    fn waving_gltf() -> String {
        let mut bin: Vec<u8> = Vec::new();
        let mut push = |values: &[f32]| {
            for v in values {
                bin.extend(v.to_le_bytes());
            }
        };
        push(&[
            -0.5, -0.5, 0.0, 0.5, -0.5, 0.0, 0.5, 0.5, 0.0, -0.5, 0.5, 0.0,
        ]); // 0..48
        push(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]); // 48..96
        push(&[0.0, 1.0]); // times, 96..104
        let h = std::f32::consts::FRAC_1_SQRT_2;
        push(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, h, h]); // rotations, 104..136
        for i in [0_u16, 1, 2, 0, 2, 3] {
            bin.extend(i.to_le_bytes()); // 136..148
        }
        format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,1]}}],
            "nodes":[{{"name":"body","mesh":0}},{{"name":"arm","mesh":0,"translation":[2,0,0]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1}},"indices":4,"material":0}}]}}],
            "materials":[{{"name":"Red"}}],
            "animations":[{{"name":"wave","channels":[{{"sampler":0,"target":{{"node":1,"path":"rotation"}}}}],
              "samplers":[{{"input":2,"output":3,"interpolation":"LINEAR"}}]}}],
            "buffers":[{{"byteLength":{len},"uri":"data:application/octet-stream;base64,{data}"}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":48}},
              {{"buffer":0,"byteOffset":96,"byteLength":8}},{{"buffer":0,"byteOffset":104,"byteLength":32}},
              {{"buffer":0,"byteOffset":136,"byteLength":12}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3"}},
              {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},
              {{"bufferView":2,"componentType":5126,"count":2,"type":"SCALAR"}},
              {{"bufferView":3,"componentType":5126,"count":2,"type":"VEC4"}},
              {{"bufferView":4,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
            len = bin.len(),
            data = encode_base64(&bin),
        )
    }

    #[test]
    fn node_animation_keeps_rigid_parts_on_their_nodes() {
        let loaded = load(Path::new("/nowhere/wave.gltf"), waving_gltf().as_bytes()).unwrap();
        let rig = loaded.rig.expect("animated files get a rig");
        assert_eq!(rig.clip_names(), ["wave"]);
        assert!((rig.clips[0].duration - 1.0).abs() < 1e-6);
        assert_eq!(rig.node("arm"), Some(1));
        assert_eq!(rig.joints.len(), 2, "one rigid joint per mesh node");

        // One merged part; the rest pose matches the old baked geometry.
        assert_eq!(loaded.parts.len(), 1);
        let part = &loaded.parts[0];
        let skin = part.skin.as_ref().unwrap();
        assert_eq!(skin.bind.len(), 8);
        assert_eq!(part.mesh.vertices[5].position, [2.5, -0.5, 0.0]);
        assert_eq!(skin.skin[0].joints[0], 0);
        assert_eq!(skin.skin[5].joints[0], 1);

        // At the end of the clip the arm has turned about its own origin;
        // the body hasn't moved.
        let mut pose = rig.rest_pose();
        rig.clips[0].sample_into(1.0, &mut pose);
        let mut palette = Vec::new();
        rig.palette(&pose, &mut palette);
        let posed = skin.posed(&palette);
        let arm = Vec3::from(posed[5].position);
        assert!(arm.abs_diff_eq(Vec3::new(2.5, 0.5, 0.0), 1e-5), "{arm}");
        assert!(Vec3::from(posed[1].position).abs_diff_eq(Vec3::new(0.5, -0.5, 0.0), 1e-6));
        let normal = Vec3::from(posed[5].normal);
        assert!(normal.abs_diff_eq(Vec3::Z, 1e-5));
    }

    #[test]
    fn static_files_have_no_rig() {
        let loaded = load(Path::new("quad.glb"), &quad_glb(&[[0.0; 3]], true)).unwrap();
        assert!(loaded.rig.is_none());
        assert!(loaded.parts.iter().all(|p| p.skin.is_none()));
    }

    #[test]
    fn simple_skin_reads_joints_weights_and_its_clip() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/SimpleSkin.gltf");
        let bytes = std::fs::read(&path).unwrap();
        let loaded = load(&path, &bytes).unwrap();
        let rig = loaded.rig.unwrap();
        assert_eq!(rig.nodes.len(), 3);
        assert_eq!(rig.nodes[2].parent, Some(1));
        assert_eq!(rig.joints.len(), 2);
        // The second joint's inverse bind undoes its rest position (0, 1, 0).
        let (node, ibm) = rig.joints[1];
        assert_eq!(node, 2);
        assert!(ibm.transform_point3(Vec3::Y).abs_diff_eq(Vec3::ZERO, 1e-5));
        let clip = &rig.clips[0];
        assert_eq!(clip.name, "Animation 0");
        assert!((clip.duration - 5.5).abs() < 1e-5);
        assert_eq!(clip.channels[0].times.len(), 12);
        assert_eq!(clip.channels[0].property, Property::Rotation);

        let part = &loaded.parts[0];
        let skin = part.skin.as_ref().unwrap();
        // No normals in the file: flat shading unwelds to 3 per triangle.
        assert_eq!(skin.bind.len(), 24);
        assert_eq!(skin.skin.len(), 24);
        for s in &skin.skin {
            assert!((s.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        // At rest the mesh is its bind pose.
        for (rest, bind) in part.mesh.vertices.iter().zip(&skin.bind) {
            assert!(Vec3::from(rest.position).abs_diff_eq(Vec3::from(bind.position), 1e-5));
        }
        // Mid-clip the top bends; the bottom row stays put.
        let mut pose = rig.rest_pose();
        let peak = clip.channels[0].times[3];
        clip.sample_into(peak, &mut pose);
        let mut palette = Vec::new();
        rig.palette(&pose, &mut palette);
        let posed = skin.posed(&palette);
        let at = |p: Vec3| {
            skin.bind
                .iter()
                .position(|v| Vec3::from(v.position).abs_diff_eq(p, 1e-6))
                .unwrap()
        };
        let bottom = Vec3::from(posed[at(Vec3::new(-0.5, 0.0, 0.0))].position);
        assert!(
            bottom.abs_diff_eq(Vec3::new(-0.5, 0.0, 0.0), 1e-5),
            "{bottom}"
        );
        let top = Vec3::from(posed[at(Vec3::new(0.5, 2.0, 0.0))].position);
        assert!(top.distance(Vec3::new(0.5, 2.0, 0.0)) > 0.3, "{top}");
    }

    fn encode_base64(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..=chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }
}
