// Shadow pass: depth-only rendering from the sun's point of view.
// Bind groups: 0 — light-space view-projection, 1 — skinned draws only:
// model matrix and joint palette (plain draws are instanced, model
// matrices per instance at locations 5–8, as in mesh.wgsl).

struct LightViewProj {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> light: LightViewProj;

@group(1) @binding(0) var<uniform> model: mat4x4<f32>;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(5) m0: vec4<f32>,
    @location(6) m1: vec4<f32>,
    @location(7) m2: vec4<f32>,
    @location(8) m3: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    return light.view_proj * mat4x4<f32>(m0, m1, m2, m3) * vec4<f32>(position, 1.0);
}

@group(1) @binding(1) var<uniform> joints: array<mat4x4<f32>, 128>;

@vertex
fn vs_skinned(
    @location(0) position: vec3<f32>,
    @location(3) joint: vec4<u32>,
    @location(4) weight: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let skin = joints[joint.x] * weight.x + joints[joint.y] * weight.y
        + joints[joint.z] * weight.z + joints[joint.w] * weight.w;
    return light.view_proj * model * skin * vec4<f32>(position, 1.0);
}
