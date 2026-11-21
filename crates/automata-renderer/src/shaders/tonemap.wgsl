// Tone mapping curves, shared by the lit shaders (via common.wgsl) and the
// post chain. No bindings: safe to prepend to any module.

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

// Khronos PBR Neutral: colors pass through unchanged up to ~0.76, then
// highlights roll off smoothly to white. Beautiful's curve for scenes that
// ask for no tone mapping, so authored colors keep their look in HDR.
fn tonemap_neutral(c: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desaturation = 0.15;
    let x = min(c.r, min(c.g, c.b));
    var offset = 0.04;
    if x < 0.08 {
        offset = x - 6.25 * x * x;
    }
    var color = c - vec3<f32>(offset);
    let peak = max(color.r, max(color.g, color.b));
    if peak < start {
        return color;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    color = color * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3<f32>(new_peak), g);
}

// Exposure and the environment's tone mapping curve (linear in and out).
fn tonemap_with(c: vec3<f32>, exposure: f32, mode: f32) -> vec3<f32> {
    let x = c * exposure;
    if mode > 3.5 {
        return tonemap_neutral(x);
    }
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
