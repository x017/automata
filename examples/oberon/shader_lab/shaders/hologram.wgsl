// Hologram: see-through light with scanlines rolling up and a bright rim.
//
// shader_params:
//   p0.rgb  colour
//   p0.a    scanlines per metre
//   p1.x    flicker (0 = steady, 1 = drops out now and then)

fn material(in: MaterialInput) -> MaterialOutput {
    var out = material_defaults(in);
    let rim = pow(1.0 - abs(dot(in.normal, in.view)), 2.0);
    let lines = 0.5 + 0.5 * sin((in.position.y - in.time * 0.6) * in.p0.a * 6.2832);
    let flicker = 1.0 - in.p1.x * step(0.93, hash1(floor(in.time * 24.0)));
    out.color = vec4<f32>(in.p0.rgb * 0.1, 0.35 + 0.5 * rim);
    out.emissive = in.p0.rgb * (0.25 + 2.0 * rim) * (0.55 + 0.45 * lines) * flicker;
    out.roughness = 0.2;
    out.metallic = 0.0;
    return out;
}
