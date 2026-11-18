// Mesh shader: camera + per-draw model transform, textured with a base
// color, lit by a single fixed directional light (Lambert). A placeholder
// until Phase 3+ introduces a real lighting model.

struct CameraUniform {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

@group(1) @binding(0) var<uniform> model: mat4x4<f32>;

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
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world;
    // Upper 3x3 of the model matrix — correct for rotation + uniform scale.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light_dir = normalize(vec3<f32>(-0.5, -1.0, -0.35));
    let n = normalize(in.normal);
    let diffuse = max(dot(n, -light_dir), 0.0);
    let ambient = 0.3;
    let base = textureSample(material_texture, material_sampler, in.uv) * material.color;
    return vec4<f32>(base.rgb * (ambient + diffuse * 0.9), base.a);
}
