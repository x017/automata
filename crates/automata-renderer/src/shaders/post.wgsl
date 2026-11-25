// Post chains. Prepended at build time: tonemap.wgsl.
//
// Beautiful:
//   1. bloom: a prefilter (Karis average against fireflies and a soft-knee
//      threshold, so only highlights glow), then a mip chain of dual-filter
//      downsamples and upsamples added back up (Bjorge, "Bandwidth-Efficient
//      Rendering", SIGGRAPH 2015): 4-5 fetches per pixel per pass
//   2. god rays and depth of field when the Environment asks for them:
//      a quarter-size mask of the sky around the sun, blurred toward the
//      sun into light shafts; a half-size disc blur of the scene sized by
//      each pixel's distance from the focus
//   3. resolve: HDR + bloom + rays (over the depth-of-field blur where it's
//      out of focus), exposure, tone mapping -> display-encoded LDR
//      with luma in alpha. The HDR image's alpha is surface coverage: the
//      background (alpha 0) skips exposure and the curve, exactly like the
//      forward path, so the clear color shows as authored.
//      Colour grading, vignette, grain and chromatic aberration happen
//      here too, after tone mapping.
//   4. fxaa: edge anti-aliasing on the LDR image, written to the target
//
// Medium to Ultra (the forward path renders display-ready colour into an
// intermediate image): on High and Ultra a cheap bloom (ldr_prefilter into
// a quarter-size mip chain, then the same down / up passes), and one
// grade pass into the target with the same grading and lens effects.
//
// Every pass draws one fullscreen triangle with the same bind group shape.

struct PostUniform {
    texel: vec4<f32>,  // xy: 1 / source size
    params: vec4<f32>, // x: unused, y: exposure, z: tone mapping, w: bloom
    output: vec4<f32>, // x: encode sRGB in the shader (the target doesn't)
    sun: vec4<f32>,    // xy: sun position (uv), z: lens flare strength, w: disk radius (uv)
    sun_color: vec4<f32>, // rgb: sunlight, a: aspect ratio
    grade: vec4<f32>,  // x: contrast, y: saturation, z: temperature, w: tint
    lift: vec4<f32>,   // rgb: lift, a: vignette
    gamma: vec4<f32>,  // rgb: gamma, a: film grain
    gain: vec4<f32>,   // rgb: gain, a: chromatic aberration
    misc: vec4<f32>,   // x: time (s), y: cheap bloom strength, z: 1 = grade
    lens: vec4<f32>,   // x: god rays, y: depth of field, z: focus distance (m), w: focus range (m)
    depth: vec4<f32>,  // x: near, y: far (m), zw: full-size viewport (px)
    style: vec4<f32>,  // x: outline width (px), y: pixel block (px, 1 = off), z: palette levels (0 = off)
    outline_color: vec4<f32>, // rgb: outline colour
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> post: PostUniform;
@group(0) @binding(3) var extra: texture_2d<f32>; // resolve: the bloom
@group(0) @binding(4) var extra2: texture_2d<f32>; // resolve: sun visibility (1x1)
@group(0) @binding(5) var extra3: texture_2d<f32>; // resolve: god rays
@group(0) @binding(6) var extra4: texture_2d<f32>; // resolve: depth of field
// The scene's depth (full size), bound as unfilterable float: depth
// textures can't be read with textureLoad in GLSL (WebGL2).
@group(0) @binding(7) var scene_depth: texture_2d<f32>;

struct FullscreenOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FullscreenOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FullscreenOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, src_sampler, uv, 0.0).rgb;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn karis(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + luma(c));
}

// Bloom's soft-knee threshold: what's above THRESHOLD glows, fading in
// over KNEE below it.
// Above display white, so sunlit surfaces and bright skies stay crisp and
// only real highlights (sun glints, hot spots) glow.
const THRESHOLD: f32 = 1.1;
const KNEE: f32 = 0.4;

