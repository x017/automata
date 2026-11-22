// Mesh shader. Prepended at build time: tonemap.wgsl and common.wgsl (camera
// and light bindings, shadows, sky, PBR, fog, tone mapping).
//
// Two fragment entry points: `fs_main` (forward path) and `fs_beautiful`.
//
// Bind groups:
//   0 — camera (common.wgsl)
//   1 — model matrix (per draw); skinned draws add the joint palette
//   2 — material (base color texture + sampler, parameters, normal /
//       metallic-roughness / emissive maps + their shared sampler)
//   3 — light (common.wgsl)

@group(1) @binding(0) var<uniform> model: mat4x4<f32>;

// Skinned draws only (`vs_skinned`): one matrix per joint, taking a vertex
// from bind space to model space. Must match `MAX_JOINTS` in Rust.
@group(1) @binding(1) var<uniform> joints: array<mat4x4<f32>, 128>;

struct MaterialData {
    color: vec4<f32>,
    // RGB emitted light.
    emissive: vec4<f32>,
    // Roughness, metallic, normal-map scale, map flags (see MAP_*).
    params: vec4<f32>,
};
@group(2) @binding(0) var material_texture: texture_2d<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<uniform> material: MaterialData;
@group(2) @binding(3) var normal_map: texture_2d<f32>;
@group(2) @binding(4) var metal_rough_map: texture_2d<f32>;
@group(2) @binding(5) var emissive_map: texture_2d<f32>;
@group(2) @binding(6) var map_sampler: sampler;

// Which optional maps the material has (bits of `params.w`): unused maps
// aren't sampled at all, which matters on software rasterizers.
const MAP_NORMAL: u32 = 1u;
const MAP_METAL_ROUGH: u32 = 2u;
const MAP_EMISSIVE: u32 = 4u;

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
    p: vec3<f32>,
    n: vec3<f32>,
    l: vec3<f32>,
    v: vec3<f32>,
    base: vec4<f32>,
    roughness: f32,
    metallic: f32,
    emissive: vec3<f32>,
};

// Bends `n` by a tangent-space normal map sample. The tangent frame comes
// from screen-space derivatives of the position and UVs (Schüler's
// cotangent frame), so meshes need no tangent attribute.
fn perturb_normal(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, ts: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let len = max(dot(t, t), dot(b, b));
    if len < 1e-20 {
        return n; // no UV gradient (untextured geometry)
    }
    let inv = inverseSqrt(len);
    return normalize(t * inv * ts.x + b * inv * ts.y + n * ts.z);
}

fn surface(in: VertexOutput) -> Surface {
    var out: Surface;
    let maps = u32(material.params.w);
    var n = normalize(in.normal);
    if (maps & MAP_NORMAL) != 0u {
        var ts = textureSample(normal_map, map_sampler, in.uv).xyz * 2.0 - 1.0;
        // glTF normal maps are +Y up in a V-down UV space: flip green.
        ts = vec3<f32>(ts.xy * vec2<f32>(material.params.z, -material.params.z), ts.z);
        n = perturb_normal(n, in.world_position, in.uv, ts);
    }
    out.p = in.world_position;
    out.n = n;
    out.l = normalize(-light.direction.xyz);
    out.v = normalize(camera.view_pos.xyz - in.world_position);
    out.base = textureSample(material_texture, material_sampler, in.uv) * material.color;
    out.roughness = material.params.x;
    out.metallic = material.params.y;
    if (maps & MAP_METAL_ROUGH) != 0u {
        let mr = textureSample(metal_rough_map, map_sampler, in.uv);
        out.roughness = clamp(out.roughness * mr.g, 0.04, 1.0);
        out.metallic = out.metallic * mr.b;
    }
    out.emissive = material.emissive.rgb;
    if (maps & MAP_EMISSIVE) != 0u {
        out.emissive = out.emissive * textureSample(emissive_map, map_sampler, in.uv).rgb;
    }
    return out;
}

// The forward path's surface response: Blinn-Phong whose highlight
// tightens and brightens as roughness drops (the default roughness gives
// the engine's original pow-32 highlight), metals tinting it and losing
// their diffuse, and sky reflections on metals and polished surfaces.
fn shade_forward(s: Surface, shadow: f32) -> vec3<f32> {
    let h = normalize(s.l + s.v);
    let diffuse = max(dot(s.n, s.l), 0.0);
    let shininess = exp2(11.0 - 10.0 * s.roughness);
    let f0 = mix(vec3<f32>(0.04), s.base.rgb, s.metallic);
    // Energy-normalized so a sharp highlight is a bright one; 0.25 for the
    // default dielectric.
    let spec_color = f0 * 6.25 * (shininess + 8.0) / 40.0;
    let specular = pow(max(dot(s.n, h), 1e-5), shininess) * diffuse * spec_color;
    let albedo = s.base.rgb * (1.0 - s.metallic);
    var lit = albedo * hemi(s.n) * light.color_ambient.a
        + (albedo * diffuse + specular) * light.color_ambient.rgb * shadow;
    // Point lights (and glowing meshes): the same diffuse + highlight.
    for (var i = 0u; i < point_count(); i = i + 1u) {
        let p = point_light(i, s.p);
        let ndotl = dot(s.n, p.l);
        if ndotl > 0.0 && any(p.radiance > vec3<f32>(0.0)) {
            let hp = normalize(p.l + s.v);
            let spec = pow(max(dot(s.n, hp), 1e-5), shininess) * spec_color;
            lit = lit + (albedo + spec) * p.radiance * ndotl;
        }
    }
    // Default (rough, non-metal) surfaces stay matte, as before.
    let gloss = max(s.metallic, clamp(1.0 - 2.0 * s.roughness, 0.0, 1.0));
    if gloss > 0.0 {
        let fr = env_brdf(f0, s.roughness, max(dot(s.n, s.v), 1e-4));
        let env = sky_radiance(reflect(-s.v, s.n));
        lit += env * fr * gloss * light.color_ambient.a * mix(0.6, 1.0, shadow);
    }
    return lit + s.emissive;
}

// Forward path: Blinn-Phong sun and point lights, 3x3 PCF shadows, hemispheric ambient, fog,
// tone mapping.
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = surface(in);
    let shadow = shadow_pcf(in.world_position, s.n, s.l);
    let lit = shade_forward(s, shadow);
    return vec4<f32>(finish_forward(apply_fog(lit, in.world_position)), s.base.a);
}

// Beautiful: GGX sun with soft cascaded shadows, sky reflections and
// sun-scattering fog, written as HDR for the post chain.
@fragment
fn fs_beautiful(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = surface(in);
    let shadow = shadow_soft(in.world_position, s.n, s.l, in.clip_position.xy);
    let albedo = s.base.rgb * (1.0 - s.metallic);
    let f0 = mix(vec3<f32>(0.04), s.base.rgb, s.metallic);
    let lit = shade_pbr(albedo, s.n, s.v, s.l, shadow, s.roughness, f0)
        + points_pbr(s.p, albedo, s.n, s.v, s.roughness, f0)
        + s.emissive;
    return vec4<f32>(apply_fog_scatter(lit, in.world_position), s.base.a);
}
