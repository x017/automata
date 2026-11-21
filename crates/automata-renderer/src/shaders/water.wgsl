// Water surface. Prepended at build time: tonemap.wgsl and common.wgsl.
//
// A flat grid displaced by four Gerstner waves (vertex), with per-pixel
// normals from the same waves plus two scrolling layers of noise ripples.
// Shading: Fresnel between the water body and the reflected sky, a GGX sun
// glint, backlit crests. Beautiful adds refraction of the scene below,
// depth-based absorption and shoreline foam (it reads a copy of the scene's
// color and depth); the forward path alpha-blends instead.
//
// Bind groups:
//   0 — camera (common.wgsl)
//   1 — water uniform
//   2 — scene color + depth copies (1x1 placeholders on the forward path)
//   3 — light (common.wgsl)

struct WaterUniform {
    model: mat4x4<f32>,   // unit grid ([-0.5, 0.5] on XZ) -> world, size included
    deep: vec4<f32>,      // rgb: deep color, a: clarity (m of visibility)
    shallow: vec4<f32>,   // rgb: shallow color, a: foam amount 0..1
    params: vec4<f32>,    // x: wave height (m), y: refraction on, z: time (s)
    // Per wave, folded on the CPU (water.rs): xy direction * k, z phase
    // (omega * t), w k; amplitudes x/y on the grid (LOD-faded), z/w per
    // pixel — each pair is (amplitude, steepness * amplitude).
    wave_k: array<vec4<f32>, 4>,
    wave_amp: array<vec4<f32>, 4>,
};
@group(1) @binding(0) var<uniform> water: WaterUniform;
// Tiling ripples baked once (water.rs ripple_texels): rg slope, b height.
@group(1) @binding(1) var ripple_map: texture_2d<f32>;
@group(1) @binding(2) var ripple_sampler: sampler;

@group(2) @binding(0) var scene_color: texture_2d<f32>;
@group(2) @binding(1) var scene_depth: texture_depth_2d;
@group(2) @binding(2) var scene_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) base_xz: vec2<f32>,
    @location(2) crest: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let base = water.model * vec4<f32>(in.position, 1.0);
    // Gerstner displacement with the short waves the grid can't hold
    // faded out (their amplitudes are zero in wave_amp.xy).
    var offset = vec3<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let wk = water.wave_k[i];
        let wa = water.wave_amp[i];
        let phase = dot(wk.xy, base.xz) - wk.z;
        let c = cos(phase);
        let d = wk.xy / wk.w;
        offset = offset + vec3<f32>(wa.y * d.x * c, wa.x * sin(phase), wa.y * d.y * c);
    }
    let world = base.xyz + offset;
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.world_position = world;
    out.base_xz = base.xz;
    out.crest = offset.y / max(water.params.x, 1e-3);
    return out;
}

// The surface slope at `xz`: every wave (no LOD; the CPU folded the
// constants, so a wave is a dot, a sin and a cos), as -dh/dx, -dh/dz
// with the Gerstner normal's y term in z.
fn wave_slope(xz: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(0.0, 1.0, 0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let wk = water.wave_k[i];
        let wa = water.wave_amp[i];
        let phase = dot(wk.xy, xz) - wk.z;
        let c = cos(phase);
        n = n - vec3<f32>(wk.x * wa.z * c, wk.w * wa.w * sin(phase), wk.y * wa.z * c);
    }
    return n;
}

// Shadows on water need far fewer taps than on solid ground: the waves
// and ripples break shadow edges up anyway. Both variants take 4 taps
// (the meshes take 9 and 12) on a flat +Y normal.
//
// Forward: a 2x2 box at half-texel offsets of the near cascade.
fn water_shadow(world: vec3<f32>) -> f32 {
    if light.params.x < 0.5 {
        return 1.0;
    }
    let projected = light.cascades[0] * vec4<f32>(world, 1.0);
    let ndc = projected.xyz / projected.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 {
        return 1.0;
    }
    let l = normalize(-light.direction.xyz);
    let depth = ndc.z - max(0.0025 * (1.0 - l.y), 0.0005);
    let h = light.params.y * 0.75;
    return (textureSampleCompareLevel(shadow_map_0, shadow_sampler, uv + vec2<f32>(-h, -h), depth)
        + textureSampleCompareLevel(shadow_map_0, shadow_sampler, uv + vec2<f32>(h, -h), depth)
        + textureSampleCompareLevel(shadow_map_0, shadow_sampler, uv + vec2<f32>(-h, h), depth)
        + textureSampleCompareLevel(shadow_map_0, shadow_sampler, uv + vec2<f32>(h, h), depth)) * 0.25;
}