fn prefilter(c: vec3<f32>) -> vec3<f32> {
    let bright = max(c.r, max(c.g, c.b));
    var soft = clamp(bright - THRESHOLD + KNEE, 0.0, 2.0 * KNEE);
    soft = soft * soft / (4.0 * KNEE + 1e-4);
    return c * (max(soft, bright - THRESHOLD) / max(bright, 1e-4));
}

// Bloom prefilter: the full-resolution scene into the first (half-size)
// mip. Four bilinear taps cover the 4x4 source block; each is weighted by
// its Karis average so lone bright pixels don't bloom into flickering
// blobs, then the soft threshold keeps only highlights.
@fragment
fn fs_prefilter(in: FullscreenOut) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let a = tap(in.uv + t * vec2<f32>(-1.0, -1.0));
    let b = tap(in.uv + t * vec2<f32>(1.0, -1.0));
    let c = tap(in.uv + t * vec2<f32>(-1.0, 1.0));
    let d = tap(in.uv + t * vec2<f32>(1.0, 1.0));
    let wa = karis(a);
    let wb = karis(b);
    let wc = karis(c);
    let wd = karis(d);
    let sum = (a * wa + b * wb + c * wc + d * wd) / max(wa + wb + wc + wd, 1e-4);
    return vec4<f32>(prefilter(sum), 1.0);
}

// Bloom downsample (dual filter, Bjorge 2015): the center plus four
// diagonal bilinear taps — 5 fetches covering a 4x4 footprint.
@fragment
fn fs_downsample(in: FullscreenOut) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    var color = tap(in.uv) * 4.0;
    color = color + tap(in.uv + t * vec2<f32>(-1.0, -1.0));
    color = color + tap(in.uv + t * vec2<f32>(1.0, -1.0));
    color = color + tap(in.uv + t * vec2<f32>(-1.0, 1.0));
    color = color + tap(in.uv + t * vec2<f32>(1.0, 1.0));
    return vec4<f32>(color / 8.0, 1.0);
}

// Bloom upsample: four diagonal bilinear taps (a smooth tent over the
// smaller mip), added onto the next larger mip by the blend state.
@fragment
fn fs_upsample(in: FullscreenOut) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    var color = tap(in.uv + t * vec2<f32>(-1.0, -1.0));
    color = color + tap(in.uv + t * vec2<f32>(1.0, -1.0));
    color = color + tap(in.uv + t * vec2<f32>(-1.0, 1.0));
    color = color + tap(in.uv + t * vec2<f32>(1.0, 1.0));
    return vec4<f32>(color * 0.25, 1.0);
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// --- Lens flare ---------------------------------------------------------------
// A 1x1 pass measures how much of the sun's disk is still blazing in the
// HDR image (the sky draws it ~40x brighter than anything lit): taps over
// the disk, each counting by how bright it is, so geometry blocks the
// flare and passing clouds dim it. The resolve then adds glare, a
// starburst, ghosts mirrored through the screen center and a faint halo
// ring, all scaled by that visibility.

@fragment
fn fs_sun_visibility(in: FullscreenOut) -> @location(0) vec4<f32> {
    let center = post.sun.xy;
    let r = post.sun.w * vec2<f32>(1.0 / post.sun_color.a, 1.0) * 0.8;
    var sum = 0.0;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * 0.5;
            let uv = center + o * r;
            let inside = uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
            let l = luma(textureSampleLevel(src, src_sampler, uv, 0.0).rgb);
            sum = sum + select(0.0, clamp((l - 4.0) / 12.0, 0.0, 1.0), inside);
        }
    }
    return vec4<f32>(vec3<f32>(sum / 25.0), 1.0);
}

// A soft disc of radius `r` (aspect-corrected distance `d`).
fn ghost(d: f32, r: f32) -> f32 {
    let x = clamp(1.0 - d / r, 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x);
}

