// Lava: dark crust drifting over glowing molten rock.
//
// shader_params:
//   p0.x    flow speed
//   p0.y    pattern size (blobs across the texture)
//   p0.z    glow brightness

fn material(in: MaterialInput) -> MaterialOutput {
    var out = material_defaults(in);
    let flow = vec3<f32>(in.time * in.p0.x * 0.1, 0.0, in.time * in.p0.x * 0.05);
    let p = vec3<f32>(in.uv * in.p0.y, 0.0);
    let n = noise3(p + flow) * 0.6 + noise3(p * 2.3 - flow) * 0.3 + noise3(p * 5.1) * 0.1;
    let heat = smoothstep(0.45, 0.72, n);
    let crust = vec3<f32>(0.07, 0.05, 0.04);
    out.color = vec4<f32>(mix(crust, vec3<f32>(0.9, 0.3, 0.05), heat), 1.0);
    out.emissive = vec3<f32>(1.0, 0.35, 0.05) * heat * heat * in.p0.z;
    out.roughness = mix(0.95, 0.5, heat);
    out.metallic = 0.0;
    return out;
}
