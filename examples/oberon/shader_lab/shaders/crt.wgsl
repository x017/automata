// CRT: an old curved monitor, with scanlines and colour fringes.
//
// params:
//   p0.x    curvature (0 = flat)
//   p0.y    scanline strength, 0..1
//   p0.z    colour fringe (pixels)
//   p0.w    vignette, 0..1

fn post(in: PostInput) -> vec3<f32> {
    let c = in.uv * 2.0 - 1.0;
    let uv = c * (1.0 + in.p0.x * dot(c, c)) * 0.5 + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec3<f32>(0.0);
    }
    let shift = vec2<f32>(in.p0.z / in.resolution.x, 0.0);
    let color = vec3<f32>(image(uv + shift).r, image(uv).g, image(uv - shift).b);
    let line = 1.0 - in.p0.y * (0.5 + 0.5 * sin(uv.y * in.resolution.y * 3.14159));
    let vignette = 1.0 - in.p0.w * 0.5 * dot(c, c);
    return color * line * vignette;
}