fn lens_flare(uv: vec2<f32>) -> vec3<f32> {
    let strength = post.sun.z;
    if strength <= 0.0 {
        return vec3<f32>(0.0);
    }
    let vis = textureSampleLevel(extra2, src_sampler, vec2<f32>(0.5), 0.0).r;
    if vis <= 0.001 {
        return vec3<f32>(0.0);
    }
    let aspect = vec2<f32>(post.sun_color.a, 1.0);
    let sun = post.sun.xy;
    let to = (uv - sun) * aspect;
    let dist = length(to);
    let tint = post.sun_color.rgb;
    // Glare: a wide soft glow and a hot core around the sun.
    var c = tint * (exp(-dist * 6.0) * 0.35 + exp(-dist * 28.0) * 1.2);
    // Starburst: six thin streaks.
    let angle = atan2(to.y, to.x);
    let streak = pow(abs(cos(angle * 3.0)), 60.0) + pow(abs(cos(angle * 3.0 + 1.5708)), 90.0) * 0.5;
    c = c + tint * streak * exp(-dist * 7.0) * 0.6;
    // Ghosts along the line from the sun through the screen center.
    let axis = (vec2<f32>(0.5) - sun);
    let ghosts = array<vec4<f32>, 5>(
        // x: position along the axis, y: radius, zw: unused (tint below)
        vec4<f32>(0.55, 0.035, 0.0, 0.0),
        vec4<f32>(1.15, 0.07, 0.0, 0.0),
        vec4<f32>(1.45, 0.02, 0.0, 0.0),
        vec4<f32>(1.9, 0.11, 0.0, 0.0),
        vec4<f32>(2.35, 0.05, 0.0, 0.0),
    );
    let tints = array<vec3<f32>, 5>(
        vec3<f32>(0.9, 0.6, 0.3),
        vec3<f32>(0.3, 0.6, 1.0),
        vec3<f32>(1.0, 0.9, 0.5),
        vec3<f32>(0.4, 1.0, 0.6),
        vec3<f32>(0.9, 0.4, 0.8),
    );
    for (var k = 0; k < 5; k = k + 1) {
        let p = sun + axis * ghosts[k].x;
        let d = length((uv - p) * aspect);
        c = c + tints[k] * tint * ghost(d, ghosts[k].y) * 0.15;
    }
    // A faint rainbow halo ring around the sun.
    let ring = dist - 0.42;
    let halo = exp(-ring * ring * 900.0);
    let rainbow = vec3<f32>(
        exp(-(ring - 0.012) * (ring - 0.012) * 2500.0),
        exp(-ring * ring * 2500.0),
        exp(-(ring + 0.012) * (ring + 0.012) * 2500.0));
    c = c + tint * (halo * 0.03 + rainbow * 0.03);
    return c * vis * strength;
}

// --- Grading and lens effects -------------------------------------------------
// On display-range linear colour (after tone mapping): white balance, then
// in a perceptual (gamma 2.2) space lift / gamma / gain, contrast around
// mid grey and saturation, the vignette and grain; linear out.

fn white_balance(temperature: f32, tint: f32) -> vec3<f32> {
    let wb = vec3<f32>(
        1.0 + 0.32 * temperature + 0.06 * tint,
        1.0 - 0.2 * tint,
        1.0 - 0.32 * temperature + 0.06 * tint,
    );
    return wb / luma(wb); // keeps the brightness
}

fn grain_noise(px: vec2<f32>) -> f32 {
    // A new pattern 24 times a second.
    let frame = floor(post.misc.x * 24.0);
    let p = fract(px * vec2<f32>(0.1031, 0.1030) + vec2<f32>(frame * 0.6180, frame * 0.4142));
    let q = p + dot(p, p.yx + 33.33);
    return fract((q.x + q.y) * q.x) - 0.5;
}

