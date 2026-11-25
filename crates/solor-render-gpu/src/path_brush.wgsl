fn solor_path_color(point: vec2f, q: solor_Quad) -> vec4f {
    if q.ramp.w == 0. { return q.params; }
    let brush = solor_stops[u32(q.ramp.w)];
    let device = point - q.radii.xy;
    return brush_color(device / brush.segment.x, q.radii.zw, device - 0.5, q.params, brush.color, brush.segment.y);
}