// Beautiful: the dithered cascade choice, then 4 bilinear compares on a
// per-pixel rotated square (16 texels of filtering).
fn water_shadow_soft(world: vec3<f32>, frag: vec2<f32>) -> f32 {
    let l = normalize(-light.direction.xyz);
    if light.params.x < 0.5 {
        return 1.0;
    }
    if l.y <= 0.0 {
        return 0.0;
    }
    let i = shadow_cascade(world, frag);
    if i >= i32(light.params.z) {
        return 1.0;
    }
    let pos = world + vec3<f32>(0.0, 1.0, 0.0) * light.cascade_texel[i] * (3.0 - 2.0 * l.y);
    let projected = light.cascades[i] * vec4<f32>(pos, 1.0);
    let ndc = projected.xyz / projected.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 {
        return 1.0;
    }
    let texel = light.params.y;
    let width = light.cascade_texel[i] / texel;
    let radius = clamp(light.direction.w / width, 1.5 * texel, 5.0 * texel) * 0.6;
    let angle = ign(frag) * 2.0 * PI;
    let a = vec2<f32>(cos(angle), sin(angle)) * radius;
    let b = vec2<f32>(-a.y, a.x);
    let depth = ndc.z - light.cascade_bias[i];
    return (textureSampleCompareLevel(shadow_map, shadow_sampler, uv + a, i, depth)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, uv - a, i, depth)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + b, i, depth)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, uv - b, i, depth)) * 0.25;
}

// Depth buffer value -> view depth (m).
fn linear_depth(d: f32) -> f32 {
    let near = light.screen.z;
    let far = light.screen.w;
    return near * far / (far - d * (far - near));
}

// What both variants compute for a water fragment.
struct WaterShade {
    n: vec3<f32>,
    fresnel: f32,
    reflection: vec3<f32>,
    glint: vec3<f32>,
    body: vec3<f32>,
    ambient: vec3<f32>,
    sunlit: vec3<f32>,
    foam_noise: f32,
};

fn water_shade(in: VertexOutput, shadow: f32) -> WaterShade {
    var out: WaterShade;
    let world = in.world_position;
    let to_cam = camera.view_pos.xyz - world;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 1e-4);
    let l = normalize(-light.direction.xyz);
    let t = water.params.z;

    // Normal: the full wave sum plus two scrolling, differently scaled and
    // rotated layers of baked ripples. Their mipmaps fade them with
    // distance (where they'd only shimmer) for free.
    let uv1 = in.base_xz * 0.163 + vec2<f32>(0.05, 0.028) * t;
    let uv2 = mat2x2<f32>(0.8, 0.6, -0.6, 0.8) * in.base_xz * 0.37 + vec2<f32>(-0.085, 0.107) * t;
    let r1 = textureSample(ripple_map, ripple_sampler, uv1);
    let r2 = textureSample(ripple_map, ripple_sampler, uv2);
    let ripple = (r1.xy * 2.0 - 1.0) * 0.2 + (r2.xy * 2.0 - 1.0) * 0.12;
    var n = normalize(wave_slope(in.base_xz) - vec3<f32>(ripple.x, 0.0, ripple.y));
    out.foam_noise = r2.z;
    if dot(n, v) < 0.0 {
        n = normalize(n + v * (0.01 - dot(n, v)));
    }
    out.n = n;

    let ndotv = max(dot(n, v), 1e-3);
    out.fresnel = 0.02 + 0.98 * pow5(1.0 - ndotv);

    // Reflection: the sky, mirrored up.
    var r = reflect(-v, n);
    r.y = abs(r.y);
    out.reflection = sky_radiance(r);

    // Sun glint: a very smooth GGX lobe (Fresnel at the half vector).
    // Skipped where the sun can't reach (shadowed or below the surface).
    let ndotl = max(dot(n, l), 0.0);
    let sun = light.color_ambient.rgb;
    out.glint = vec3<f32>(0.0);
    if ndotl * shadow > 0.0 {
        let h = normalize(l + v);
        let rough = 0.08;
        let spec = d_ggx(max(dot(n, h), 0.0), rough) * g_smith(ndotv, ndotl, rough)
            * (0.02 + 0.98 * pow5(1.0 - max(dot(v, h), 0.0)))
            / max(4.0 * ndotv * ndotl, 1e-4);
        out.glint = sun * spec * PI * ndotl * shadow;
    }

    // The water body: deep color lit by sky and sun, glowing where light
    // passes through crests toward the viewer.
    out.ambient = hemi(vec3<f32>(0.0, 1.0, 0.0)) * light.color_ambient.a;
    out.sunlit = sun * max(l.y, 0.0) * mix(0.35, 1.0, shadow);
    var body = water.deep.rgb * (out.ambient + out.sunlit * 0.6);
    let back_dot = max(dot(-v, l), 0.0);
    let back = back_dot * back_dot * back_dot * 0.5 + 0.15;
    body = body + water.shallow.rgb * out.sunlit * clamp(in.crest, 0.0, 1.0) * back * 0.6;
    out.body = body;
    return out;
}