fn grade(c: vec3<f32>, uv: vec2<f32>, px: vec2<f32>) -> vec3<f32> {
    let balanced = max(c, vec3<f32>(0.0)) * white_balance(post.grade.z, post.grade.w);
    var g = pow(balanced, vec3<f32>(1.0 / 2.2));
    g = g * post.gain.rgb + post.lift.rgb * (vec3<f32>(1.0) - g);
    g = pow(max(g, vec3<f32>(0.0)), vec3<f32>(1.0) / max(post.gamma.rgb, vec3<f32>(0.05)));
    g = (g - 0.5) * post.grade.x + 0.5;
    g = mix(vec3<f32>(luma(g)), g, post.grade.y);
    // Vignette: darker toward the corners (an ellipse matching the frame).
    let d = length(uv - 0.5) * 1.4142;
    g = g * (1.0 - post.lift.a * smoothstep(0.3, 1.05, d));
    // Grain: strongest in the midtones, like film.
    let l = clamp(luma(g), 0.0, 1.0);
    g = g + grain_noise(px) * post.gamma.a * 0.16 * (0.35 + 2.6 * l * (1.0 - l));
    return pow(clamp(g, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

// The source with colour fringes toward the edges: red pushed out, blue
// pulled in (chromatic aberration). Plain when it's off.
fn fringed(uv: vec2<f32>) -> vec3<f32> {
    let center = tap(uv);
    let k = post.gain.a;
    if k <= 0.0 {
        return center;
    }
    let off = (uv - 0.5) * k * 0.014;
    return vec3<f32>(tap(uv + off).r, center.g, tap(uv - off).b);
}

// --- Styles: outlines, pixelation, palette ----------------------------------

// 1 / view depth at pixel `p` (clamped to the image): it changes linearly
// across a flat surface on screen, so it's what edge detection compares.
fn inv_depth_at(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(scene_depth));
    let d = textureLoad(scene_depth, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0).x;
    return 1.0 / view_depth(d);
}

// Outlines, blended over the scene: a second difference of 1 / depth is
// zero across any flat surface, so what's left marks silhouettes (a jump)
// and creases (a change of slope). Measured against the depth itself, so
// near and far objects get the same lines; faded out far away.
@fragment
fn fs_outline(in: FullscreenOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let r = i32(max(round(post.style.x * 0.5), 1.0));
    let c = inv_depth_at(p);
    let n = inv_depth_at(p + vec2<i32>(0, -r));
    let s = inv_depth_at(p + vec2<i32>(0, r));
    let e = inv_depth_at(p + vec2<i32>(r, 0));
    let w = inv_depth_at(p + vec2<i32>(-r, 0));
    let nearest = max(c, max(max(n, s), max(e, w)));
    let bend = max(abs(n + s - 2.0 * c), abs(e + w - 2.0 * c)) / max(nearest, 1e-6);
    // Thin lines get faint rather than vanishing below a pixel.
    let width = clamp(post.style.x, 0.0, 1.0);
    // Faded out from 80 m (or half the view) to 160 m (or most of it).
    let reach = min(post.depth.y * 0.8, 160.0);
    let far_fade = 1.0 - smoothstep(reach * 0.5, reach, 1.0 / nearest);
    let edge = smoothstep(0.012, 0.04, bend) * width * far_fade;
    return vec4<f32>(post.outline_color.rgb, edge);
}

// `uv` moved to the centre of its pixel block (pixelation), or as it is.
fn pixel_snap(uv: vec2<f32>) -> vec2<f32> {
    let b = post.style.y;
    if b <= 1.0 {
        return uv;
    }
    // The texel nearest the block's centre, so it reads one pixel, not a
    // blend of its neighbours.
    let full = post.depth.zw;
    return (floor(uv * full / b) * b + floor(b * 0.5) + 0.5) / full;
}

// Pixel `frag` moved to the corner of its block, so per-pixel noise
// (grain, dither) is per block too.
fn pixel_frag(frag: vec2<f32>) -> vec2<f32> {
    let b = max(post.style.y, 1.0);
    return floor(frag / b) * b;
}

const BAYER: array<f32, 16> = array<f32, 16>(
    0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0,
    3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0,
);

// The palette: each channel limited to `levels` steps (spaced evenly to
// the eye), with an ordered dither between neighbouring steps.
fn quantize(c: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let levels = post.style.z;
    if levels < 2.0 {
        return c;
    }
    let cell = vec2<u32>(floor(frag / max(post.style.y, 1.0))) % vec2<u32>(4u);
    let dither = (BAYER[cell.y * 4u + cell.x] + 0.5) / 16.0;
    let steps = levels - 1.0;
    let q = floor(sqrt(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))) * steps + dither) / steps;
    return q * q;
}

