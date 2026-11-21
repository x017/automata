// Shared lighting for lit 3D shaders (mesh.wgsl, water.wgsl): the camera
// and light bindings, cascaded shadow lookup, the sky used for reflections,
// physically based sun shading, fog, and tone mapping.
//
// Bind groups used here:
//   0 — camera (view-projection + world position)
//   3 — light (uniform, shadow cascades, comparison sampler)

const PI: f32 = 3.14159265;

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct LightUniform {
    direction: vec4<f32>,      // xyz: direction the light travels, w: soft shadow radius (m)
    color_ambient: vec4<f32>,  // rgb: light color, a: ambient fraction
    cascades: array<mat4x4<f32>, 4>, // light view-projection per cascade
    splits: vec4<f32>,         // far view depth of each cascade
    cascade_texel: vec4<f32>,  // world size of one shadow texel, per cascade
    cascade_bias: vec4<f32>,   // depth bias (NDC units), per cascade
    params: vec4<f32>,         // x: shadows on, y: shadow texel (uv), z: cascades, w: beautiful
    sky_exposure: vec4<f32>,   // rgb: sky ambient, a: exposure
    ground_tonemap: vec4<f32>, // rgb: ground ambient, a: tone mapping mode
    fog: vec4<f32>,            // rgb: fog color, a: density (0 = off)
    fog_params: vec4<f32>,     // x: fog start distance, y: time (s)
    camera_forward: vec4<f32>, // xyz: camera forward
    background: vec4<f32>,     // rgb: clear color (the horizon in reflections)
    screen: vec4<f32>,         // xy: viewport size (px), z: near, w: far
};
@group(3) @binding(0) var<uniform> light: LightUniform;
@group(3) @binding(1) var shadow_map: texture_depth_2d_array;
@group(3) @binding(2) var shadow_sampler: sampler_comparison;
@group(3) @binding(3) var shadow_map_0: texture_depth_2d; // cascade 0 (forward path)

// Two variants share these helpers: the forward path (`shadow_pcf`,
// `apply_fog`, `finish_forward`) and Beautiful (`shadow_soft`, `shade_pbr`,
// `apply_fog_scatter`). Each shader has an entry point per variant, so a
// pipeline only compiles the code its preset runs — no runtime switches,
// which software rasterizers would pay for on both sides.

// Interleaved gradient noise (Jimenez): a per-pixel value in [0, 1) that
// tiles invisibly; used to rotate shadow kernels.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

const SOFT_TAPS: i32 = 12;

// 16-point Poisson disk in the unit circle (the soft filter uses the first
// SOFT_TAPS, which cover it evenly).
const POISSON: array<vec2<f32>, 16> = array<vec2<f32>, 16>(
    vec2<f32>(-0.94201624, -0.39906216), vec2<f32>(0.94558609, -0.76890725),
    vec2<f32>(-0.09418410, -0.92938870), vec2<f32>(0.34495938, 0.29387760),
    vec2<f32>(-0.91588581, 0.45771432), vec2<f32>(-0.81544232, -0.87912464),
    vec2<f32>(-0.38277543, 0.27676845), vec2<f32>(0.97484398, 0.75648379),
    vec2<f32>(0.44323325, -0.97511554), vec2<f32>(0.53742981, -0.47373420),
    vec2<f32>(-0.26496911, -0.41893023), vec2<f32>(0.79197514, 0.19090188),
    vec2<f32>(-0.24188840, 0.99706507), vec2<f32>(-0.81409955, 0.91437590),
    vec2<f32>(0.19984126, 0.78641367), vec2<f32>(0.14383161, -0.14100790),
);

