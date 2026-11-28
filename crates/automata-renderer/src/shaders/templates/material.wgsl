// A custom material shader. The engine calls `material` for every pixel of
// every mesh wearing the material, with the standard material already read
// (colour, maps, tiling) in `in`; the lights then shade what it returns.
//
// in.position  world position      in.normal   surface normal
// in.view      toward the camera   in.uv       texture coordinates
// in.pixel     window pixel        in.time     seconds
// in.color     colour (rgba)       in.roughness, in.metallic, in.emissive
// in.p0, in.p1 the material's shader_params (8 numbers)
//
// Helpers: material_defaults(in), base_map(uv), noise3(p), hash1(n).
// Save to see it change; mistakes show in the Console.

fn material(in: MaterialInput) -> MaterialOutput {
    var out = material_defaults(in);
    // A rim of light around the edges, coloured by params 0-2 and as
    // bright as param 3, pulsing over time.
    let rim = pow(1.0 - max(dot(in.normal, in.view), 0.0), 3.0);
    let pulse = 0.75 + 0.25 * sin(in.time * 3.0);
    out.emissive = out.emissive + in.p0.rgb * in.p0.a * rim * pulse;
    return out;
}