// --- God rays ----------------------------------------------------------------
// Light shafts (after Mitchell, "Volumetric Light Scattering as a
// Post-Process", GPU Gems 3): a quarter-size mask of the open sky, glowing
// toward the sun (geometry blocks it), then each pixel sums the mask along
// the line toward the sun with a decay, so light streams through the gaps
// between leaves and pillars and shadowed shafts trail behind them. The
// resolve tints the result with the sunlight.

// The scene's depth at `uv`, clamped to the image.
fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(scene_depth));
    let p = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(scene_depth, p, 0).x;
}

@fragment
fn fs_rays_mask(in: FullscreenOut) -> @location(0) vec4<f32> {
    // Four depth reads over the 4x4 block this pixel covers.
    let t = post.texel.xy;
    var sky = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<f32>(f32(k & 1), f32(k >> 1u)) * 2.0 - 1.0;
        sky = sky + select(0.0, 0.25, depth_at(in.uv + o * t * 0.375) >= 1.0);
    }
    if sky <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    // Open sky glows by its distance from the sun.
    let d = length((in.uv - post.sun.xy) * vec2<f32>(post.sun_color.a, 1.0));
    return vec4<f32>(vec3<f32>(sky * exp(-d * 10.0)), 1.0);
}

const RAY_TAPS: i32 = 40;
const RAY_REACH: f32 = 0.6;

@fragment
fn fs_rays(in: FullscreenOut) -> @location(0) vec4<f32> {
    // At most RAY_REACH screen heights toward the sun, so far corners
    // don't gather the glow around it (shafts stay near the sun).
    let to_sun = post.sun.xy - in.uv;
    let len = length(to_sun * vec2<f32>(post.sun_color.a, 1.0));
    let step = to_sun * min(1.0, RAY_REACH / max(len, 1e-4)) / f32(RAY_TAPS);
    // A per-pixel offset along the line trades banding for fine noise.
    let jitter = fract(52.9829189 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))));
    var uv = in.uv + step * jitter;
    var decay = 1.0;
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < RAY_TAPS; k = k + 1) {
        let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
        sum = sum + select(vec3<f32>(0.0), tap(uv), inside) * decay;
        decay = decay * 0.975;
        uv = uv + step;
    }
    return vec4<f32>(sum / f32(RAY_TAPS), 1.0);
}

// --- Depth of field --------------------------------------------------------
// Each pixel's circle of confusion grows with its distance from the focus
// (past the sharp range around it), as a fraction of the screen height.
// A half-size pass gathers a disc of that size from the full-size scene;
// samples nearer than the pixel only count as far as their own blur
// reaches (so a sharp subject doesn't smear into the blurred background),
// and bright highlights weigh more, so they open into bokeh discs.

fn view_depth(d: f32) -> f32 {
    let near = post.depth.x;
    let far = post.depth.y;
    return near * far / max(far - d * (far - near), 1e-6);
}

