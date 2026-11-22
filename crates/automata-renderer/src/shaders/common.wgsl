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

// A point light or spotlight (`PointLight` in Rust). Glowing meshes become
// these too.
struct PointLightData {
    position_range: vec4<f32>,   // xyz: position, w: where it fades out (m)
    intensity_radius: vec4<f32>, // rgb: intensity, w: glowing body radius (m)
    shadow: vec4<f32>,           // x: first shadow layer (-1 = none), y: near, z: far,
                                 // w: cone offset (1 = shines every way)
    cone: vec4<f32>,             // xyz: spot axis, w: cone scale (0 = not a spot)
};

// How much wider than its cone a spot's shadow map looks (radians); must
// match `SPOT_SHADOW_MARGIN` in light.rs.
const SPOT_SHADOW_MARGIN: f32 = 0.0698132;

// Must match `MAX_POINT_LIGHTS` / `MAX_POINT_SHADOWS` in Rust.
const MAX_POINT_LIGHTS: u32 = 32u;
const MAX_POINT_SHADOWS: u32 = 4u;

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
    fog_params: vec4<f32>,     // x: fog start distance, y: time (s), z: point lights
    camera_forward: vec4<f32>, // xyz: camera forward
    background: vec4<f32>,     // rgb: clear color (the horizon in reflections)
    screen: vec4<f32>,         // xy: viewport size (px), z: near, w: far
    inv_view_proj: mat4x4<f32>, // clip -> world (the sky's view rays)
    sky_zenith: vec4<f32>,     // rgb: sky color straight up, a: sky dome on
    sky_sun: vec4<f32>,        // x: cos(sun radius), y: glow, z: clouds, w: lens flare
    underwater: vec4<f32>,     // rgb: the water around the camera, a: extinction /m (0 = dry)
    water_eye: vec4<f32>,      // x: surface height over the camera, y: near half height, z: caustic waters
    water_rects: array<vec4<f32>, 4>,  // caustic waters' world XZ min (xy), max (zw)
    water_levels: array<vec4<f32>, 4>, // x: rest height, y: caustic strength, z: clarity (m)
    points: array<PointLightData, MAX_POINT_LIGHTS>,
    // Six views (+X, -X, +Y, -Y, +Z, -Z) per shadowed point light.
    point_faces: array<mat4x4<f32>, 24>,
    point_shadow: vec4<f32>,   // x: texel (uv), y: taps (1, 4 or 8), z: tan(face fov / 2), w: AO strength
    fog_height: vec4<f32>,     // x: fog layer top (world y), y: falloff /m above it (0 = flat), z: sun light
};
@group(3) @binding(0) var<uniform> light: LightUniform;
@group(3) @binding(1) var shadow_map: texture_depth_2d_array;
@group(3) @binding(2) var shadow_sampler: sampler_comparison;
@group(3) @binding(4) var point_shadow_map: texture_depth_2d_array; // point light cubes

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
// PCSS blocker search reads, and how far a caster can be from the surface
// it shadows before the penumbra stops widening (m).
const PCSS_SEARCH_TAPS: i32 = 8;
const PCSS_MAX_CASTER_DISTANCE: f32 = 10.0;

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

// Where `world` lands in cascade `i`'s map: (uv, compare depth), or x < 0
// outside it. The lookup starts slightly off the surface along `n` (about
// a texel, more at grazing angles) and compares about a texel nearer the
// sun, which keeps lit faces free of acne without detaching shadows.
fn cascade_coords(i: i32, world: vec3<f32>, n: vec3<f32>, ndotl: f32) -> vec3<f32> {
    let pos = world + n * light.cascade_texel[i] * (1.0 + 2.0 * (1.0 - ndotl));
    let projected = light.cascades[i] * vec4<f32>(pos, 1.0);
    let ndc = projected.xyz / projected.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 {
        return vec3<f32>(-1.0);
    }
    return vec3<f32>(uv, ndc.z - light.cascade_bias[i]);
}

