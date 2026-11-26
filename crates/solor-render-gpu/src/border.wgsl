// Horizontal inset of an elliptical corner at distance dy from the box edge.
fn inset(rx: f32, ry: f32, dy: f32) -> f32 {
    if dy >= ry || ry <= 0. { return 0.; }
    let k = 1. - dy / ry;
    return rx * (1. - sqrt(max(0., 1. - k*k)));
}

fn edge_color(paint: vec4f, coverage: f32, gradient: bool, local: vec2f, size: vec2f) -> vec4f {
    if !gradient { return paint * coverage; }
    if coverage <= 0. { return vec4f(0.); }
    let brush = solor_stops[u32(paint.x)];
    let color = gradient_color(local, size, brush.color, brush.segment, paint.y != 0.);
    return vec4f(color.rgb, 1.) * (color.a * coverage);
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
    let cover = border_coverage(pixel, size, w, r, inner, rx, ry, u32(q.ramp.z));
    let index = u32(q.ramp.x);
    let tr = solor_stops[index];
    let bl = solor_stops[index + 1u];
    let mask = u32(q.ramp.y);
    let local = (v.pos.xy - q.rect.xy * scale) / scale;
    let c = edge_color(tr.color,   cover.x, (mask & 1u) != 0u, local, q.rect.zw)
          + edge_color(tr.segment, cover.y, (mask & 2u) != 0u, local, q.rect.zw)
          + edge_color(bl.color,   cover.z, (mask & 4u) != 0u, local, q.rect.zw)
          + edge_color(bl.segment, cover.w, (mask & 8u) != 0u, local, q.rect.zw);
    if c.a <= 0. { discard; }
    return vec4f(c.rgb / c.a, min(c.a, 1.) * solor_clip_coverage(v.pos.xy, v.clip));
}
