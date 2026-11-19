// Shadow pass: depth-only rendering from the sun's point of view.
// Bind groups: 0 — light-space view-projection, 1 — model matrix.

struct LightViewProj {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> light: LightViewProj;

@group(1) @binding(0) var<uniform> model: mat4x4<f32>;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return light.view_proj * model * vec4<f32>(position, 1.0);
}