// The cascade covering `world` (by view depth) and how far it is into the
// band at that cascade's far edge (0..1; 0 before the band). Across the
// band the next cascade takes over, or past the last one shadows fade
// out. A scene-wide single map never ends (its split is f32 max).
fn cascade_pick(world: vec3<f32>) -> vec2<f32> {
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
    return vec2<f32>(f32(i), clamp((depth - (light.splits[i] - band)) / band, 0.0, 1.0));
}

// Forward: sunlight through cascade `i`: 4 bilinear compares (16 texels)
// on a rotated grid, as wide as the sun's softness asks but between 1 and
// 3 texels, so it stays crisp and cheap.
fn forward_cascade(i: i32, world: vec3<f32>, n: vec3<f32>, ndotl: f32) -> f32 {
    let c = cascade_coords(i, world, n, ndotl);
    if c.x < 0.0 {
        return 1.0;
    }
    let texel = light.params.y;
    let width = light.cascade_texel[i] / texel; // the cascade's width (m)
    let r = clamp(light.direction.w / width, texel, 3.0 * texel);
    let a = vec2<f32>(0.5, 0.2) * r;
    let b = vec2<f32>(-0.2, 0.5) * r;
    return (textureSampleCompareLevel(shadow_map, shadow_sampler, c.xy + a, i, c.z)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, c.xy - a, i, c.z)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, c.xy + b, i, c.z)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, c.xy - b, i, c.z)) * 0.25;
}

// Forward path: 1.0 lit, 0.0 shadowed. Cascade edges cross-fade (both
// cascades are filtered inside the band, one everywhere else).
fn shadow_forward(world: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    if light.params.x < 0.5 {
        return 1.0;
    }
    let ndotl = dot(n, l);
    if ndotl <= 0.0 {
        return 0.0; // facing away: no direct sun to shadow
    }
    let pick = cascade_pick(world);
    let i = i32(pick.x);
    var lit = forward_cascade(i, world, n, ndotl);
    if pick.y > 0.0 {
        var next = 1.0;
        if i + 1 < i32(light.params.z) {
            next = forward_cascade(i + 1, world, n, ndotl);
        }
        lit = mix(lit, next, pick.y);
    }
    return lit;
}

