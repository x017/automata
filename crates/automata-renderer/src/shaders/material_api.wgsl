
// ---------------------------------------------------------------------------
// Custom material shaders: the fixed interface (appended to mesh.wgsl).
//
// A material file's `shader` is WGSL defining
//
//     fn material(in: MaterialInput) -> MaterialOutput
//
// which the engine calls for every pixel of every mesh wearing it, after
// the standard material has been read (colour, maps, tiling) and before
// lighting. Only plain functions, structs and constants are allowed in it:
// no bindings, entry points or global variables (see `shader.rs`).

// What the engine hands a custom material, per pixel.
struct MaterialInput {
    // World-space position of the surface (metres).
    position: vec3<f32>,
    // World-space surface normal (unit length, normal map applied).
    normal: vec3<f32>,
    // Unit vector from the surface toward the camera.
    view: vec3<f32>,
    // Texture coordinates, with the material's tiling and offset.
    uv: vec2<f32>,
    // Window position in pixels (top-left origin).
    pixel: vec2<f32>,
    // The standard material's results: base colour × base map (alpha in
    // `a`), roughness, metalness, glow.
    color: vec4<f32>,
    roughness: f32,
    metallic: f32,
    emissive: vec3<f32>,
    // Seconds since the scene started.
    time: f32,
    // The material file's `shader_params`: p0 = the first four numbers,
    // p1 = the next four (missing ones are 0).
    p0: vec4<f32>,
    p1: vec4<f32>,
};

// What a custom material returns: the surface the lights then shade.
struct MaterialOutput {
    color: vec4<f32>,
    normal: vec3<f32>,
    roughness: f32,
    metallic: f32,
    emissive: vec3<f32>,
};

// The standard material unchanged: start from this and change what you
// need.
fn material_defaults(in: MaterialInput) -> MaterialOutput {
    var out: MaterialOutput;
    out.color = in.color;
    out.normal = in.normal;
    out.roughness = in.roughness;
    out.metallic = in.metallic;
    out.emissive = in.emissive;
    return out;
}

// The mip level the material's own texture coordinates sample at, set
// before `material` runs (so `base_map` works anywhere, even in branches
// and loops, where derivatives don't).
var<private> material_lod: f32;

// The material's base colour map at `uv` (not tinted by its colour).
fn base_map(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(material_texture, material_sampler, uv, material_lod);
}

// The level of detail `uv` samples the base map at (from its screen-space
// derivatives; called in uniform control flow).
fn material_mip(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(material_texture));
    let dx = dpdx(uv) * size;
    let dy = dpdy(uv) * size;
    let d = max(dot(dx, dx), dot(dy, dy));
    return max(0.5 * log2(max(d, 1e-8)), 0.0);
}

// Builds the custom material's input from the standard surface.
fn material_input(in: VertexOutput, uv: vec2<f32>, s: Surface) -> MaterialInput {
    var m: MaterialInput;
    m.position = s.p;
    m.normal = s.n;
    m.view = s.v;
    m.uv = uv;
    m.pixel = in.clip_position.xy;
    m.color = s.base;
    m.roughness = s.roughness;
    m.metallic = s.metallic;
    m.emissive = s.emissive;
    m.time = light.fog_params.y;
    m.p0 = material_data.custom0;
    m.p1 = material_data.custom1;
    return m;
}

// Puts a custom material's output back into the surface (sanitized: a
// zero normal or NaN would poison the lighting).
fn material_surface(s: Surface, o: MaterialOutput) -> Surface {
    var out = s;
    out.base = clamp(o.color, vec4<f32>(0.0), vec4<f32>(64.0));
    let len = length(o.normal);
    if len > 1e-5 {
        out.n = o.normal / len;
    }
    out.roughness = clamp(o.roughness, 0.04, 1.0);
    out.metallic = clamp(o.metallic, 0.0, 1.0);
    out.emissive = clamp(o.emissive, vec3<f32>(0.0), vec3<f32>(1000.0));
    return out;
}
