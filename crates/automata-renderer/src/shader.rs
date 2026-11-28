//! Custom shaders: user WGSL for materials and post effects, behind a fixed
//! interface.
//!
//! A **material shader** defines `fn material(in: MaterialInput) ->
//! MaterialOutput` (`shaders/material_api.wgsl`): the engine reads the
//! standard material (colour, maps, tiling), hands the result to it, and
//! lights what it returns, on every quality preset. A **post shader**
//! defines `fn post(in: PostInput) -> vec3<f32>` (`shaders/post_api.wgsl`),
//! run over the finished frame before the HUD.
//!
//! User code only ever adds plain functions, structs and constants: no
//! bindings, entry points, global variables or overrides, and `discard`
//! only in materials (cut-outs) ([`check_shader`]). It's spliced into the engine's shader after the
//! interface, then parsed and validated with naga, checked to add no
//! resources or entry points, and translated for WebGL2 — so a shader that
//! loads works on every backend, and one that doesn't reports
//! `file.wgsl:line:column: what's wrong` and leaves the standard look.

use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

/// The longest custom shader (bytes).
pub const MAX_SHADER_SOURCE: usize = 64 * 1024;

/// How many numbers a custom shader takes from its material or Post
/// Shader (`p0` and `p1` in WGSL).
pub const SHADER_PARAMS: usize = 8;

/// Engine code before a custom material's (the mesh shader and the
/// interface).
const MATERIAL_PREFIX: &str = concat!(
    include_str!("shaders/tonemap.wgsl"),
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/mesh.wgsl"),
    include_str!("shaders/material_api.wgsl"),
    include_str!("shaders/shader_helpers.wgsl"),
    "\n// ---- custom material ----\n",
);
/// Engine code after a custom material's.
const MATERIAL_SUFFIX: &str = include_str!("shaders/material_custom.wgsl");
/// Engine code before a custom post effect's.
const POST_PREFIX: &str = concat!(
    include_str!("shaders/post_api.wgsl"),
    include_str!("shaders/shader_helpers.wgsl"),
    "\n// ---- custom post effect ----\n",
);

/// The starting point of a new material shader (Oberon's "New Material
/// Shader").
pub const MATERIAL_TEMPLATE: &str = include_str!("shaders/templates/material.wgsl");
/// The starting point of a new post shader.
pub const POST_TEMPLATE: &str = include_str!("shaders/templates/post.wgsl");

/// The fragment entry point (both kinds have one) the checks translate
/// for WebGL2.
const CHECKED_ENTRY: &str = "fs_main";

/// What a custom shader shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderKind {
    /// A mesh surface: `fn material(in: MaterialInput) -> MaterialOutput`.
    Material,
    /// The finished frame: `fn post(in: PostInput) -> vec3<f32>`.
    Post,
}

impl ShaderKind {
    /// The function a shader of this kind must define.
    #[must_use]
    pub fn function(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::Post => "post",
        }
    }

    /// That function's signature, for messages.
    #[must_use]
    pub fn signature(self) -> &'static str {
        match self {
            Self::Material => "fn material(in: MaterialInput) -> MaterialOutput",
            Self::Post => "fn post(in: PostInput) -> vec3<f32>",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Material => MATERIAL_PREFIX,
            Self::Post => POST_PREFIX,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Material => MATERIAL_SUFFIX,
            Self::Post => "",
        }
    }
}

/// Why a custom shader didn't load: where (in the user's file, when the
/// problem is there) and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderError {
    /// The file, as given to [`check_shader`].
    pub file: String,
    /// 1-based line in the file, when known.
    pub line: Option<u32>,
    /// 1-based column, when known.
    pub column: Option<u32>,
    /// What's wrong.
    pub message: String,
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "{}:{l}:{c}: {}", self.file, self.message),
            (Some(l), None) => write!(f, "{}:{l}: {}", self.file, self.message),
            _ => write!(f, "{}: {}", self.file, self.message),
        }
    }
}

impl std::error::Error for ShaderError {}