// Beautiful: sunlight reaching `world` through cascade `i`, with
// contact-hardening penumbrae (PCSS): shadows start sharp where an object
// meets the ground and widen with the distance to whatever casts them, as
// much as the sun's apparent size (and the sun's softness) says.
//
// 1. Blocker search: 8 depth reads over the widest penumbra find the
//    average depth of whatever lies between the surface and the sun (none:
//    fully lit, done).
// 2. Filter: 12 bilinear 2x2 compares on a per-pixel rotated Poisson disk
//    as wide as that blocker distance casts the penumbra.
fn sample_cascade(i: i32, world: vec3<f32>, n: vec3<f32>, ndotl: f32, frag: vec2<f32>) -> f32 {
    let c = cascade_coords(i, world, n, ndotl);
    if c.x < 0.0 {
        return 1.0;
    }
    let uv = c.xy;
    let texel = light.params.y;
    let width = light.cascade_texel[i] / texel; // the cascade's width (m)
    let depth_range = light.cascade_texel[i] / max(light.cascade_bias[i], 1e-9); // (m)
    // How fast the penumbra grows per meter between caster and receiver:
    // tan(sun radius), scaled by the sun's softness (1 = as drawn).
    let cos_r = clamp(light.sky_sun.x, 0.5, 1.0);
    let spread = sqrt(1.0 - cos_r * cos_r) / cos_r * light.direction.w * 20.0;
    let max_radius = clamp(spread * PCSS_MAX_CASTER_DISTANCE / width, 2.0 * texel, 16.0 * texel);
    let angle = ign(frag) * 2.0 * PI;
    let rot = mat2x2<f32>(cos(angle), sin(angle), -sin(angle), cos(angle));
    let depth = c.z;

    let dims = vec2<f32>(textureDimensions(shadow_map));
    var blockers = 0.0;
    var blocker_depth = 0.0;
    for (var k = 0; k < PCSS_SEARCH_TAPS; k = k + 1) {
        let p = uv + rot * POISSON[k * 2 + 1] * max_radius;
        let texel_pos = clamp(vec2<i32>(p * dims), vec2<i32>(0), vec2<i32>(dims) - 1);
        let d = textureLoad(shadow_map, texel_pos, i, 0);
        if d < depth {
            blockers += 1.0;
            blocker_depth += d;
        }
    }
    if blockers == 0.0 {
        return 1.0;
    }
    let caster_distance = max(depth - blocker_depth / blockers, 0.0) * depth_range;
    let radius = clamp(spread * caster_distance / width, texel, max_radius);

    var sum = 0.0;
    // Each tap is a bilinear 2x2 compare, and the per-pixel rotation turns
    // what banding remains into fine noise FXAA smooths.
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

// --- Point lights -----------------------------------------------------------
// Inverse-square falloff, held flat inside the glowing body (no hot spot)
// and windowed to reach exactly zero at the light's range (Karis's
// (1 - (d/R)^4)^2), so lights can be skipped past it. A spotlight also
// fades from its core to its cone's edge (smoothly, on the cosine of the
// angle off its axis). The nearest few can cast shadows (`point_shadow`,
// below).

struct PointSample {
    l: vec3<f32>,        // direction to the light
    radiance: vec3<f32>, // light arriving (before the cosine term)
};

fn point_count() -> u32 {
    return min(u32(light.fog_params.z), MAX_POINT_LIGHTS);
}

// What light `i` delivers at `world`; zero radiance beyond its range.
fn point_light(i: u32, world: vec3<f32>) -> PointSample {
    var out: PointSample;
    let p = light.points[i];
    let to_light = p.position_range.xyz - world;
    let d2 = dot(to_light, to_light);
    let range2 = p.position_range.w * p.position_range.w;
    out.l = to_light * inverseSqrt(max(d2, 1e-8));
    out.radiance = vec3<f32>(0.0);
    if d2 >= range2 {
        return out;
    }
    let x = d2 / range2;
    let window = clamp(1.0 - x * x, 0.0, 1.0);
    let r = p.intensity_radius.w;
    // Spot cone: 1 inside the core, 0 past the edge (always 1 for point
    // lights, whose scale is 0 and offset 1).
    let cone = clamp(dot(-out.l, p.cone.xyz) * p.cone.w + p.shadow.w, 0.0, 1.0);
    out.radiance = p.intensity_radius.rgb * (window * window * cone * cone / max(d2, r * r));
    return out;
}

// Below this much light (about one 8-bit step), the forward path doesn't
// look up a point light's shadow: it couldn't visibly change the pixel.
const POINT_SHADOW_VISIBLE: f32 = 0.004;

// tan(SPOT_SHADOW_MARGIN), and of the widest a spot's shadow view gets
// (85°).
const TAN_SPOT_SHADOW_MARGIN: f32 = 0.0699268;
const TAN_SPOT_SHADOW_MAX: f32 = 11.430052;

// Where `world` lands in point light `i`'s shadow map: (uv, layer, compare
// depth), or layer < 0 when the light casts none. Each shadowed point light
// renders six slightly wider than 90° views; the direction's major axis
// picks one. A spotlight renders one view down its cone. A normal offset of
// about a texel at that distance keeps surfaces from shadowing themselves.
//
// The view is worked out here rather than read from `point_faces`, and
// without branches: which face a pixel picks varies across the screen, a
// matrix fetched by a per-pixel index is a slow gather, and divergent
// branches run both sides on many GPUs and software rasterizers. It must
// match `PointShadow::for_light`.
fn point_shadow_coords(i: u32, world: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let p = light.points[i];
    let first = i32(p.shadow.x);
    if first < 0 {
        return vec4<f32>(0.0, 0.0, -1.0, 0.0);
    }
    let to_frag = world - p.position_range.xyz;
    // A point light's face: along the major axis, looking with up +Y (+Z
    // for the ±Y faces).
    let a = abs(to_frag);
    let s = select(vec3<f32>(-1.0), vec3<f32>(1.0), to_frag > vec3<f32>(0.0));
    let on_x = a.x >= a.y && a.x >= a.z;
    let on_y = !on_x && a.y >= a.z;
    var forward_axis = select(
        select(vec3<f32>(0.0, 0.0, s.z), vec3<f32>(0.0, s.y, 0.0), on_y),
        vec3<f32>(s.x, 0.0, 0.0),
        on_x,
    );
    var right = select(
        select(vec3<f32>(-s.z, 0.0, 0.0), vec3<f32>(s.y, 0.0, 0.0), on_y),
        vec3<f32>(0.0, 0.0, s.x),
        on_x,
    );
    var up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), on_y);
    // +X, -X, +Y, -Y, +Z, -Z: the negative side is the odd layer.
    let negative = dot(forward_axis, vec3<f32>(1.0)) < 0.0;
    var face = select(select(4, 2, on_y), 0, on_x) + select(0, 1, negative);
    var half_tan = light.point_shadow.z;
    // A spotlight's one view, down its axis (up +Y, or +Z when it points
    // nearly straight up or down); its half field of view is the cone's
    // edge plus a margin: tan(a + b) from the cone's cosine.
    let spot = p.cone.w > 0.0;
    let axis = p.cone.xyz;
    let spot_up0 = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(axis.y) > 0.99);
    let spot_side = cross(axis, spot_up0); // zero for point lights
    let spot_right = spot_side * inverseSqrt(max(dot(spot_side, spot_side), 1e-12));
    let cos_edge = clamp(-p.shadow.w / max(p.cone.w, 1e-6), 0.05, 1.0);
    let tan_edge = sqrt(1.0 - cos_edge * cos_edge) / cos_edge;
    let denom = 1.0 - tan_edge * TAN_SPOT_SHADOW_MARGIN;
    let spot_tan = select(
        TAN_SPOT_SHADOW_MAX,
        min((tan_edge + TAN_SPOT_SHADOW_MARGIN) / denom, TAN_SPOT_SHADOW_MAX),
        denom > 1e-3,
    );
    forward_axis = select(forward_axis, axis, spot);
    right = select(right, spot_right, spot);
    up = select(up, cross(spot_right, axis), spot);
    face = select(face, 0, spot);
    half_tan = select(half_tan, spot_tan, spot);

    let l = -to_frag * inverseSqrt(max(dot(to_frag, to_frag), 1e-8));
    let texel_world = 2.0 * max(dot(to_frag, forward_axis), 0.0) * half_tan * light.point_shadow.x;
    let d = to_frag + n * (texel_world * 1.5) + l * texel_world;
    let forward = max(dot(d, forward_axis), 1e-6);
    let ndc = vec2<f32>(dot(d, right), dot(d, up)) / (forward * half_tan);
    let uv = clamp(ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    // Depth as the view's perspective (D3D-style, 0 at near) writes it.
    let near = p.shadow.y;
    let far = p.shadow.z;
    let depth = far * (forward - near) / ((far - near) * forward);
    return vec4<f32>(uv, f32(first + face), depth);
}