// Blur diameter in full-size pixels at view depth `z`.
fn coc_px(z: f32) -> f32 {
    let blur = max(abs(z - post.lens.z) - post.lens.w, 0.0) / max(z, 1e-3);
    return min(post.lens.y * blur * post.depth.w * 0.02, post.depth.w * 0.03);
}

const DOF_TAPS: i32 = 32;

@fragment
fn fs_dof(in: FullscreenOut) -> @location(0) vec4<f32> {
    let full = post.depth.zw;
    // Taps sit on pixel centres so colour and depth come from one pixel
    // (a filtered tap across an edge would smear a sharp object outward).
    let here = (floor(in.uv * full) + 0.5) / full;
    let zc = view_depth(depth_at(here));
    let cc = coc_px(zc);
    let center = tap(here);
    // Alpha keeps the circle of confusion for the depth-aware upsample.
    if cc < 0.5 {
        return vec4<f32>(center, cc);
    }
    let px = 1.0 / post.depth.zw;
    let radius = cc * 0.5;
    var sum = center;
    var weight = 1.0;
    for (var k = 0; k < DOF_TAPS; k = k + 1) {
        let r = sqrt((f32(k) + 0.5) / f32(DOF_TAPS)) * radius;
        let a = f32(k) * 2.39996323;
        let uv = (floor((here + vec2<f32>(cos(a), sin(a)) * r * px) * full) + 0.5) / full;
        let zs = view_depth(depth_at(uv));
        let cs = coc_px(zs);
        let reach = select(cc, cs, zs < zc) * 0.5;
        // A nearer, sharper sample only spreads as far as its own blur,
        // so a far background doesn't halo an in-focus object.
        let w = clamp((reach - r) * 2.0, 0.0, 1.0);
        let c = tap(uv);
        let bright = 1.0 + max(luma(c) - 1.0, 0.0);
        sum = sum + c * w * bright;
        weight = weight + w * bright;
    }
    return vec4<f32>(sum / weight, cc);
}

// The scene at `uv`, over its depth-of-field blur where that's out of
// focus.
fn focused(uv: vec2<f32>) -> vec3<f32> {
    let sharp = fringed(uv);
    if post.lens.y <= 0.0 {
        return sharp;
    }
    let coc = coc_px(view_depth(depth_at(uv)));
    let blend = smoothstep(0.75, 3.0, coc);
    if blend <= 0.0 {
        return sharp;
    }
    // Upsample the half-size blur from the texels whose blur matches this
    // pixel's, so sharp things don't bleed into a blurred background.
    let size = vec2<f32>(textureDimensions(extra4));
    let base = floor(uv * size - 0.5);
    let f = uv * size - 0.5 - base;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<f32>(f32(k & 1), f32(k >> 1u));
        let t = textureSampleLevel(extra4, src_sampler, (base + o + 0.5) / size, 0.0);
        let bilinear = mix(1.0 - f.x, f.x, o.x) * mix(1.0 - f.y, f.y, o.y);
        let w = bilinear / (0.05 + abs(t.a - coc)) + 1e-5;
        sum = sum + t.rgb * w;
        weight = weight + w;
    }
    return mix(sharp, sum / weight, blend);
}

@fragment
fn fs_resolve(in: FullscreenOut) -> @location(0) vec4<f32> {
    let hdr = textureSampleLevel(src, src_sampler, in.uv, 0.0);
    let bloom = textureSampleLevel(extra, src_sampler, in.uv, 0.0).rgb;
    var color = focused(in.uv) + bloom * post.params.w + lens_flare(in.uv);
    if post.lens.x > 0.0 {
        let rays = textureSampleLevel(extra3, src_sampler, in.uv, 0.0).r;
        color = color + post.sun_color.rgb * rays * post.lens.x * 1.5;
    }
    let lit = tonemap_with(color, post.params.y, post.params.z);
    var mapped = clamp(mix(color, lit, hdr.a), vec3<f32>(0.0), vec3<f32>(1.0));
    if post.misc.z > 0.5 {
        mapped = grade(mapped, in.uv, in.position.xy);
    }
    // Stored with gamma 2 (sqrt / square: an exact, cheap inverse pair that
    // spends 8 bits perceptually like sRGB). FXAA reads luma from alpha.
    let encoded = sqrt(mapped);
    return vec4<f32>(encoded, luma(encoded));
}

