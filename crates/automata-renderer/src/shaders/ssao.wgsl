// Screen-space ambient occlusion and contact shadows (the Beautiful preset).
//
// A depth prepass of the opaque meshes feeds one half-resolution pass that
// works out, per pixel: (r) how open the surface is to the sky (ambient
// occlusion from nearby geometry), (g) whether a short ray toward the sun
// hits something the shadow map is too coarse to see (contact shadows),
// and (b) its view depth. Two depth-aware blurs smooth r and g, and the
// mesh shader upsamples them by that depth (`screen_occlusion`).

struct SsaoUniform {
    view_proj: mat4x4<f32>,
    camera: vec4<f32>,  // xyz: camera position, w: near plane
    forward: vec4<f32>, // xyz: camera forward, w: far plane
    right: vec4<f32>,   // xyz: right * tan(half fov) * aspect, w: AO radius (m)
    up: vec4<f32>,      // xyz: up * tan(half fov), w: contact ray length (m)
    sun: vec4<f32>,     // xyz: toward the sun, w: contact shadows on
    size: vec4<f32>,    // xy: full-resolution size (px), zw: blur step (half-res px)
    params: vec4<f32>,  // x: ambient occlusion on
};
@group(0) @binding(0) var<uniform> u: SsaoUniform;
@group(0) @binding(1) var depth_map: texture_depth_2d;
@group(0) @binding(2) var src: texture_2d<f32>;

const AO_SAMPLES: i32 = 12;
const CONTACT_STEPS: i32 = 12;
// View depth written for the background (the sky): far beyond anything.
const SKY_DEPTH: f32 = 60000.0;

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Distance along the camera's forward axis for a depth-buffer value
// (perspective, depth 0 at the near plane and 1 at the far plane).
fn linear_depth(d: f32) -> f32 {
    let n = u.camera.w;
    let f = u.forward.w;
    return n * f / max(f - d * (f - n), 1e-6);
}

fn full_size() -> vec2<i32> {
    return vec2<i32>(u.size.xy);
}

// View depth at a full-resolution pixel (clamped to the screen).
fn depth_at(px: vec2<i32>) -> f32 {
    let d = textureLoad(depth_map, clamp(px, vec2<i32>(0), full_size() - 1), 0);
    if d >= 1.0 {
        return SKY_DEPTH;
    }
    return linear_depth(d);
}

// The world position seen at uv (0..1, y down) at view depth `vz`.
fn world_at(uv: vec2<f32>, vz: f32) -> vec3<f32> {
    let ray = u.forward.xyz + u.right.xyz * (uv.x * 2.0 - 1.0) + u.up.xyz * (1.0 - uv.y * 2.0);
    return u.camera.xyz + ray * vz;
}

fn pixel_uv(px: vec2<i32>) -> vec2<f32> {
    return (vec2<f32>(px) + 0.5) / u.size.xy;
}

fn view_depth_of(p: vec3<f32>) -> f32 {
    return dot(p - u.camera.xyz, u.forward.xyz);
}