// Forward path: how much of point light `i` reaches `world` past the
// objects between (1 lit, 0 blocked). High takes one bilinear compare,
// Ultra a 2x2 box of them. (No constant-array lookups here: those copy the
// whole array per call on some compilers, and this runs per light per
// pixel.)
fn point_shadow(i: u32, world: vec3<f32>, n: vec3<f32>) -> f32 {
    let c = point_shadow_coords(i, world, n);
    let layer = i32(c.z);
    if layer < 0 {
        return 1.0;
    }
    if light.point_shadow.y < 1.5 {
        return textureSampleCompareLevel(point_shadow_map, shadow_sampler, c.xy, layer, c.w);
    }
    let h = light.point_shadow.x * 0.5;
    return (textureSampleCompareLevel(point_shadow_map, shadow_sampler, c.xy + vec2<f32>(-h, -h), layer, c.w)
        + textureSampleCompareLevel(point_shadow_map, shadow_sampler, c.xy + vec2<f32>(h, -h), layer, c.w)
        + textureSampleCompareLevel(point_shadow_map, shadow_sampler, c.xy + vec2<f32>(-h, h), layer, c.w)
        + textureSampleCompareLevel(point_shadow_map, shadow_sampler, c.xy + vec2<f32>(h, h), layer, c.w)) * 0.25;
}

