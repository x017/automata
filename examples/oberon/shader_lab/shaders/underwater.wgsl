// Underwater: the view wavers and turns blue-green.
//
// params:
//   p0.x    wave size (pixels)
//   p0.y    wave speed
//   p1.rgb  tint
//   p1.a    tint amount, 0..1

fn post(in: PostInput) -> vec3<f32> {
    let t = in.time * in.p0.y;
    let wave = vec2<f32>(sin(in.uv.y * 18.0 + t), cos(in.uv.x * 14.0 + t * 1.3));
    let color = image(in.uv + wave * in.p0.x / in.resolution);
    let tinted = mix(color, color * in.p1.rgb, in.p1.a);
    // Darker toward the bottom, like deeper water.
    return tinted * (0.8 + 0.2 * (1.0 - in.uv.y));
}
