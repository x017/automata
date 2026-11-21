// Mesh shader. Prepended at build time: tonemap.wgsl and common.wgsl (camera
// and light bindings, shadows, sky, PBR, fog, tone mapping).
//
// Two fragment entry points: `fs_main` (forward path) and `fs_beautiful`.
//
// Bind groups:
//   0 — camera (common.wgsl)
//   1 — model matrix (per draw); skinned draws add the joint palette
//   2 — material (texture, sampler, base color)
//   3 — light (common.wgsl)

@group(1) @binding(0) var<uniform> model: mat4x4<f32>;

// Skinned draws only (`vs_skinned`): one matrix per joint, taking a vertex
// from bind space to model space. Must match `MAX_JOINTS` in Rust.
@group(1) @binding(1) var<uniform> joints: array<mat4x4<f32>, 128>;

struct MaterialData {
    color: vec4<f32>,
};
@group(2) @binding(0) var material_texture: texture_2d<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<uniform> material: MaterialData;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world;
    out.world_position = world.xyz;
    // Upper 3x3 of the model matrix — correct for rotation + uniform scale.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

struct SkinInput {
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
};

// The vertex's blended joint matrix.
fn skin_matrix(skin: SkinInput) -> mat4x4<f32> {
    return joints[skin.joints.x] * skin.weights.x
        + joints[skin.joints.y] * skin.weights.y
        + joints[skin.joints.z] * skin.weights.z
        + joints[skin.joints.w] * skin.weights.w;
}

@vertex
fn vs_skinned(in: VertexInput, skin: SkinInput) -> VertexOutput {
    var out: VertexOutput;
    let posed = model * skin_matrix(skin);
    let world = posed * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world;
    out.world_position = world.xyz;
    out.normal = (posed * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

// What both variants know about a fragment.
struct Surface {
    n: vec3<f32>,
    l: vec3<f32>,
    v: vec3<f32>,
    base: vec4<f32>,
};

fn surface(in: VertexOutput) -> Surface {
    var out: Surface;
    out.n = normalize(in.normal);
    out.l = normalize(-light.direction.xyz);
    out.v = normalize(camera.view_pos.xyz - in.world_position);
    out.base = textureSample(material_texture, material_sampler, in.uv) * material.color;
    return out;
}

// Forward path: Blinn-Phong sun, 3x3 PCF shadows, hemispheric ambient, fog,
// tone mapping.
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = surface(in);
    let shadow = shadow_pcf(in.world_position, s.n, s.l);
    let h = normalize(s.l + s.v);
    let diffuse = max(dot(s.n, s.l), 0.0);
    // pow(n.h, 32) by repeated squaring.
    var nh = max(dot(s.n, h), 0.0);
    nh = nh * nh;
    nh = nh * nh;
    nh = nh * nh;
    nh = nh * nh;
    let specular = nh * nh * 0.25 * diffuse;
    let lit = s.base.rgb * hemi(s.n) * light.color_ambient.a
        + (s.base.rgb * diffuse + vec3<f32>(specular)) * light.color_ambient.rgb * shadow;
    return vec4<f32>(finish_forward(apply_fog(lit, in.world_position)), s.base.a);
}

// Beautiful: GGX sun with soft cascaded shadows, sky reflections and
// sun-scattering fog, written as HDR for the post chain.
@fragment
fn fs_beautiful(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = surface(in);
    let shadow = shadow_soft(in.world_position, s.n, s.l, in.clip_position.xy);
    let lit = shade_pbr(s.base.rgb, s.n, s.v, s.l, shadow, 0.6, vec3<f32>(0.04));
    return vec4<f32>(apply_fog_scatter(lit, in.world_position), s.base.a);
}