// Cheap bloom (High, Ultra): the display-ready image into a quarter-size
// mip. Four bilinear taps cover the 4x4 source block, and only what's near
// white glows (the image is already tone mapped, so that's the highlights:
// the sun, glints, lamps, glowing things).
const LDR_THRESHOLD: f32 = 0.82;
const LDR_KNEE: f32 = 0.18;

@fragment
fn fs_ldr_prefilter(in: FullscreenOut) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let c = (tap(in.uv + t * vec2<f32>(-1.0, -1.0)) + tap(in.uv + t * vec2<f32>(1.0, -1.0))
        + tap(in.uv + t * vec2<f32>(-1.0, 1.0)) + tap(in.uv + t * vec2<f32>(1.0, 1.0))) * 0.25;
    let bright = max(c.r, max(c.g, c.b));
    var soft = clamp(bright - LDR_THRESHOLD + LDR_KNEE, 0.0, 2.0 * LDR_KNEE);
    soft = soft * soft / (4.0 * LDR_KNEE + 1e-4);
    return vec4<f32>(c * (max(soft, bright - LDR_THRESHOLD) / max(bright, 1e-4)), 1.0);
}

// Medium to Ultra: the intermediate image (plus the cheap bloom) graded
// into the target. Values are linear either way: an sRGB source decodes on
// read and an sRGB target encodes on write.
@fragment
fn fs_grade(in: FullscreenOut) -> @location(0) vec4<f32> {
    let uv = pixel_snap(in.uv);
    let frag = pixel_frag(in.position.xy);
    var c = fringed(uv);
    c = c + textureSampleLevel(extra, src_sampler, uv, 0.0).rgb * post.misc.y;
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    if post.misc.z > 0.5 {
        c = grade(c, uv, frag);
    }
    return vec4<f32>(quantize(c, frag), 1.0);
}

// FXAA (after Lottes' FXAA 3.11, "quality" variant with a short search).
fn lum(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(src, src_sampler, uv, 0.0).a;
}

