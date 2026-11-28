// A custom post effect. The engine calls `post` for every pixel of the
// finished frame (after tone mapping and grading, before the HUD) and
// shows what it returns.
//
// in.uv          0..1, top-left origin    in.pixel   the same in pixels
// in.color       the image here (rgb)     in.resolution  image size (px)
// in.time        seconds
// in.p0, in.p1   the Post Shader's params (8 numbers)
//
// Helpers: image(uv) and image_pixel(pixel) read the image anywhere;
// noise3(p), hash1(n).
// Save to see it change; mistakes show in the Console.

fn post(in: PostInput) -> vec3<f32> {
    // Scanlines as strong as param 0, and a vignette as strong as param 1.
    let line = 1.0 - in.p0.x * 0.5 * (1.0 - cos(in.pixel.y * 3.14159));
    let d = distance(in.uv, vec2<f32>(0.5));
    let vignette = 1.0 - in.p0.y * smoothstep(0.3, 0.8, d);
    return in.color * line * vignette;
}