// Forward path: blends over the scene; grazing views reflect fully.
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let shadow = water_shadow(in.world_position);
    let w = water_shade(in, shadow);
    let color = mix(w.body, w.reflection, w.fresnel) + w.glint;
    let alpha = mix(0.8, 1.0, w.fresnel);
    return vec4<f32>(finish_forward(apply_fog(color, in.world_position)), alpha);
}

// Beautiful: refraction through the water column (from copies of the HDR
// scene), depth-based color absorption and shoreline foam.
@fragment
fn fs_refract(in: VertexOutput) -> @location(0) vec4<f32> {
    let world = in.world_position;
    let shadow = water_shadow_soft(world, in.clip_position.xy);
    let w = water_shade(in, shadow);
    let n = w.n;
    let t = water.params.z;
    let size = light.screen.xy;
    let px = vec2<i32>(in.clip_position.xy);
    let water_depth = linear_depth(in.clip_position.z);
    let ground0 = linear_depth(textureLoad(scene_depth, px, 0));
    let column0 = max(ground0 - water_depth, 0.0);
    // Distort less in the shallows (and never pull in foreground objects).
    var uv = in.clip_position.xy / size;
    let distorted = uv + n.xz * 0.035 * clamp(column0, 0.0, 1.0);
    let dpx = vec2<i32>(clamp(distorted * size, vec2<f32>(0.0), size - 1.0));
    let ground1 = linear_depth(textureLoad(scene_depth, dpx, 0));
    var ground = ground0;
    if ground1 > water_depth {
        uv = distorted;
        ground = ground1;
    }
    let below = textureSampleLevel(scene_color, scene_sampler, uv, 0.0).rgb;
    let column = max(ground - water_depth, 0.0);
    // Beer-Lambert: the scene below fades into the body color with depth,
    // tinted by the shallow color on the way.
    let clarity = max(water.deep.a, 0.05);
    let transmit = exp(-column / clarity);
    let tint = mix(vec3<f32>(1.0), water.shallow.rgb * 1.6, clamp(column / clarity, 0.0, 1.0));
    let under = mix(w.body, below * tint, transmit);
    var color = mix(under, w.reflection, w.fresnel) + w.glint;

    // Shoreline foam: a broken band where the water is shallow.
    let foam_band = 1.0 - smoothstep(0.0, 0.35, column);
    let pattern = smoothstep(0.35, 0.75, w.foam_noise
        + 0.35 * sin(column * 18.0 - t * 2.0));
    let foam = clamp(foam_band * pattern * water.shallow.a * 1.5, 0.0, 1.0);
    let foam_color = vec3<f32>(0.95) * (w.ambient + w.sunlit);
    color = mix(color, foam_color, foam);
    // Soft edge where the water meets the shore.
    let edge = smoothstep(0.0, 0.06, column);
    color = mix(below, color, edge);
    return vec4<f32>(apply_fog_scatter(color, world), 1.0);
}
