// Pixel area belonging to each edge, in top, right, bottom, left order.
fn edge_areas(pixel: vec2f, size: vec2f, widths: vec4f, box: vec4f, rx: vec4f, ry: vec4f) -> vec4f {
    var areas = vec4f(0.);
    if any(box.xy >= box.zw) { return areas; }
    for (var i = 0u; i < 4u; i++) {
        let start = pixel.y + f32(i) * 0.25;
        let lo = max(start, box.y);
        let hi = min(start + 0.25, box.w);
        if lo >= hi { continue; }
        let y = (lo + hi) * 0.5;
        let t = y - box.y;
        let b = box.w - y;
        let left = max(pixel.x, box.x + max(inset(rx.x, ry.x, t), inset(rx.w, ry.w, b)));
        let right = min(pixel.x + 1., box.z - max(inset(rx.y, ry.y, t), inset(rx.z, ry.z, b)));
        if left >= right { continue; }
        let upper = select(1e30, y / widths.x, widths.x > 0.);
        let lower = select(1e30, (size.y - y) / widths.z, widths.z > 0.);
        let depth = min(upper, lower);
        let split = size.x * widths.w / max(widths.w + widths.y, 1e-30);
        let a = clamp(min(widths.w * depth, split), left, right);
        let z = clamp(max(size.x - widths.y * depth, split), left, right);
        let middle = z - a;
        areas += (hi - lo) * vec4f(select(0., middle, upper <= lower), right - z,
                                  select(middle, 0., upper <= lower), a - left);
    }
    return areas;
}

fn border_coverage(pixel: vec2f, size: vec2f, w: vec4f, r: vec4f, inner: vec4f, rx: vec4f, ry: vec4f, index: u32) -> vec4f {
    return max(vec4f(0.), edge_areas(pixel, size, w, vec4f(0., 0., size), r, r)
                         - edge_areas(pixel, size, w, inner, rx, ry));
}
