// Mesh shader. Prepended at build time: tonemap.wgsl and common.wgsl (camera
// and light bindings, shadows, sky, PBR, fog, tone mapping).
//
// Fragment entry points: four for the forward path, one per mix of the
// shadows a frame samples (`fs_main` sun and point light shadows,
// `fs_sun_shadows`, `fs_point_shadows`, `fs_unshadowed`), and
// `fs_beautiful`. Shadow code that never runs still costs (registers, and
// software rasterizers walk it for every pixel), so each variant compiles
// only what it samples.
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
fn shade_forward(s: Surface, shadow: f32, point_shadows: bool) -> vec3<f32> {
    let h = normalize(s.l + s.v);
    let diffuse = max(dot(s.n, s.l), 0.0);
    let shininess = exp2(11.0 - 10.0 * s.roughness);
    let f0 = mix(vec3<f32>(0.04), s.base.rgb, s.metallic);
    // Energy-normalized so a sharp highlight is a bright one; 0.25 for the
    // default dielectric.
    let spec_color = f0 * 6.25 * (shininess + 8.0) / 40.0;
    let sun_in = toon_light(diffuse * shadow);
    var highlight = pow(max(dot(s.n, h), 1e-5), shininess);
    if toon_on() {
        highlight = toon_highlight(highlight);
    }
    let specular = highlight * spec_color;
    let albedo = s.base.rgb * (1.0 - s.metallic);
    var lit = albedo * hemi(s.n) * light.color_ambient.a * shade_tint(sun_in)
        + (albedo + specular) * light.color_ambient.rgb * sun_in
        + rim_light(s.n, s.v, s.l, shadow, albedo);
    // Point lights (and glowing meshes): the same diffuse + highlight.
    for (var i = 0u; i < point_count(); i = i + 1u) {
        let p = point_light(i, s.p);
        let ndotl = dot(s.n, p.l);
        if ndotl > 0.0 && any(p.radiance > vec3<f32>(0.0)) {
            let hp = normalize(p.l + s.v);
            let spec = pow(max(dot(s.n, hp), 1e-5), shininess) * spec_color;
            var light_in = (albedo + spec) * p.radiance * toon_light(ndotl);
            // Shadows only where the light shows: in its faint outer reach
            // the lookup would cost more than it could change.
            if point_shadows && max(light_in.r, max(light_in.g, light_in.b)) > POINT_SHADOW_VISIBLE {
                light_in = light_in * point_shadow(i, s.p, s.n);
            }
            lit = lit + light_in;
        }
    }
    // Default (rough, non-metal) surfaces stay matte, as before.
    let gloss = max(s.metallic, clamp(1.0 - 2.0 * s.roughness, 0.0, 1.0));
    if gloss > 0.0 {
        let fr = env_brdf(f0, s.roughness, max(dot(s.n, s.v), 1e-4));
        let env = sky_radiance(reflect(-s.v, s.n));
        lit += env * fr * gloss * light.color_ambient.a * mix(0.6, 1.0, min(shadow, 1.0));
    }
    return lit + s.emissive;
}

// Forward path: Blinn-Phong sun and point lights, filtered shadows,
// hemispheric ambient, fog, tone mapping. The flags are constants in each
// entry point, so the compiler drops the shadow lookups a variant skips.
fn forward(in: VertexOutput, sun_shadows: bool, point_shadows: bool) -> vec4<f32> {
    let s = surface(in);
    var shadow = 1.0;
    if sun_shadows {
        shadow = shadow_forward(in.world_position, s.n, s.l);
    }
    // Under water the light is tinted and the sun comes through in caustics.
    let wl = water_light(in.world_position);
    let lit = shade_forward(s, shadow * wl.a, point_shadows) * wl.rgb;
    return vec4<f32>(finish_forward(apply_fog(lit, in.world_position, in.clip_position.xy)), s.base.a);
}

// Sun and point light shadows (High and up, when both cast).
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return forward(in, true, true);
}

// Sun shadows only (Medium; higher presets without shadowed point lights).
@fragment
fn fs_sun_shadows(in: VertexOutput) -> @location(0) vec4<f32> {
    return forward(in, true, false);
}

// Point light shadows only (a sun without shadows, or none).
@fragment
fn fs_point_shadows(in: VertexOutput) -> @location(0) vec4<f32> {
    return forward(in, false, true);
}

// No shadows (Low, or nothing casts any).
@fragment
fn fs_unshadowed(in: VertexOutput) -> @location(0) vec4<f32> {
    return forward(in, false, false);
}

// Beautiful's screen-space occlusion (ssao.wgsl), half resolution: r
// ambient occlusion, g contact shadow, b view depth.
@group(3) @binding(5) var occlusion_map: texture_2d<f32>;

// The occlusion at full-resolution pixel `frag`: the four nearest
// half-resolution texels, bilinearly weighted but leaving out those on
// another surface (by view depth), so it doesn't bleed across edges.
fn screen_occlusion(frag: vec2<f32>, world: vec3<f32>) -> vec2<f32> {
    let dims = vec2<i32>(textureDimensions(occlusion_map));
    let depth = dot(world - camera.view_pos.xyz, light.camera_forward.xyz);
    // Texel i was computed at full-resolution pixel 2i.
    let t = (frag - vec2<f32>(0.5)) * 0.5;
    let base = vec2<i32>(floor(t));
    let f = t - floor(t);
    let tolerance = 0.03 * depth + 0.05;
    var sum = vec2<f32>(0.0);
    var total = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let s = textureLoad(occlusion_map, clamp(base + o, vec2<i32>(0), dims - 1), 0);
        let bilinear = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let gap = abs(s.b - depth);
        let w = (bilinear + 1e-3) * f32(gap < tolerance);
        sum += s.rg * w;
        total += w;
    }
    if total <= 0.0 {
        // No texel on this surface: see-through meshes (not in the depth
        // prepass) and slivers too thin for half resolution stay open.
        return vec2<f32>(1.0);
    }
    return sum / total;
}

// Beautiful: GGX sun with contact-hardening cascaded shadows, ambient
// occlusion, sky reflections and sun-scattering fog, written as HDR for
// the post chain.
@fragment
fn fs_beautiful(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = surface(in);
    let occ = screen_occlusion(in.clip_position.xy, in.world_position);
    let wl = water_light(in.world_position);
    let shadow = shadow_soft(in.world_position, s.n, s.l, in.clip_position.xy) * occ.y * wl.a;
    let albedo = s.base.rgb * (1.0 - s.metallic);
    let f0 = mix(vec3<f32>(0.04), s.base.rgb, s.metallic);
    let ao = pow(max(occ.x, 1e-3), light.point_shadow.w);
    let lit = (shade_pbr(albedo, s.n, s.v, s.l, shadow, s.roughness, f0, ao)
        + points_pbr(s.p, albedo, s.n, s.v, s.roughness, f0)
        + s.emissive) * wl.rgb;
    return vec4<f32>(apply_fog_scatter(lit, in.world_position, in.clip_position.xy), s.base.a);
}
