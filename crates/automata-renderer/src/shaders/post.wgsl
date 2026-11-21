// Post chain for the Beautiful preset. Prepended at build time: tonemap.wgsl.
//
//   1. bloom: a prefilter (Karis average against fireflies and a soft-knee
//      threshold, so only highlights glow), then a mip chain of dual-filter
//      downsamples and upsamples added back up (Bjorge, "Bandwidth-Efficient
//      Rendering", SIGGRAPH 2015): 4-5 fetches per pixel per pass
//   2. resolve: HDR + bloom, exposure, tone mapping -> display-encoded LDR
//      with luma in alpha. The HDR image's alpha is surface coverage: the
//      background (alpha 0) skips exposure and the curve, exactly like the
//      forward path, so the clear color shows as authored.
//   3. fxaa: edge anti-aliasing on the LDR image, written to the target
//
// Every pass draws one fullscreen triangle with the same bind group shape.

struct PostUniform {
    texel: vec4<f32>,  // xy: 1 / source size
    params: vec4<f32>, // x: unused, y: exposure, z: tone mapping, w: bloom
    output: vec4<f32>, // x: encode sRGB in the shader (the target doesn't)
    sun: vec4<f32>,    // xy: sun position (uv), z: lens flare strength, w: disk radius (uv)
    sun_color: vec4<f32>, // rgb: sunlight, a: aspect ratio
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> post: PostUniform;
@group(0) @binding(3) var extra: texture_2d<f32>; // resolve: the bloom
@group(0) @binding(4) var extra2: texture_2d<f32>; // resolve: sun visibility (1x1)

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

@fragment
fn fs_resolve(in: FullscreenOut) -> @location(0) vec4<f32> {
    let hdr = textureSampleLevel(src, src_sampler, in.uv, 0.0);
    let bloom = textureSampleLevel(extra, src_sampler, in.uv, 0.0).rgb;
    let color = hdr.rgb + bloom * post.params.w + lens_flare(in.uv);
    let lit = tonemap_with(color, post.params.y, post.params.z);
    let mapped = clamp(mix(color, lit, hdr.a), vec3<f32>(0.0), vec3<f32>(1.0));
    // Stored with gamma 2 (sqrt / square: an exact, cheap inverse pair that
    // spends 8 bits perceptually like sRGB). FXAA reads luma from alpha.
    let encoded = sqrt(mapped);
    return vec4<f32>(encoded, luma(encoded));
}

// FXAA (after Lottes' FXAA 3.11, "quality" variant with a short search).
fn lum(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(src, src_sampler, uv, 0.0).a;
}

@fragment
fn fs_fxaa(in: FullscreenOut) -> @location(0) vec4<f32> {
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