// Where world point `p` lands: (uv, view depth); z <= 0 behind the camera
// or off screen.
fn project(p: vec3<f32>) -> vec3<f32> {
    let clip = u.view_proj * vec4<f32>(p, 1.0);
    if clip.w <= 1e-4 {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    let uv = clip.xy / clip.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    return vec3<f32>(uv, clip.w);
}

fn scene_depth_at_uv(uv: vec2<f32>) -> f32 {
    return depth_at(vec2<i32>(uv * u.size.xy));
}

@fragment
fn fs_ao(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let half_px = vec2<i32>(frag.xy);
    let px = half_px * 2;
    let vz = depth_at(px);
    if vz >= SKY_DEPTH {
        return vec4<f32>(1.0, 1.0, SKY_DEPTH, 1.0);
    }
    let p = world_at(pixel_uv(px), vz);

    // The normal from neighboring depths, taking the nearer neighbor on
    // each axis so silhouettes don't bend it.
    let l = depth_at(px - vec2<i32>(1, 0));
    let r = depth_at(px + vec2<i32>(1, 0));
    let t = depth_at(px - vec2<i32>(0, 1));
    let b = depth_at(px + vec2<i32>(0, 1));
    var dx: vec3<f32>;
    if abs(r - vz) < abs(l - vz) {
        dx = world_at(pixel_uv(px + vec2<i32>(1, 0)), r) - p;
    } else {
        dx = p - world_at(pixel_uv(px - vec2<i32>(1, 0)), l);
    }
    var dy: vec3<f32>;
    if abs(b - vz) < abs(t - vz) {
        dy = world_at(pixel_uv(px + vec2<i32>(0, 1)), b) - p;
    } else {
        dy = p - world_at(pixel_uv(px - vec2<i32>(0, 1)), t);
    }
    var n = normalize(cross(dy, dx));
    if dot(n, u.camera.xyz - p) < 0.0 {
        n = -n;
    }

    let noise = ign(frag.xy);
    // A small step off the surface keeps it from occluding itself.
    let origin = p + n * (0.01 + 0.002 * vz);

    // Ambient occlusion: points in the hemisphere above the surface that
    // are behind the depth buffer are blocked, weighted down when the
    // blocker is much farther away than the sampling radius.
    var ao = 1.0;
    if u.params.x > 0.0 {
        var helper = vec3<f32>(0.0, 1.0, 0.0);
        if abs(n.y) > 0.9 {
            helper = vec3<f32>(1.0, 0.0, 0.0);
        }
        let tangent = normalize(cross(helper, n));
        let bitangent = cross(n, tangent);
        let radius = u.right.w;
        var occluded = 0.0;
        for (var k = 0; k < AO_SAMPLES; k = k + 1) {
            let f = (f32(k) + 0.5) / f32(AO_SAMPLES);
            // A golden-angle spiral over the hemisphere (uniform in solid
            // angle, so low directions, which find walls and the ground
            // next to objects, count fully), at reaches spread evenly
            // out to the radius.
            let angle = (noise + f32(k) * 0.381966) * 6.2831853;
            let z = 1.0 - f;
            let ring = sqrt(max(1.0 - z * z, 0.0));
            let dir = (tangent * cos(angle) + bitangent * sin(angle)) * ring + n * z;
            let reach = radius * mix(0.1, 1.0, fract(f + noise * 0.618));
            let s = origin + dir * reach;
            let proj = project(s);
            if proj.z <= 0.0 {
                continue;
            }
            let scene = scene_depth_at_uv(proj.xy);
            let behind = proj.z - scene;
            if behind > 0.02 {
                occluded += clamp(radius / max(abs(vz - scene), 1e-3) - 0.5, 0.0, 1.0);
            }
        }
        // Squared: open ground stays open while creases go properly dark.
        ao = 1.0 - occluded / f32(AO_SAMPLES);
        ao = ao * ao;
    }

    // Contact shadow: march a short ray toward the sun; anything in front
    // of it (but not too thick, so it isn't just a distant wall) blocks.
    var lit = 1.0;
    let l_dir = u.sun.xyz;
    if u.sun.w > 0.0 && dot(n, l_dir) > 0.0 {
        let step = u.up.w / f32(CONTACT_STEPS);
        let thickness = max(0.15, u.up.w * 0.5);
        for (var k = 0; k < CONTACT_STEPS; k = k + 1) {
            let s = origin + l_dir * step * (f32(k) + noise);
            let proj = project(s);
            if proj.z <= 0.0 {
                break;
            }
            let delta = proj.z - scene_depth_at_uv(proj.xy);
            if delta > 0.01 + 0.002 * proj.z && delta < thickness {
                // Softer toward the ray's end.
                lit = (f32(k) + noise) / f32(CONTACT_STEPS);
                break;
            }
        }
    }
    return vec4<f32>(ao, lit, vz, 1.0);
}

// A 7-tap depth-aware blur along `u.size.zw` (half-res pixels); taps on
// a different surface (by view depth) are left out.
@fragment
fn fs_blur(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(frag.xy);
    let dims = vec2<i32>(textureDimensions(src));
    let center = textureLoad(src, p, 0);
    if center.b >= SKY_DEPTH {
        return center;
    }
    let step = vec2<i32>(u.size.zw);
    let tolerance = 0.03 * center.b + 0.05;
    var sum = center.rg;
    var total = 1.0;
    for (var k = 1; k <= 3; k = k + 1) {
        let w = exp(-f32(k * k) / 8.0);
        let a = textureLoad(src, clamp(p + step * k, vec2<i32>(0), dims - 1), 0);
        let b = textureLoad(src, clamp(p - step * k, vec2<i32>(0), dims - 1), 0);
        let wa = w * f32(abs(a.b - center.b) < tolerance);
        let wb = w * f32(abs(b.b - center.b) < tolerance);
        sum += a.rg * wa + b.rg * wb;
        total += wa + wb;
    }
    return vec4<f32>(sum / total, center.b, 1.0);
}
