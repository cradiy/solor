fn paint(local: vec2f, size: vec2f, q: Quad) -> vec4f {
    if q.ramp.y == 0. { return q.params; }
    return gradient_color(local, size, q.params, q.ramp, q.viewport.w != 0.);
}