// Beautiful: 8 filtered taps over ~3 texels.
fn point_shadow_soft(i: u32, world: vec3<f32>, n: vec3<f32>) -> f32 {
    let c = point_shadow_coords(i, world, n);
    let layer = i32(c.z);
    if layer < 0 {
        return 1.0;
    }
    let spread = light.point_shadow.x * 1.5;
    var lit = 0.0;
    for (var t = 0; t < 8; t = t + 1) {
        lit = lit + textureSampleCompareLevel(point_shadow_map, shadow_sampler,
            c.xy + POISSON[t] * spread, layer, c.w);
    }
    return lit / 8.0;
}

// Hemispheric ambient: sky from above, ground from below.
fn hemi(n: vec3<f32>) -> vec3<f32> {
    return mix(light.ground_tonemap.rgb, light.sky_exposure.rgb, n.y * 0.5 + 0.5);
}

// --- Sky dome ---------------------------------------------------------------
// One sky for everything: the dome behind the scene (sky.wgsl adds the sun
// disk and clouds), reflections, and fog, so distant things fade into the
// horizon behind them. The horizon is the clear color, the top
// `sky_zenith`; around the sun a halo of three exponential lobes (wide
// haze, glow, tight corona) that warms and spreads as the sun gets low.

fn sky_on() -> bool {
    return light.sky_zenith.a > 0.5;
}

fn sun_dir() -> vec3<f32> {
    return normalize(-light.direction.xyz);
}

// exp(k (cos - 1)): 1 at the sun, falling off with angle; cheaper than pow.
fn lobe(mu: f32, k: f32) -> f32 {
    return exp(k * (mu - 1.0));
}

// How low the sun is: 0 high in the sky, 1 at (and below) the horizon.
fn sun_low() -> f32 {
    return 1.0 - clamp(sun_dir().y * 3.0, 0.0, 1.0);
}

// The sunlight's tint in the sky: reddens toward sunset.
fn sun_tint() -> vec3<f32> {
    let low = sun_low();
    return light.color_ambient.rgb * mix(vec3<f32>(1.0), vec3<f32>(1.0, 0.55, 0.28), low * low);
}

// The sky along `d` without the sun disk or clouds.
fn sky_dome(d: vec3<f32>) -> vec3<f32> {
    let horizon = light.background.rgb;
    let up = clamp(d.y, 0.0, 1.0);
    // A thin bright band at the horizon, deepening upward.
    var c = mix(horizon, light.sky_zenith.rgb, sqrt(sqrt(up)) * (2.0 - sqrt(sqrt(up))));
    let l = sun_dir();
    let mu = dot(d, l);
    let glow = light.sky_sun.y;
    let above = smoothstep(-0.25, 0.05, l.y); // the halo sets with the sun
    let low = sun_low();
    let tint = sun_tint();
    let edge = 1.0 - up;
    let edge4 = edge * edge * edge * edge;
    c = c + tint * glow * above * (
        lobe(mu, 6.0) * (0.12 + 0.35 * low * edge4)
        + lobe(mu, 60.0) * 0.35
        + lobe(mu, 900.0) * 1.2);
    if d.y < 0.0 {
        // Below the horizon: fade to a dim ground (only seen in reflections
        // and over the edge of the world).
        let ground = light.ground_tonemap.rgb * light.color_ambient.a;
        c = mix(c, ground, 1.0 - exp(d.y * 12.0));
    }
    return c;
}