/// A custom shader that passed [`check_shader`]: the full WGSL module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedShader {
    /// What it shapes.
    pub kind: ShaderKind,
    /// The engine's shader with the user's code spliced in.
    pub source: String,
}

/// Checks a custom shader and builds the module it runs in: `source` must
/// define the kind's function ([`ShaderKind::signature`]) and only plain
/// functions, structs and constants besides; it must parse, validate and
/// translate for WebGL2. `file` names it in errors.
///
/// # Errors
/// What's wrong, with the line and column in `source` when the problem is
/// in it.
pub fn check_shader(
    kind: ShaderKind,
    file: &str,
    source: &str,
) -> Result<CheckedShader, ShaderError> {
    let fail = |line: Option<u32>, column: Option<u32>, message: String| ShaderError {
        file: file.to_string(),
        line,
        column,
        message,
    };
    if source.len() > MAX_SHADER_SOURCE {
        return Err(fail(
            None,
            None,
            format!(
                "too long ({} KB; custom shaders stop at {} KB)",
                source.len() / 1024,
                MAX_SHADER_SOURCE / 1024
            ),
        ));
    }
    scan(kind, source).map_err(|(offset, message)| {
        let (line, column) = line_column(source, offset);
        fail(Some(line), Some(column), message)
    })?;
    let prefix = kind.prefix();
    let full = format!("{prefix}{source}\n{}", kind.suffix());
    let user = prefix.len()..prefix.len() + source.len();
    // Where a span in the full module is, in the user's file.
    let place = |span: naga::Span| -> (Option<u32>, Option<u32>, bool) {
        let Some(range) = span.to_range() else {
            return (None, None, false);
        };
        if user.contains(&range.start) || range.start == user.end {
            let (l, c) = line_column(source, range.start - user.start);
            (Some(l), Some(c), true)
        } else {
            (None, None, false)
        }
    };
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| {
        let span = e.labels().next().map(|(s, _)| s).unwrap_or_default();
        let (line, column, inside) = place(span);
        fail(line, column, explain(kind, e.message(), inside))
    })?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| {
        let span = e.spans().next().map(|(s, _)| *s).unwrap_or_default();
        let (line, column, inside) = place(span);
        fail(
            line,
            column,
            explain(kind, &error_chain(e.as_inner()), inside),
        )
    })?;
    let base = baseline(kind);
    if module.global_variables.len() != base.globals
        || module.entry_points.len() != base.entry_points
        || module.overrides.len() != base.overrides
    {
        return Err(fail(
            None,
            None,
            "custom shaders can't add bindings, global variables, overrides or entry points".into(),
        ));
    }
    webgl2(&module, &info, naga::ShaderStage::Fragment, CHECKED_ENTRY).map_err(|e| {
        fail(
            None,
            None,
            format!("won't run on WebGL2 (web exports): {e}"),
        )
    })?;
    Ok(CheckedShader { kind, source: full })
}

/// A parse or validation message, with a hint when it's about the
/// interface rather than a line of the user's code.
fn explain(kind: ShaderKind, message: &str, inside: bool) -> String {
    if inside {
        if message.contains("redefinition") {
            return format!("{message} (the engine already uses this name; pick another)");
        }
        return message.to_string();
    }
    format!("{message} (the shader must define `{}`)", kind.signature())
}

/// An error and its causes, one per clause.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        let text = c.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        cause = c.source();
    }
    out
}

