
// ---------------------------------------------------------------------------
// Helpers custom shaders (materials and post effects) can call.

// Smooth value noise in 0..1 (one octave).
fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let n = i.x + i.y * 57.0 + i.z * 113.0;
    let a = mix(mix(hash1(n), hash1(n + 1.0), u.x), mix(hash1(n + 57.0), hash1(n + 58.0), u.x), u.y);
    let b = mix(
        mix(hash1(n + 113.0), hash1(n + 114.0), u.x),
        mix(hash1(n + 170.0), hash1(n + 171.0), u.x),
        u.y
    );
    return mix(a, b, u.z);
}

// A pseudo-random number in 0..1 from any number.
fn hash1(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453);
}