// The fog's color looking along `d`: the horizon in that direction (with
// the sun's glow), so fogged geometry melts into the sky behind it.
fn fog_tint(d: vec3<f32>) -> vec3<f32> {
    if !sky_on() {
        return light.fog.rgb;
    }
    let flat_dir = normalize(vec3<f32>(d.x, 0.03, d.z) + vec3<f32>(0.0, max(d.y, 0.0) * 0.3, 0.0));
    return sky_dome(flat_dir);
}

// The sky seen along `d`, for reflections: horizon (the clear color) up to
// the sky ambient color, down to the ground color.
fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    if sky_on() {
        return sky_dome(d);
    }
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

// GGX specular + energy-conserving diffuse for one light from `l`, times
// the cosine term (multiply by the light's radiance).
fn brdf_direct(
    albedo: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    roughness: f32,
    f0: vec3<f32>,
) -> vec3<f32> {
    let ndotl = max(dot(n, l), 0.0);
    if ndotl <= 0.0 {
        return vec3<f32>(0.0);
    }
    let h = normalize(l + v);
    let ndotv = max(dot(n, v), 1e-4);
    let ndoth = max(dot(n, h), 0.0);
    let vdoth = max(dot(v, h), 0.0);
    let f = fresnel_schlick(vdoth, f0);
    let spec = d_ggx(ndoth, roughness) * g_smith(ndotv, ndotl, roughness) * f
        / max(4.0 * ndotv * ndotl, 1e-4);
    let kd = vec3<f32>(1.0) - f;
    return (kd * albedo + spec * PI) * ndotl;
}

// Every point light on a surface, physically based (Beautiful).
fn points_pbr(
    world: vec3<f32>,
    albedo: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    roughness: f32,
    f0: vec3<f32>,
) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < point_count(); i = i + 1u) {
        let p = point_light(i, world);
        if dot(n, p.l) > 0.0 && any(p.radiance > vec3<f32>(0.0)) {
            sum = sum + brdf_direct(albedo, n, v, p.l, roughness, f0) * p.radiance
                * point_shadow_soft(i, world, n);
        }
    }
    return sum;
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
    ao: f32,
) -> vec3<f32> {
    let ndotv = max(dot(n, v), 1e-4);
    let direct = brdf_direct(albedo, n, v, l, roughness, f0) * light.color_ambient.rgb * shadow;

    // Ambient: diffuse from the hemisphere, specular from the sky, weighted
    // by the environment BRDF so rough surfaces don't turn into mirrors.
    let fr = env_brdf(f0, roughness, ndotv);
    let env = sky_radiance(reflect(-v, n));
    // Reflections of the sky are partly blocked in shadowed creases, and
    // all ambient light by nearby geometry (`ao`, 1 = open).
    let occlusion = mix(0.6, 1.0, min(shadow, 1.0)) * ao;
    let ambient = (albedo * hemi(n) * (vec3<f32>(1.0) - fr) * ao + env * fr * occlusion)
        * light.color_ambient.a;
    return direct + ambient;
}

// --- Under water -------------------------------------------------------------
// When the camera dips into water (`underwater.a` > 0), pixels whose bit of
// the lens is below the surface see through water instead of air: light
// is absorbed (red first) and scattered toward the water's colour with
// distance. The surface cuts across the lens in a gently wobbling
// waterline. Below any water's rest height, the sun dances in caustics.

// The world point on the near plane under pixel `frag`.
fn near_point(frag: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(frag.x / light.screen.x * 2.0 - 1.0, 1.0 - frag.y / light.screen.y * 2.0);
    let h = light.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return h.xyz / h.w;
}

// Whether pixel `frag` looks out from under the water.
fn eye_under_water(frag: vec2<f32>) -> bool {
    if light.underwater.a <= 0.0 {
        return false;
    }
    let u = frag.x / light.screen.x;
    let t = light.fog_params.y;
    let wobble = light.water_eye.y * (0.14 * sin(u * 7.0 + t * 1.9) + 0.06 * sin(u * 19.0 - t * 3.1));
    return near_point(frag).y < light.water_eye.x + wobble;
}

