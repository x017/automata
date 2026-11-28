// Ink: comic-book outlines and flat colour bands.
//
// params:
//   p0.x    line strength, 0..1
//   p0.y    how sharp a change in brightness draws a line (0.05..0.5)
//   p0.z    colour bands per channel (0 = smooth colour)

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn post(in: PostInput) -> vec3<f32> {
    // Sobel: how fast the brightness changes around this pixel.
    var gx = 0.0;
    var gy = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let l = luma(image_pixel(in.pixel + vec2<f32>(f32(x), f32(y))));
            let wx = f32(x) * select(1.0, 2.0, y == 0);
            let wy = f32(y) * select(1.0, 2.0, x == 0);
            gx += l * wx;
            gy += l * wy;
        }
    }
    let edge = sqrt(gx * gx + gy * gy);
    var color = in.color;
    if in.p0.z > 0.0 {
        color = floor(color * in.p0.z + 0.5) / in.p0.z;
    }
    let ink = smoothstep(in.p0.y, in.p0.y * 2.0, edge) * in.p0.x;
    return mix(color, vec3<f32>(0.04, 0.03, 0.05), ink);
}
