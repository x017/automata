// Composite pass: a CPU-drawn overlay (an editor's UI, premultiplied RGBA8
// stored as raw bytes) over a rendered scene texture that shows through the
// overlay's transparent pixels. One fullscreen triangle; no blending, the
// "over" operator runs here in the overlay's (sRGB-encoded) space so the
// result matches compositing on the CPU.

struct Params {
    origin: vec2<i32>,  // the scene's top-left corner on the target, pixels
    size: vec2<i32>,    // the scene's size, pixels
    srgb_target: u32,   // 1 when the target encodes to sRGB on write
    has_scene: u32,
    pad0: u32,
    pad1: u32,
};

@group(0) @binding(0) var overlay: texture_2d<f32>;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(pos.xy);
    let ov = textureLoad(overlay, px, 0);
    var sc = vec3<f32>(0.0);
    let local = px - params.origin;
    if params.has_scene == 1u && all(local >= vec2<i32>(0)) && all(local < params.size) {
        // The scene texture is sRGB: loads decode to linear, so re-encode.
        sc = to_srgb(textureLoad(scene, local, 0).rgb);
    }
    var c = ov.rgb + sc * (1.0 - ov.a);
    if params.srgb_target == 1u {
        // The target encodes on write; hand it linear values.
        c = to_linear(c);
    }
    return vec4<f32>(c, 1.0);
}