// The water's own colour around the camera: lit from above, darker the
// deeper the camera is.
fn water_fog_color() -> vec3<f32> {
    let l = sun_dir();
    let lit = hemi(vec3<f32>(0.0, 1.0, 0.0)) * light.color_ambient.a
        + light.color_ambient.rgb * max(l.y, 0.0) * 0.6;
    let depth = max(light.water_eye.x - camera.view_pos.y, 0.0);
    return light.underwater.rgb * lit * exp(-depth * light.underwater.a * 0.5);
}

// How much of `dist` metres of water lets through, per channel.
fn water_transmission(dist: f32) -> vec3<f32> {
    return exp(-light.underwater.a * vec3<f32>(2.2, 1.0, 0.8) * max(dist, 0.0));
}

// `lit` seen through `dist` metres of water.
fn water_fog(lit: vec3<f32>, dist: f32) -> vec3<f32> {
    let t = water_transmission(dist);
    return lit * t + water_fog_color() * (vec3<f32>(1.0) - t);
}

// How far the view ray to `world` runs under water, the camera being in
// it: all the way, or up to the surface for things above it.
fn water_path(world: vec3<f32>) -> f32 {
    let eye = camera.view_pos.xyz;
    let dist = length(world - eye);
    let level = light.water_eye.x;
    let rise = world.y - eye.y;
    if world.y <= level || rise <= 1e-4 {
        return dist;
    }
    return dist * clamp((level - eye.y) / rise, 0.0, 1.0);
}

// The tileable caustic net (after Dave Hoskins' "Tileable Water Caustic"):
// thin bright lines that drift and re-form, about 0 to 1 (mostly dark).
fn caustic_pattern(uv: vec2<f32>, time: f32) -> f32 {
    let p = fract(uv) * 6.2831853 - vec2<f32>(250.0);
    var i = p;
    var c = 1.0;
    let inten = 0.005;
    for (var n = 0; n < 4; n = n + 1) {
        let t = time * (1.0 - 3.5 / f32(n + 1));
        i = p + vec2<f32>(cos(t - i.x) + sin(t + i.y), sin(t - i.y) + cos(t + i.x));
        c = c + 1.0 / length(vec2<f32>(p.x / (sin(i.x + t) / inten), p.y / (cos(i.y + t) / inten)));
    }
    c = 1.17 - pow(c * 0.25, 1.4);
    let c2 = c * c;
    let c4 = c2 * c2;
    return clamp(c4 * c4, 0.0, 4.0);
}

// Light reaching `world` through any water above it: rgb tints all of it
// (the water swallows red first, then green, with depth), a scales the
// sun's: a moving net of brighter and darker light (caustics, about 1 on
// average) fading with depth. (1, 1, 1, 1) in the dry.
fn water_light(world: vec3<f32>) -> vec4<f32> {
    let count = u32(light.water_eye.z);
    for (var i = 0u; i < count; i = i + 1u) {
        let r = light.water_rects[i];
        if world.x < r.x || world.z < r.y || world.x > r.z || world.z > r.w {
            continue;
        }
        let w = light.water_levels[i];
        let depth = w.x - world.y;
        if depth <= 0.0 {
            continue;
        }
        let tint = exp(-(0.35 / w.z) * vec3<f32>(2.2, 1.0, 0.8) * depth);
        let l = sun_dir();
        if l.y <= 0.02 {
            return vec4<f32>(tint, 1.0);
        }
        // Where the sunbeam reaching this point came through the surface.
        let d = refract(-l, vec3<f32>(0.0, 1.0, 0.0), 0.75);
        let entry = world.xz + d.xz * (depth / max(-d.y, 0.1));
        let net = caustic_pattern(entry * 0.45, light.fog_params.y * 0.55);
        let fade = smoothstep(0.0, 0.3, depth) * exp(-depth / (w.z * 3.0)) * w.y;
        // Strength over 1 deepens the contrast.
        return vec4<f32>(tint, max(1.0 + (0.35 + 3.2 * net - 1.0) * fade, 0.0));
    }
    return vec4<f32>(1.0);
}

