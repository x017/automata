// Particles: camera-facing quads, one instance each, drawn after the
// opaque scene and water. Prepended at build time: tonemap.wgsl and
// common.wgsl (camera in group 0, light in group 3).
//
// Each instance is a quad of `size` metres turned to face the camera (and
// spun by `rotation`), or, with a `streak`, stretched along it (sparks).
// Soft particles fade where they meet the scene behind them, read from the
// depth buffer, instead of cutting a hard line through it.

struct ParticleUniform {
    right_soft: vec4<f32>, // xyz: camera right, w: soft fade distance (m; 0 = hard)
    up_lit: vec4<f32>,     // xyz: camera up, w: 1 = lit by the sun and lights
};
@group(1) @binding(0) var<uniform> particles: ParticleUniform;
@group(1) @binding(1) var sprite: texture_2d<f32>;
@group(1) @binding(2) var sprite_sampler: sampler;
@group(1) @binding(3) var scene_depth: texture_2d<f32>; // depth, as unfilterable float (WebGL2)

struct Instance {
    @location(0) position_size: vec4<f32>,   // xyz: centre, w: size (m)
    @location(1) color: vec4<f32>,           // linear RGBA
    @location(2) uv: vec4<f32>,              // sprite rectangle: u0, v0, u1, v1
    @location(3) streak_rotation: vec4<f32>, // xyz: streak (m), w: rotation (rad)
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) light_in: vec3<f32>,
    // The sunlight it gets when nothing shades it, and its centre (where
    // the fragment stage looks the shade up).
    @location(4) sun_in: vec3<f32>,
    @location(5) @interpolate(flat) center: vec3<f32>,
};

// The light a particle at `p` gets: sky and ground ambient and the point
// lights. Lighting per particle rather than per pixel: a puff has no
// surface to shade.
fn particle_light(p: vec3<f32>) -> vec3<f32> {
    var lit = hemi(vec3<f32>(0.0, 1.0, 0.0)) * light.color_ambient.a;
    for (var i = 0u; i < point_count(); i = i + 1u) {
        lit = lit + point_light(i, p).radiance * 0.6;
    }
    return lit;
}

// The sun on a particle at `p`, wrapped round it (a puff lit from behind
// still glows a little); shade is looked up per pixel.
fn particle_sun(p: vec3<f32>) -> vec3<f32> {
    let to_cam = normalize(camera.view_pos.xyz - p);
    let facing = 0.5 + 0.5 * dot(to_cam, sun_dir());
    return light.color_ambient.rgb * mix(0.3, 1.0, facing);
}

// Whether the sun reaches `p` (1) or something shades it (0): one
// filtered compare in its shadow cascade, so motes glint in sunbeams and
// smoke darkens in shade.
fn particle_shade(p: vec3<f32>) -> f32 {
    if light.params.x < 0.5 {
        return 1.0;
    }
    let i = i32(cascade_pick(p).x);
    let c = cascade_coords(i, p, sun_dir(), 1.0);
    if c.x < 0.0 {
        return 1.0;
    }
    return textureSampleCompareLevel(shadow_map, shadow_sampler, c.xy, i, c.z);
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> VertexOutput {
    // Triangle strip: (0,0), (1,0), (0,1), (1,1).
    let t = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let corner = t * 2.0 - 1.0;
    let half = inst.position_size.w * 0.5;
    var right = particles.right_soft.xyz;
    var up = particles.up_lit.xyz;
    var center = inst.position_size.xyz;
    var extent = vec2<f32>(half, half);
    let streak = inst.streak_rotation.xyz;
    let length_ = length(streak);
    var streaked = false;
    if length_ > 1e-4 {
        let axis = streak / length_;
        let side = cross(axis, camera.view_pos.xyz - center);
        if dot(side, side) > 1e-8 {
            // Along the streak, trailing behind the particle.
            up = axis;
            right = normalize(side);
            center = center - streak * 0.5;
            extent = vec2<f32>(half, half + length_ * 0.5);
            streaked = true;
        }
    }
    if !streaked {
        let c = cos(inst.streak_rotation.w);
        let s = sin(inst.streak_rotation.w);
        let r = right * c + up * s;
        up = up * c - right * s;
        right = r;
    }
    let world = center + right * (corner.x * extent.x) + up * (corner.y * extent.y);
    var out: VertexOutput;
    out.clip = camera.view_proj * vec4<f32>(world, 1.0);
    // Texture v runs down: the quad's top is v0.
    out.uv = vec2<f32>(mix(inst.uv.x, inst.uv.z, t.x), mix(inst.uv.w, inst.uv.y, t.y));
    out.color = inst.color;
    out.world = world;
    out.light_in = vec3<f32>(1.0);
    out.sun_in = vec3<f32>(0.0);
    out.center = inst.position_size.xyz;
    if particles.up_lit.w > 0.5 {
        out.light_in = particle_light(inst.position_size.xyz);
        out.sun_in = particle_sun(inst.position_size.xyz);
    }
    return out;
}

// Depth buffer value -> view depth (m).
fn view_depth(d: f32) -> f32 {
    let near = light.screen.z;
    let far = light.screen.w;
    return near * far / (far - d * (far - near));
}

// How much fog covers `world` (0 none, 1 all).
fn fog_amount(world: vec3<f32>, frag: vec2<f32>) -> f32 {
    if eye_under_water(frag) {
        return 1.0 - water_transmission(water_path(world)).g;
    }
    return fog_cover(world);
}

// The particle's colour and coverage, before fog and tone mapping.
fn particle_color(in: VertexOutput) -> vec4<f32> {
    let tex = textureSample(sprite, sprite_sampler, in.uv);
    var c = in.color * tex;
    var light_in = in.light_in;
    if particles.up_lit.w > 0.5 {
        light_in = light_in + in.sun_in * particle_shade(in.center);
    }
    c = vec4<f32>(c.rgb * light_in, c.a);
    let mine = dot(in.world - camera.view_pos.xyz, light.camera_forward.xyz);
    // Fade out right in front of the lens instead of filling the screen.
    c.a = c.a * clamp((mine - 0.15) * 2.5, 0.0, 1.0);
    let soft = particles.right_soft.w;
    if soft > 0.0 {
        let depth = textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0).x;
        if depth < 1.0 {
            c.a = c.a * clamp((view_depth(depth) - mine) / soft, 0.0, 1.0);
        }
    }
    return c;
}

// Forward path (tone mapped here), alpha blended: smoke, dust.
@fragment
fn fs_ldr_alpha(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = particle_color(in);
    return vec4<f32>(finish_forward(apply_fog(c.rgb, in.world, in.clip.xy)), c.a);
}

// Forward path, added on top: fire, sparks, magic. Fog hides them rather
// than tinting them.
@fragment
fn fs_ldr_add(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = particle_color(in);
    return vec4<f32>(finish_forward(c.rgb) * c.a * (1.0 - fog_amount(in.world, in.clip.xy)), c.a);
}

// Beautiful (HDR, tone mapped after bloom), alpha blended.
@fragment
fn fs_hdr_alpha(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = particle_color(in);
    return vec4<f32>(apply_fog_scatter(c.rgb, in.world, in.clip.xy), c.a);
}

// Beautiful, added: bright particles bloom.
@fragment
fn fs_hdr_add(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = particle_color(in);
    return vec4<f32>(c.rgb * c.a * (1.0 - fog_amount(in.world, in.clip.xy)), min(c.a, 1.0));
}
