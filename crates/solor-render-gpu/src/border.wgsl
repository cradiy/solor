// Horizontal inset of an elliptical corner at distance dy from the box edge.
fn inset(rx: f32, ry: f32, dy: f32) -> f32 {
    if dy >= ry || ry <= 0. { return 0.; }
    let k = 1. - dy / ry;
    return rx * (1. - sqrt(max(0., 1. - k*k)));
}

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

@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    if solor_outside_clip(v.pos.xy, q) { discard; }
    let scale = q.viewport.z;
    let size = q.rect.zw * scale;
    let pixel = v.pos.xy - q.rect.xy * scale - 0.5;
    let w = q.params * scale;
    let r = q.radii * scale;
    let inner = vec4f(w.w, w.x, size.x - w.y, size.y - w.z);
    let rx = max(r - w.wyyw, vec4f(0.));
    let ry = max(r - w.xxzz, vec4f(0.));
    if all(pixel >= inner.xy + vec2f(max(rx.x, rx.w), max(ry.x, ry.y))) &&
       all(pixel + 1. <= inner.zw - vec2f(max(rx.y, rx.z), max(ry.z, ry.w))) { discard; }
    let cover = max(vec4f(0.), edge_areas(pixel, size, w, vec4f(0., 0., size), r, r)
                              - edge_areas(pixel, size, w, inner, rx, ry));
    let index = u32(q.ramp.x);
    let tr = solor_stops[index];
    let bl = solor_stops[index + 1u];
    let c = tr.color * cover.x + tr.segment * cover.y + bl.color * cover.z + bl.segment * cover.w;
    if c.a <= 0. { discard; }
    return vec4f(c.rgb / c.a, min(c.a, 1.));
}