// The mean of the height fog's density profile (1 at and below the
// layer's top, thinning by exp(-falloff * height) above it) along a
// straight line between heights `a` and `b` above the layer's top.
fn height_fog_mean(a: f32, b: f32, falloff: f32) -> f32 {
    let lo = min(a, b);
    let hi = max(a, b);
    if hi <= 0.0 {
        return 1.0;
    }
    let span = hi - lo;
    if span < 1e-3 {
        return exp(-falloff * max(lo, 0.0));
    }
    let below = max(-lo, 0.0);
    let above = (exp(-falloff * max(lo, 0.0)) - exp(-falloff * hi)) / falloff;
    return (below + above) / span;
}

// How much fog lies between the camera and `world` in dry air (0 none,
// 1 all): exponential in the distance past `fog_start`, and with a
// falloff, thick at and below the layer's top and thinning above it
// (height fog, integrated exactly along the view ray).
fn fog_cover(world: vec3<f32>) -> f32 {
    let density = light.fog.a;
    if density <= 0.0 {
        return 0.0;
    }
    let to_frag = world - camera.view_pos.xyz;
    let len = length(to_frag);
    let dist = max(len - light.fog_params.x, 0.0);
    var depth = density * dist;
    let falloff = light.fog_height.y;
    if falloff > 0.0 && dist > 0.0 {
        let h1 = world.y - light.fog_height.x;
        let h0 = h1 - to_frag.y * dist / max(len, 1e-4);
        depth = depth * height_fog_mean(h0, h1, falloff);
    }
    return clamp(1.0 - exp(-depth), 0.0, 1.0);
}

// Sunlight scattered toward the eye by fog seen along `d`: a glow around
// the sun's direction (`fog_height.z` sets how much).
fn fog_sunlight(d: vec3<f32>) -> vec3<f32> {
    let k = light.fog_height.z;
    if k <= 0.0 {
        return vec3<f32>(0.0);
    }
    let mu = dot(d, sun_dir());
    return sun_tint() * k * (0.6 * lobe(mu, 6.0) + 0.15 * lobe(mu, 1.5));
}

// Distance and height fog (before tone mapping, like real scattering),
// or the water's when pixel `frag` looks out from under it.
fn apply_fog(lit: vec3<f32>, world: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    if eye_under_water(frag) {
        return water_fog(lit, water_path(world));
    }
    let f = fog_cover(world);
    if f <= 0.0 {
        return lit;
    }
    let to_frag = world - camera.view_pos.xyz;
    let d = to_frag / max(length(to_frag), 1e-4);
    return mix(lit, fog_tint(d) + fog_sunlight(d), f);
}

// Beautiful's fog adds sun scattering: it glows toward the sun.
fn apply_fog_scatter(lit: vec3<f32>, world: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    if eye_under_water(frag) {
        return water_fog(lit, water_path(world));
    }
    let f = fog_cover(world);
    if f <= 0.0 {
        return lit;
    }
    let to_frag = world - camera.view_pos.xyz;
    let d = to_frag / max(length(to_frag), 1e-4);
    if sky_on() {
        return mix(lit, fog_tint(d) + fog_sunlight(d), f);
    }
    let l = normalize(-light.direction.xyz);
    let s2 = max(dot(d, l), 0.0);
    let s4 = s2 * s2;
    let s8 = s4 * s4;
    let s = s8 * s8;
    let color = light.fog.rgb + light.color_ambient.rgb * s * 0.5 + fog_sunlight(d);
    return mix(lit, color, f);
}

// The forward path tone maps in the lit shaders (Beautiful writes HDR and
// the post chain tone maps after bloom).
fn finish_forward(c: vec3<f32>) -> vec3<f32> {
    return tonemap_with(c, light.sky_exposure.a, light.ground_tonemap.a);
}
