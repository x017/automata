// Mesh shader: Blinn-Phong lighting from a directional sun, with shadow
// mapping (3x3 PCF over a depth texture rendered from the light's view).
//
// Bind groups:
//   0 — camera (view-projection + world position)
//   1 — model matrix (per draw)
//   2 — material (texture, sampler, base color)
//   3 — light (sun uniform, shadow map, comparison sampler)

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

@group(1) @binding(0) var<uniform> model: mat4x4<f32>;

struct MaterialData {
    color: vec4<f32>,
};
@group(2) @binding(0) var material_texture: texture_2d<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<uniform> material: MaterialData;

struct LightUniform {
    direction: vec4<f32>,      // xyz: direction the light travels
    color_ambient: vec4<f32>,  // rgb: light color, a: ambient fraction
    light_view_proj: mat4x4<f32>,
    params: vec4<f32>,         // x: shadows enabled, y: shadow texel size
};
@group(3) @binding(0) var<uniform> light: LightUniform;
@group(3) @binding(1) var shadow_map: texture_depth_2d;
@group(3) @binding(2) var shadow_sampler: sampler_comparison;

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

// Fraction of sunlight reaching the fragment: 1.0 lit, 0.0 fully shadowed.
fn shadow_factor(world_position: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    if light.params.x < 0.5 {
        return 1.0;
    }
    let projected = light.light_view_proj * vec4<f32>(world_position, 1.0);
    let ndc = projected.xyz / projected.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 {
        return 1.0; // outside the shadow map counts as lit
    }
    // Slope-scaled bias fights shadow acne on grazing angles.
    let bias = max(0.0025 * (1.0 - dot(n, l)), 0.0005);
    let texel = light.params.y;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            sum = sum + textureSampleCompare(
                shadow_map, shadow_sampler, uv + offset, ndc.z - bias);
        }
    }
    return sum / 9.0;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(-light.direction.xyz);
    let v = normalize(camera.view_pos.xyz - in.world_position);
    let h = normalize(l + v);

    let diffuse = max(dot(n, l), 0.0);
    let specular = pow(max(dot(n, h), 0.0), 32.0) * 0.25 * diffuse;
    let shadow = shadow_factor(in.world_position, n, l);

    let base = textureSample(material_texture, material_sampler, in.uv) * material.color;
    let lit = base.rgb * light.color_ambient.a
        + (base.rgb * diffuse + vec3<f32>(specular)) * light.color_ambient.rgb * shadow;
    return vec4<f32>(lit, base.a);
}