@fragment
fn fs_fxaa(in: FullscreenOut) -> @location(0) vec4<f32> {
    if post.style.y > 1.0 || post.style.z >= 2.0 {
        // Pixel art: one sample per block, no anti-aliasing (it would
        // blur the blocks' edges), then the palette.
        let c = textureSampleLevel(src, src_sampler, pixel_snap(in.uv), 0.0).rgb;
        var rgb = quantize(c * c, pixel_frag(in.position.xy));
        if post.output.x > 0.5 {
            rgb = to_srgb(rgb);
        }
        return vec4<f32>(rgb, 1.0);
    }
    let t = post.texel.xy;
    let uv = in.uv;
    let center = textureSampleLevel(src, src_sampler, uv, 0.0);
    let m = center.a;
    let n = lum(uv + vec2<f32>(0.0, -t.y));
    let s = lum(uv + vec2<f32>(0.0, t.y));
    let e = lum(uv + vec2<f32>(t.x, 0.0));
    let w = lum(uv + vec2<f32>(-t.x, 0.0));
    let lo = min(m, min(min(n, s), min(e, w)));
    let hi = max(m, max(max(n, s), max(e, w)));
    let range = hi - lo;
    var rgb = center.rgb;
    if range >= max(0.0312, hi * 0.125) {
        let nw = lum(uv + vec2<f32>(-t.x, -t.y));
        let ne = lum(uv + vec2<f32>(t.x, -t.y));
        let sw = lum(uv + vec2<f32>(-t.x, t.y));
        let se = lum(uv + vec2<f32>(t.x, t.y));
        // Sub-pixel aliasing amount.
        let avg = (2.0 * (n + s + e + w) + nw + ne + sw + se) / 12.0;
        let sub = clamp(abs(avg - m) / range, 0.0, 1.0);
        let sub_smooth = smoothstep(0.0, 1.0, sub);
        let sub_blend = sub_smooth * sub_smooth * 0.75;
        // Edge orientation.
        let horiz = abs(nw + ne - 2.0 * n) + 2.0 * abs(w + e - 2.0 * m) + abs(sw + se - 2.0 * s);
        let vert = abs(nw + sw - 2.0 * w) + 2.0 * abs(n + s - 2.0 * m) + abs(ne + se - 2.0 * e);
        let horizontal = horiz >= vert;
        var step = select(t.x, t.y, horizontal);
        let p_lum = select(e, s, horizontal);
        let n_lum = select(w, n, horizontal);
        let p_grad = abs(p_lum - m);
        let n_grad = abs(n_lum - m);
        var edge_lum = 0.5 * (m + p_lum);
        var grad = p_grad;
        if n_grad > p_grad {
            step = -step;
            edge_lum = 0.5 * (m + n_lum);
            grad = n_grad;
        }
        var edge_uv = uv;
        let along = select(vec2<f32>(0.0, t.y), vec2<f32>(t.x, 0.0), horizontal);
        if horizontal {
            edge_uv.y = edge_uv.y + step * 0.5;
        } else {
            edge_uv.x = edge_uv.x + step * 0.5;
        }
        // Walk both ways along the edge until it ends.
        let threshold = grad * 0.25;
        var pos_uv = edge_uv + along;
        var neg_uv = edge_uv - along;
        var pos_end = lum(pos_uv) - edge_lum;
        var neg_end = lum(neg_uv) - edge_lum;
        var pos_done = abs(pos_end) >= threshold;
        var neg_done = abs(neg_end) >= threshold;
        // A short search with growing strides (FXAA's low-preset schedule):
        // long edges are rare, and a step's miss only softens the blend.
        let steps = array<f32, 5>(1.5, 2.0, 4.0, 8.0, 12.0);
        for (var k = 0; k < 5; k = k + 1) {
            if pos_done && neg_done {
                break;
            }
            if !pos_done {
                pos_uv = pos_uv + along * steps[k];
                pos_end = lum(pos_uv) - edge_lum;
                pos_done = abs(pos_end) >= threshold;
            }
            if !neg_done {
                neg_uv = neg_uv - along * steps[k];
                neg_end = lum(neg_uv) - edge_lum;
                neg_done = abs(neg_end) >= threshold;
            }
        }
        let pos_dist = select(pos_uv.y - uv.y, pos_uv.x - uv.x, horizontal);
        let neg_dist = select(uv.y - neg_uv.y, uv.x - neg_uv.x, horizontal);
        let nearest = min(pos_dist, neg_dist);
        let span = pos_dist + neg_dist;
        let end_lum = select(neg_end, pos_end, pos_dist < neg_dist);
        // Only blend on the side where the edge's luma crosses ours.
        var edge_blend = 0.0;
        if (m - edge_lum < 0.0) != (end_lum < 0.0) {
            edge_blend = 0.5 - nearest / max(span, 1e-6);
        }
        let blend = max(edge_blend, sub_blend);
        var final_uv = uv;
        if horizontal {
            final_uv.y = final_uv.y + blend * step;
        } else {
            final_uv.x = final_uv.x + blend * step;
        }
        rgb = textureSampleLevel(src, src_sampler, final_uv, 0.0).rgb;
    }
    rgb = rgb * rgb; // gamma 2 -> linear
    if post.output.x > 0.5 {
        rgb = to_srgb(rgb); // a target that doesn't encode sRGB itself
    }
    return vec4<f32>(rgb, 1.0);
}