// Forward path: one shadow map, 3x3 PCF with a slope-scaled depth bias.
// 1.0 lit, 0.0 shadowed.
fn shadow_pcf(world: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    if light.params.x < 0.5 {
        return 1.0;
    }
    let projected = light.cascades[0] * vec4<f32>(world, 1.0);
    let ndc = projected.xyz / projected.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 {
        return 1.0; // outside the shadow map counts as lit
    }
    let bias = max(0.0025 * (1.0 - dot(n, l)), 0.0005);
    let texel = light.params.y;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            sum = sum + textureSampleCompareLevel(
                shadow_map_0, shadow_sampler, uv + offset, ndc.z - bias);
        }
    }
    return sum / 9.0;
}

// Beautiful: sunlight reaching `world` through cascade `i`, filtered by a
// rotated Poisson disk whose radius is a fixed width in the world (a
// constant penumbra across cascades), each tap a bilinear 2x2 compare.
fn sample_cascade(i: i32, world: vec3<f32>, n: vec3<f32>, ndotl: f32, frag: vec2<f32>) -> f32 {
    // Normal offset: look the shadow up slightly off the surface, more at
    // grazing angles, instead of a large depth bias.
    let pos = world + n * light.cascade_texel[i] * (1.0 + 2.0 * (1.0 - ndotl));
    let projected = light.cascades[i] * vec4<f32>(pos, 1.0);
    let ndc = projected.xyz / projected.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 {
        return 1.0;
    }
    let texel = light.params.y;
    let width = light.cascade_texel[i] / texel; // the cascade's width (m)
    let radius = clamp(light.direction.w / width, 1.5 * texel, 5.0 * texel);
    let angle = ign(frag) * 2.0 * PI;
    let rot = mat2x2<f32>(cos(angle), sin(angle), -sin(angle), cos(angle));
    let depth = ndc.z - light.cascade_bias[i];
    var sum = 0.0;
    // 12 taps: each is a bilinear 2x2 compare, and the per-pixel rotation
    // turns what banding remains into fine noise FXAA smooths.
    for (var k = 0; k < SOFT_TAPS; k = k + 1) {
        let offset = rot * POISSON[k] * radius;
        sum = sum + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, i, depth);
    }
    return sum / f32(SOFT_TAPS);
}

// Beautiful: picks the cascade by view depth. Near a cascade's far edge
// it dithers into the next one (per-pixel noise against the blend weight)
// so seams don't show, while every pixel still filters a single cascade;
// past the last one shadows fade out the same way (returns the count).
fn shadow_cascade(world: vec3<f32>, frag: vec2<f32>) -> i32 {
    let count = i32(light.params.z);
    let depth = dot(world - camera.view_pos.xyz, light.camera_forward.xyz);
    var i = 0;
    for (; i < count - 1; i = i + 1) {
        if depth < light.splits[i] {
            break;
        }
    }
    var start = 0.0;
    if i > 0 {
        start = light.splits[i - 1];
    }
    let band = (light.splits[i] - start) * 0.15;
    let t = (depth - (light.splits[i] - band)) / band;
    if t > ign(frag + vec2<f32>(37.0, 17.0)) {
        i = i + 1;
    }
    return i;
}

fn shadow_soft(world: vec3<f32>, n: vec3<f32>, l: vec3<f32>, frag: vec2<f32>) -> f32 {
    let ndotl = dot(n, l);
    if light.params.x < 0.5 {
        return 1.0;
    }
    if ndotl <= 0.0 {
        return 0.0; // facing away: no direct sun to shadow
    }
    let i = shadow_cascade(world, frag);
    if i >= i32(light.params.z) {
        return 1.0;
    }
    return sample_cascade(i, world, n, ndotl, frag);
}

// Hemispheric ambient: sky from above, ground from below.
fn hemi(n: vec3<f32>) -> vec3<f32> {
    return mix(light.ground_tonemap.rgb, light.sky_exposure.rgb, n.y * 0.5 + 0.5);
}

// The sky seen along `d`, for reflections: horizon (the clear color) up to
// the sky ambient color, down to the ground color.
fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    let horizon = light.background.rgb;
    if d.y >= 0.0 {
        return mix(horizon, light.sky_exposure.rgb, sqrt(clamp(d.y, 0.0, 1.0)));
    }
    return mix(horizon, light.ground_tonemap.rgb, sqrt(clamp(-d.y, 0.0, 1.0)));
}

