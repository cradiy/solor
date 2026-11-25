fn paint(local: vec2f, size: vec2f, q: Quad) -> vec4f {
    let coverage = textureLoad(bitmap, vec2i(floor(q.ramp.xy + local)), 0).r;
    var ink = q.params;
    if q.ramp.z != 0. {
        let paint_box = stops[u32(q.ramp.z)];
        let bounds = paint_box.color;
        let point = (q.rect.xy + local) / q.ramp.w - bounds.xy;
        ink = foreground(point, bounds.zw, q, paint_box.segment);
    }
    return vec4f(ink.rgb, clamp(ink.a, 0., 1.) * coverage);
}
