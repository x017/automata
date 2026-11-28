
// A custom material: the user's `material` function shapes the surface.
fn apply_material(in: VertexOutput, uv: vec2<f32>, s: Surface) -> Surface {
    material_lod = material_mip(uv);
    return material_surface(s, material(material_input(in, uv, s)));
}