/// 1-based line and column of byte `offset` in `text`.
fn line_column(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

/// Words a custom shader may not use, and why.
const FORBIDDEN: &[(&str, &str)] = &[
    (
        "override",
        "pipeline overrides aren't allowed in a custom shader",
    ),
    (
        "enable",
        "`enable` directives aren't allowed in a custom shader",
    ),
    (
        "requires",
        "`requires` directives aren't allowed in a custom shader",
    ),
    (
        "diagnostic",
        "`diagnostic` directives aren't allowed in a custom shader",
    ),
];

/// The quick checks before parsing, on the source without its comments:
/// no attributes (bindings, entry points, built-ins) but `@must_use`, no
/// global address spaces, no forbidden words (`discard` in post effects),
/// and the kind's function defined. On failure: the byte offset and why.
fn scan(kind: ShaderKind, source: &str) -> Result<(), (usize, String)> {
    let code = strip_comments(source);
    let bytes = code.as_bytes();
    let ident_at = |mut i: usize| -> (usize, &str) {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        (i, &code[start..i])
    };
    let mut defines = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'@' {
            let (_, name) = ident_at(i + 1);
            if name != "must_use" {
                return Err((
                    i,
                    format!(
                        "attributes like `@{name}` aren't allowed in a custom shader: the \
                         engine owns its bindings and entry points"
                    ),
                ));
            }
            i += 1;
            continue;
        }
        let word_start = b.is_ascii_alphabetic() || b == b'_';
        let after_word = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if !(word_start && after_word) {
            i += 1;
            continue;
        }
        let (end, word) = ident_at(i);
        if let Some((_, why)) = FORBIDDEN.iter().find(|(w, _)| *w == word) {
            return Err((i, (*why).to_string()));
        }
        if word == "discard" && kind == ShaderKind::Post {
            return Err((
                i,
                "`discard` isn't allowed in a post effect: return a colour for every pixel".into(),
            ));
        }
        if word == "var" {
            let mut j = end;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) == Some(&b'<') {
                let (_, space) = ident_at(j + 1);
                if space != "function" {
                    return Err((
                        i,
                        format!(
                            "`var<{space}>` isn't allowed in a custom shader: only local \
                             variables (inside functions)"
                        ),
                    ));
                }
            }
        }
        if word == "fn" && ident_at(end).1 == kind.function() {
            defines = true;
        }
        i = end;
    }
    if defines {
        Ok(())
    } else {
        Err((0, format!("define `{}`", kind.signature())))
    }
}

/// `source` with its comments (`//` lines and nested `/* */` blocks)
/// blanked out, keeping every byte offset and line break.
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    let mut depth = 0_usize;
    while i < bytes.len() {
        let pair = (bytes[i], bytes.get(i + 1).copied());
        if depth == 0 && pair == (b'/', Some(b'/')) {
            while i < bytes.len() && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        if pair == (b'/', Some(b'*')) {
            depth += 1;
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            continue;
        }
        if depth > 0 && pair == (b'*', Some(b'/')) {
            depth -= 1;
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            continue;
        }
        if depth > 0 && bytes[i] != b'\n' {
            out[i] = b' ';
        }
        i += 1;
    }
    // Only ASCII bytes were replaced (with spaces), and whole multi-byte
    // characters inside comments became spaces byte by byte.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// What the engine's module of a kind holds on its own.
struct Baseline {
    globals: usize,
    entry_points: usize,
    overrides: usize,
}

fn baseline(kind: ShaderKind) -> &'static Baseline {
    static MATERIAL: OnceLock<Baseline> = OnceLock::new();
    static POST: OnceLock<Baseline> = OnceLock::new();
    let (cell, user) = match kind {
        ShaderKind::Material => (
            &MATERIAL,
            "fn material(in: MaterialInput) -> MaterialOutput { return material_defaults(in); }",
        ),
        ShaderKind::Post => (
            &POST,
            "fn post(in: PostInput) -> vec3<f32> { return in.color; }",
        ),
    };
    cell.get_or_init(|| {
        let full = format!("{}{user}\n{}", kind.prefix(), kind.suffix());
        let module = naga::front::wgsl::parse_str(&full).expect("the engine's shader parses");
        Baseline {
            globals: module.global_variables.len(),
            entry_points: module.entry_points.len(),
            overrides: module.overrides.len(),
        }
    })
}

