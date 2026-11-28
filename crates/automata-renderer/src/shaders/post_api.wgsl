// Custom post effects: the fixed interface around a user's
//
//     fn post(in: PostInput) -> vec3<f32>
//
// run once per pixel of the finished frame (after tone mapping, grading
// and anti-aliasing; before the HUD). Only plain functions, structs and
// constants are allowed in it: no bindings, entry points or global
// variables (see `shader.rs`).

// What the engine hands a post effect, per pixel.
struct PostInput {
    // Where on the screen, 0..1 (top-left origin).
    uv: vec2<f32>,
    // The same in pixels.
    pixel: vec2<f32>,
    // The finished image at `uv` (linear RGB, 0..1).
    color: vec3<f32>,
    // The image size in pixels.
    resolution: vec2<f32>,
    // Seconds since the scene started.
    time: f32,
    // The Post Shader's `params`: p0 = the first four numbers, p1 = the
    // next four (missing ones are 0).
    p0: vec4<f32>,
    p1: vec4<f32>,
};

struct PostShaderData {
    // xy: size (px), zw: one pixel in uv.
    size: vec4<f32>,
    // x: time (s), y: 1 when the image and target hold gamma-encoded
    // values (non-sRGB formats) that need decoding and encoding.
    misc: vec4<f32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
};

@group(0) @binding(0) var post_image: texture_2d<f32>;
@group(0) @binding(1) var post_sampler: sampler;
@group(0) @binding(2) var<uniform> post_data: PostShaderData;

// The finished image at `uv` (linear RGB; clamped to the edges).
fn image(uv: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(post_image, post_sampler, uv, 0.0).rgb;
    if post_data.misc.y > 0.5 {
        return post_decode(c);
    }
    return c;
}

// The finished image at a pixel (rounded to the nearest one).
fn image_pixel(pixel: vec2<f32>) -> vec3<f32> {
    return image((floor(pixel) + 0.5) * post_data.size.zw);
}

// sRGB transfer functions, for targets that don't apply them.
fn post_decode(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn post_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

struct PostVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the screen.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> PostVertex {
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: PostVertex;
    out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

@fragment
fn fs_main(v: PostVertex) -> @location(0) vec4<f32> {
    var in: PostInput;
    in.uv = v.uv;
    in.resolution = post_data.size.xy;
    in.pixel = v.uv * post_data.size.xy;
    in.color = image(v.uv);
    in.time = post_data.misc.x;
    in.p0 = post_data.p0;
    in.p1 = post_data.p1;
    // NaN and negative colours become black, and the result stays in the
    // displayable range.
    var c = clamp(post(in), vec3<f32>(0.0), vec3<f32>(1.0));
    c = select(c, vec3<f32>(0.0), c != c);
    if post_data.misc.y > 0.5 {
        c = post_encode(c);
    }
    return vec4<f32>(c, 1.0);
}
