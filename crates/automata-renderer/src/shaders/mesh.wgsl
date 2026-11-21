// Mesh shader: Blinn-Phong lighting from a directional sun, with shadow
// mapping (3x3 PCF over a depth texture rendered from the light's view),
// hemispheric sky/ground ambient, exponential distance fog, exposure, and
// tone mapping (None / Reinhard / ACES / AgX-style).
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
    sky_exposure: vec4<f32>,   // rgb: sky ambient, a: exposure
    ground_tonemap: vec4<f32>, // rgb: ground ambient, a: tone mapping mode
    fog: vec4<f32>,            // rgb: fog color, a: density (0 = off)
    fog_params: vec4<f32>,     // x: fog start distance
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

// ACES filmic approximation (Krzysztof Narkowicz).
fn tonemap_aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// A compact AgX-style curve: log2 encoding plus a polynomial sigmoid fit,
// which desaturates bright values gracefully instead of clipping hue.
fn tonemap_agx(x: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479, 0.0423282, 0.0423756),
        vec3<f32>(0.0784336, 0.878468, 0.0784336),
        vec3<f32>(0.0792237, 0.0791661, 0.879142),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.19688, -0.0528969, -0.0529716),
        vec3<f32>(-0.0980209, 1.15190, -0.0980435),
        vec3<f32>(-0.0990297, -0.0989612, 1.15107),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * max(x, vec3<f32>(1e-10));
    v = clamp(log2(v), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    let v2 = v * v;
    let v4 = v2 * v2;
    v = 15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v
        + 0.4298 * v2 + 0.1191 * v - 0.00232;
    // The curve's output is display-encoded; the target applies sRGB on
    // write, so linearize it back.
    return pow(clamp(outset * v, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = c * light.sky_exposure.a;
    let mode = light.ground_tonemap.a;
    if mode > 2.5 {
        return tonemap_agx(x);
    }
    if mode > 1.5 {
        return tonemap_aces(x);
    }
    if mode > 0.5 {
        return x / (vec3<f32>(1.0) + x);
    }
    return x;
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
    // Hemispheric ambient: sky from above, ground from below.
    let hemi = mix(light.ground_tonemap.rgb, light.sky_exposure.rgb, n.y * 0.5 + 0.5);
    var lit = base.rgb * hemi * light.color_ambient.a
        + (base.rgb * diffuse + vec3<f32>(specular)) * light.color_ambient.rgb * shadow;

    // Exponential distance fog (before tone mapping, like real scattering).
    let density = light.fog.a;
    if density > 0.0 {
        let dist = max(distance(camera.view_pos.xyz, in.world_position) - light.fog_params.x, 0.0);
        let f = 1.0 - exp(-density * dist);
        lit = mix(lit, light.fog.rgb, clamp(f, 0.0, 1.0));
    }
    return vec4<f32>(tonemap(lit), base.a);
}