/// Translates `entry` to GLSL ES 3.0 as WebGL2 gets it.
fn webgl2(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: naga::ShaderStage,
    entry: &str,
) -> Result<(), String> {
    let (module, info) = naga::back::pipeline_constants::process_overrides(
        module,
        info,
        Some((stage, entry)),
        &naga::back::PipelineConstants::default(),
    )
    .map_err(|e| e.to_string())?;
    let options = naga::back::glsl::Options {
        version: naga::back::glsl::Version::Embedded {
            version: 300,
            is_webgl: true,
        },
        ..naga::back::glsl::Options::default()
    };
    let pipeline = naga::back::glsl::PipelineOptions {
        shader_stage: stage,
        entry_point: entry.to_owned(),
        multiview: None,
    };
    let mut out = String::new();
    naga::back::glsl::Writer::new(
        &mut out,
        &module,
        &info,
        &options,
        &pipeline,
        naga::proc::BoundsCheckPolicies::default(),
    )
    .and_then(|mut writer| writer.write().map(|_| ()))
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Compiled shaders.

/// A custom shader compiled for the GPU
/// ([`Renderer::create_shader`](crate::Renderer::create_shader)): a
/// material's ([`Renderer::create_shaded_material`](crate::Renderer::create_shaded_material))
/// or a frame's post effect ([`DrawList::set_post_shader`](crate::DrawList::set_post_shader)).
/// Cheap to clone; the same source compiles once while any clone lives.
#[derive(Clone)]
pub struct Shader {
    pub(crate) inner: Arc<ShaderInner>,
}

pub(crate) struct ShaderInner {
    /// Unique per compiled shader (pipelines are cached by it).
    pub id: u64,
    pub kind: ShaderKind,
    pub file: String,
    pub module: wgpu::ShaderModule,
    /// Set when the GPU refused a pipeline built from it: it then draws
    /// with the standard look.
    pub failure: Mutex<Option<String>>,
}

impl Shader {
    /// What it shapes.
    #[must_use]
    pub fn kind(&self) -> ShaderKind {
        self.inner.kind
    }

    /// The file it came from.
    #[must_use]
    pub fn file(&self) -> &str {
        &self.inner.file
    }

    /// Why the GPU refused it, if it did (after it passed the checks, so
    /// a driver problem): until it's fixed it draws as the standard look.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        self.inner
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn id(&self) -> u64 {
        self.inner.id
    }

    pub(crate) fn broken(&self) -> bool {
        self.failure().is_some()
    }

    pub(crate) fn fail(&self, why: String) {
        tracing::error!("{}: the GPU refused the shader: {why}", self.inner.file);
        *self
            .inner
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(why);
    }
}

impl std::fmt::Debug for Shader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shader")
            .field("kind", &self.inner.kind)
            .field("file", &self.inner.file)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Shader {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Compiled shaders by (kind, source hash), while something uses them.
#[derive(Default)]
pub(crate) struct ShaderCache {
    entries: Mutex<Vec<(ShaderKind, u64, Weak<ShaderInner>)>>,
}

impl ShaderCache {
    /// The live shader compiled from `source`, or one made by `make`.
    pub fn get_or_make(
        &self,
        kind: ShaderKind,
        source: &str,
        make: impl FnOnce() -> Result<Shader, ShaderError>,
    ) -> Result<Shader, ShaderError> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut h);
        let hash = h.finish();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|(_, _, w)| w.strong_count() > 0);
        if let Some(inner) = entries
            .iter()
            .find(|(k, h, _)| *k == kind && *h == hash)
            .and_then(|(_, _, w)| w.upgrade())
        {
            return Ok(Shader { inner });
        }
        let shader = make()?;
        entries.push((kind, hash, Arc::downgrade(&shader.inner)));
        Ok(shader)
    }
}

/// A new compiled shader's id.
pub(crate) fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Runs `f` (creating GPU objects) and reports the validation error it
/// raised, if any, instead of letting wgpu's default handler panic.
pub(crate) fn catching<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> (T, Option<String>) {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = f();
    let error = now_or_never(scope.pop()).flatten().map(|e| e.to_string());
    (value, error)
}

