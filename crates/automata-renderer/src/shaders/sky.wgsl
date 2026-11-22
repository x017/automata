// The sky dome: a fullscreen triangle drawn after opaque geometry, behind
// it (depth test only where nothing was drawn). Prepended at build time:
// tonemap.wgsl and common.wgsl, whose `sky_dome` gives the gradient and the
// sun's halo; this adds the sun disk and drifting clouds.

struct SkyOut {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    // On the far plane, so only uncovered pixels pass the depth test.
    out.position = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    let h = light.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(h.xyz / h.w - camera.view_pos.xyz);
}

fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(127.1, 311.7));
    return fract((q.x + q.y) * (q.x * q.y + 19.19) * 43.758);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Four octaves, each rotated so the grid never lines up.
fn fbm(p0: vec2<f32>) -> f32 {
    let rot = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    var p = p0;
    var sum = 0.0;
    var amp = 0.5;
    for (var k = 0; k < 4; k = k + 1) {
        sum = sum + amp * value_noise(p);
        p = rot * p * 2.03 + vec2<f32>(17.0, 3.0);
        amp = amp * 0.5;
    }
    return sum;
}

// Cloud cover along `d` (above the horizon): coverage and the light
// through it, from a noise layer projected onto a plane overhead.
fn clouds(d: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    let cover = light.sky_sun.z;
    if cover <= 0.001 || d.y <= 0.0 {
        return base;
    }
    let time = light.fog_params.y;
    let uv = d.xz / (d.y + 0.1) * 1.8 + vec2<f32>(time * 0.01, time * 0.003);
    // Big shapes from a coarse layer, detail eroding their edges.
    let n = fbm(uv * 0.5) * 0.65 + fbm(uv * 2.3 + vec2<f32>(5.2, 1.3)) * 0.35;
    let threshold = 1.0 - cover;
    let density = smoothstep(threshold * 0.7 + 0.08, threshold * 0.7 + 0.26, n);
    if density <= 0.0 {
        return base;
    }
    let l = sun_dir();
    let mu = dot(d, l);
    // Thick parts darken underneath; edges toward the sun light up.
    let tint = sun_tint();
    let ambient = mix(light.background.rgb, light.sky_zenith.rgb, 0.3) * 0.9 + vec3<f32>(0.05);
    let lit = tint * (0.75 + 0.25 * l.y) + ambient * 0.35;
    let shade = mix(lit, ambient * 0.8, smoothstep(0.4, 1.0, density) * 0.6);
    let silver = tint * lobe(mu, 12.0) * (1.0 - density) * 1.5;
    // Clouds thin out toward the horizon, where they'd alias into stripes.
    let fade = smoothstep(0.0, 0.12, d.y);
    return mix(base, shade + silver, density * fade * 0.9);
}

fn sky_color(ndc: vec2<f32>) -> vec3<f32> {
    let d = view_ray(ndc);
    var c = sky_dome(d);
    let l = sun_dir();
    let mu = dot(d, l);
    // The disk: a hot core with darker limbs and an anti-aliased edge,
    // bright enough in HDR for bloom to catch.
    let edge = light.sky_sun.x;
    let width = max((1.0 - edge) * 0.25, 2e-6);
    let disk = smoothstep(edge - width, edge + width, mu);
    if disk > 0.0 && d.y > -0.02 {
        let r = clamp((1.0 - mu) / max(1.0 - edge, 1e-6), 0.0, 1.0);
        let limb = 1.0 - 0.45 * r * r;
        c = c + sun_tint() * disk * limb * 40.0 * smoothstep(-0.02, 0.01, d.y);
    }
    return clouds(d, c);
}

// The background at pixel `frag`: the sky, or from under the water, the
// sky dimmed by the water up to the surface (looking up) or just the
// water's colour (looking level or down, where nothing ends the view).
fn background(ndc: vec2<f32>, frag: vec2<f32>) -> vec3<f32> {
    var c = light.background.rgb; // the sky pass runs without a dome only for water
    if sky_on() {
        c = sky_color(ndc);
    }
    if eye_under_water(frag) {
        let d = view_ray(ndc);
        let depth = max(light.water_eye.x - camera.view_pos.y, 0.0);
        let up = smoothstep(0.02, 0.2, d.y);
        return mix(water_fog_color(), water_fog(c, depth / max(d.y, 0.02)), up);
    }
    return c;
}

// The forward path writes display colors (tone mapped here).
@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4<f32>(finish_forward(background(in.ndc, in.position.xy)), 1.0);
}

// Beautiful writes HDR; alpha 1 so the post chain tone maps and blooms it.
@fragment
fn fs_sky_hdr(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4<f32>(background(in.ndc, in.position.xy), 1.0);
}