// x^5 by multiplication (pow() is exp(log()) on many GPUs and emulators).
fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x;
}

// GGX normal distribution.
fn d_ggx(ndoth: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = ndoth * ndoth * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Smith-Schlick geometry term for direct light.
fn g_smith(ndotv: f32, ndotl: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    return (ndotv / (ndotv * (1.0 - k) + k)) * (ndotl / (ndotl * (1.0 - k) + k));
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow5(1.0 - cos_theta);
}

// Analytical environment BRDF (Karis, "Physically Based Shading on
// Mobile"): the share of sky a surface reflects, without a lookup texture.
fn env_brdf(f0: vec3<f32>, roughness: f32, ndotv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * ndotv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + vec3<f32>(ab.y);
}

// Physically based sun + ambient shading (Beautiful): GGX specular with
// Fresnel, energy-conserving diffuse, hemispheric diffuse ambient and sky
// reflections. Light intensities follow the forward path's convention (a
// white sun lights a white, sun-facing surface to 1.0).
fn shade_pbr(
    albedo: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    shadow: f32,
    roughness: f32,
    f0: vec3<f32>,
) -> vec3<f32> {
    let h = normalize(l + v);
    let ndotl = max(dot(n, l), 0.0);
    let ndotv = max(dot(n, v), 1e-4);
    let ndoth = max(dot(n, h), 0.0);
    let vdoth = max(dot(v, h), 0.0);
    let f = fresnel_schlick(vdoth, f0);
    let spec = d_ggx(ndoth, roughness) * g_smith(ndotv, ndotl, roughness) * f
        / max(4.0 * ndotv * ndotl, 1e-4);
    let kd = vec3<f32>(1.0) - f;
    let direct = (kd * albedo + spec * PI) * light.color_ambient.rgb * ndotl * shadow;

    // Ambient: diffuse from the hemisphere, specular from the sky, weighted
    // by the environment BRDF so rough surfaces don't turn into mirrors.
    let fr = env_brdf(f0, roughness, ndotv);
    let env = sky_radiance(reflect(-v, n));
    // Reflections of the sky are partly blocked in shadowed creases.
    let occlusion = mix(0.6, 1.0, shadow);
    let ambient = (albedo * hemi(n) * (vec3<f32>(1.0) - fr) + env * fr * occlusion)
        * light.color_ambient.a;
    return direct + ambient;
}

// Exponential distance fog (before tone mapping, like real scattering).
fn apply_fog(lit: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let density = light.fog.a;
    if density <= 0.0 {
        return lit;
    }
    let dist = max(distance(camera.view_pos.xyz, world) - light.fog_params.x, 0.0);
    let f = clamp(1.0 - exp(-density * dist), 0.0, 1.0);
    return mix(lit, light.fog.rgb, f);
}

// Beautiful's fog adds sun scattering: it glows toward the sun.
fn apply_fog_scatter(lit: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let density = light.fog.a;
    if density <= 0.0 {
        return lit;
    }
    let to_frag = world - camera.view_pos.xyz;
    let dist = max(length(to_frag) - light.fog_params.x, 0.0);
    let f = clamp(1.0 - exp(-density * dist), 0.0, 1.0);
    let l = normalize(-light.direction.xyz);
    let s2 = max(dot(normalize(to_frag), l), 0.0);
    let s4 = s2 * s2;
    let s8 = s4 * s4;
    let s = s8 * s8;
    let color = light.fog.rgb + light.color_ambient.rgb * s * 0.5;
    return mix(lit, color, f);
}

// The forward path tone maps in the lit shaders (Beautiful writes HDR and
// the post chain tone maps after bloom).
fn finish_forward(c: vec3<f32>) -> vec3<f32> {
    return tonemap_with(c, light.sky_exposure.a, light.ground_tonemap.a);
}
