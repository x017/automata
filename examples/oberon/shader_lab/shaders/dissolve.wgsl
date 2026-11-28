// Dissolve: the surface burns away through noise, with a glowing edge.
//
// shader_params:
//   p0.x    how much is gone, 0..1 (used when p0.y is 0)
//   p0.y    > 0: burn away and back on its own, this fast
//   p0.z    noise size (patches per metre)
//   p1.rgb  edge colour
//   p1.a    edge brightness

fn material(in: MaterialInput) -> MaterialOutput {
    var out = material_defaults(in);
    var amount = in.p0.x;
    if in.p0.y > 0.0 {
        amount = 0.5 + 0.5 * sin(in.time * in.p0.y);
    }
    let p = in.position * in.p0.z;
    let n = noise3(p) * 0.7 + noise3(p * 2.7) * 0.3;
    // Below 0: burnt away. Cut-outs still cast the whole mesh's shadow.
    let edge = n - amount * 1.05;
    if edge < 0.0 {
        discard;
    }
    let glow = 1.0 - smoothstep(0.0, 0.06, edge);
    out.emissive = out.emissive + in.p1.rgb * in.p1.a * glow;
    out.color = vec4<f32>(out.color.rgb * (1.0 - glow), out.color.a);
    return out;
}