/// A future's result if it's already ready (wgpu's error scopes resolve
/// as soon as they pop on every backend the engine uses).
fn now_or_never<F: std::future::Future>(fut: F) -> Option<F::Output> {
    let mut fut = std::pin::pin!(fut);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => Some(v),
        std::task::Poll::Pending => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(body: &str) -> Result<CheckedShader, ShaderError> {
        check_shader(ShaderKind::Material, "shaders/test.wgsl", body)
    }

    fn post(body: &str) -> Result<CheckedShader, ShaderError> {
        check_shader(ShaderKind::Post, "shaders/test.wgsl", body)
    }

    fn rejected(result: Result<CheckedShader, ShaderError>) -> ShaderError {
        result.expect_err("the shader should be refused")
    }

    /// Every entry point of a checked module builds for WebGL2 (the
    /// checks translate one; the others hold the same user code).
    fn all_entry_points_translate(checked: &CheckedShader) {
        let module = naga::front::wgsl::parse_str(&checked.source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for entry in &module.entry_points {
            webgl2(&module, &info, entry.stage, &entry.name)
                .unwrap_or_else(|e| panic!("{}: {e}", entry.name));
        }
    }

    #[test]
    fn the_templates_load_and_build_for_webgl2() {
        let m = material(MATERIAL_TEMPLATE).unwrap();
        assert_eq!(m.kind, ShaderKind::Material);
        all_entry_points_translate(&m);
        let p = post(POST_TEMPLATE).unwrap();
        all_entry_points_translate(&p);
    }

    #[test]
    fn the_examples_in_the_docs_load() {
        let examples = [
            (
                ShaderKind::Material,
                include_str!("../../../examples/oberon/shader_lab/shaders/hologram.wgsl"),
            ),
            (
                ShaderKind::Material,
                include_str!("../../../examples/oberon/shader_lab/shaders/dissolve.wgsl"),
            ),
            (
                ShaderKind::Material,
                include_str!("../../../examples/oberon/shader_lab/shaders/lava.wgsl"),
            ),
            (
                ShaderKind::Post,
                include_str!("../../../examples/oberon/shader_lab/shaders/crt.wgsl"),
            ),
            (
                ShaderKind::Post,
                include_str!("../../../examples/oberon/shader_lab/shaders/ink.wgsl"),
            ),
            (
                ShaderKind::Post,
                include_str!("../../../examples/oberon/shader_lab/shaders/underwater.wgsl"),
            ),
        ];
        for (kind, source) in examples {
            let checked =
                check_shader(kind, "example.wgsl", source).unwrap_or_else(|e| panic!("{e}"));
            all_entry_points_translate(&checked);
        }
    }

    #[test]
    fn helpers_work_anywhere_even_in_branches_and_loops() {
        material(
            "fn material(in: MaterialInput) -> MaterialOutput {
                var out = material_defaults(in);
                if in.uv.x > 0.5 {
                    out.color = base_map(in.uv * 2.0) * noise3(in.position);
                }
                for (var i = 0; i < 4; i++) { out.roughness += 0.01 * hash1(f32(i)); }
                return out;
            }",
        )
        .unwrap();
        post(
            "const TAPS: i32 = 4;
            fn blur(uv: vec2<f32>, r: f32) -> vec3<f32> {
                var sum = vec3<f32>(0.0);
                for (var i = -TAPS; i <= TAPS; i++) {
                    sum += image(uv + vec2<f32>(f32(i) * r, 0.0));
                }
                return sum / f32(2 * TAPS + 1);
            }
            fn post(in: PostInput) -> vec3<f32> {
                if in.uv.y > 0.5 { return blur(in.uv, in.p0.x / in.resolution.x); }
                return image_pixel(in.pixel);
            }",
        )
        .unwrap();
    }

    #[test]
    fn bindings_entry_points_and_globals_are_refused_where_they_are() {
        let e = rejected(material(
            "fn material(in: MaterialInput) -> MaterialOutput { return material_defaults(in); }\n\
             @group(0) @binding(9) var<uniform> secret: vec4<f32>;",
        ));
        assert_eq!((e.line, e.column), (Some(2), Some(1)), "{e}");
        assert!(e.message.contains("@group"), "{e}");
        assert!(e.to_string().starts_with("shaders/test.wgsl:2:1: "), "{e}");

        let e = rejected(post(
            "fn post(in: PostInput) -> vec3<f32> { return in.color; }\n\
             @fragment fn fs_extra() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
        ));
        assert!(e.message.contains("@fragment"), "{e}");

        for global in [
            "var<private> g: f32;",
            "var<workgroup> g: f32;",
            "var<storage> g: f32;",
        ] {
            let e = rejected(post(&format!(
                "{global}\nfn post(in: PostInput) -> vec3<f32> {{ return in.color; }}"
            )));
            assert!(e.message.contains("var<"), "{global}: {e}");
            assert_eq!(e.line, Some(1));
        }
        // Local variables, even spelled out, are fine.
        post("fn post(in: PostInput) -> vec3<f32> { var<function> c = in.color; return c; }")
            .unwrap();
    }

    #[test]
    fn overrides_directives_and_discard_are_refused() {
        for (code, word) in [
            ("override k: f32 = 1.0;", "override"),
            ("enable f16;", "enable"),
            ("diagnostic(off, derivative_uniformity);", "diagnostic"),
        ] {
            let e = rejected(post(&format!(
                "{code}\nfn post(in: PostInput) -> vec3<f32> {{ return in.color; }}"
            )));
            assert!(e.message.contains(word), "{code}: {e}");
        }
        // Materials may cut holes (like the engine's own distance fade);
        // a post effect must colour every pixel.
        material(
            "fn material(in: MaterialInput) -> MaterialOutput {\n\
                 if in.color.a < 0.5 { discard; }\n\
                 return material_defaults(in);\n\
             }",
        )
        .unwrap();
        let e = rejected(post(
            "fn post(in: PostInput) -> vec3<f32> {\n\
                 if in.uv.x < 0.5 { discard; }\n\
                 return in.color;\n\
             }",
        ));
        assert_eq!(e.line, Some(2));
        assert!(e.message.contains("discard"), "{e}");
    }

    #[test]
    fn comments_may_mention_anything() {
        material(
            "// @group(0) @binding(0) var<uniform> discard override\n\
             /* nested /* @vertex */ var<storage> */\n\
             fn material(in: MaterialInput) -> MaterialOutput { return material_defaults(in); }",
        )
        .unwrap();
    }

    #[test]
    fn mistakes_point_at_their_line_in_the_users_file() {
        let e = rejected(material(
            "fn material(in: MaterialInput) -> MaterialOutput {\n\
                 var out = material_defaults(in);\n\
                 out.color = vec4<f32>(undefined_thing, 1.0);\n\
                 return out;\n\
             }",
        ));
        assert_eq!(e.line, Some(3), "{e}");
        assert!(e.message.contains("undefined_thing"), "{e}");

        let e = rejected(post(
            "fn post(in: PostInput) -> vec3<f32> {\n    return in.colour;\n}",
        ));
        assert_eq!(e.line, Some(2), "{e}");
    }

    #[test]
    fn the_function_must_exist_with_its_signature() {
        let e = rejected(material("fn shade(x: f32) -> f32 { return x; }"));
        assert!(e.message.contains(ShaderKind::Material.signature()), "{e}");
        let e = rejected(post("fn post(x: f32) -> f32 { return x; }"));
        assert!(e.message.contains(ShaderKind::Post.signature()), "{e}");
        assert_eq!(e.line, None, "the mismatch shows at the engine's call");
    }

    #[test]
    fn names_the_engine_uses_are_explained() {
        let e = rejected(material(
            "fn surface(x: f32) -> f32 { return x; }\n\
             fn material(in: MaterialInput) -> MaterialOutput { return material_defaults(in); }",
        ));
        assert!(e.message.contains("pick another"), "{e}");
        assert_eq!(e.line, Some(1), "{e}");
    }

    #[test]
    fn huge_sources_are_refused() {
        let big = format!(
            "fn post(in: PostInput) -> vec3<f32> {{ return in.color; }}\n{}",
            "// padding\n".repeat(MAX_SHADER_SOURCE / 8)
        );
        assert!(rejected(post(&big)).message.contains("too long"));
    }

    #[test]
    fn line_columns_count_characters() {
        assert_eq!(line_column("ab\ncdé f", 0), (1, 1));
        assert_eq!(line_column("ab\ncdé f", 3), (2, 1));
        assert_eq!(line_column("ab\ncdé f", 8), (2, 5));
    }
}
