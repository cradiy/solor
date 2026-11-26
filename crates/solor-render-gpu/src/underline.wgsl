@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    if solor_outside_clip(v.pos.xy, q) { discard; }
    let scale = q.viewport.z;
    let point = v.pos.xy - q.rect.xy * scale;
    let size = q.rect.zw * scale;
    let start_x = max(0., point.x - 0.5);
    let step = (min(size.x, point.x + 0.5) - start_x) * 0.25;
    if step <= 0. { discard; }

    let thickness = size.y / 3.;
    let amplitude = 0.8 * thickness;
    let frequency = 6.2831853 / (9. * thickness);
    var coverage = 0.;
    // Integrate the vertical stroke interval at four positions across the pixel.
    for (var i = 0u; i < 4u; i++) {
        let phase = (start_x + (f32(i) + 0.5) * step) * frequency;
        let center = 0.5 * size.y + amplitude * sin(phase);
        let slope = amplitude * frequency * cos(phase);
        let half = 0.5 * thickness * sqrt(1. + slope * slope);
        let top = max(point.y - 0.5, center - half);
        let bottom = min(point.y + 0.5, center + half);
        coverage += max(bottom - top, 0.) * step;
    }
    let ink = clamp(solor_paint(point / scale, q.rect.zw, q), vec4f(0.), vec4f(1.));
    return vec4f(ink.rgb, ink.a * coverage * solor_clip_coverage(v.pos.xy, v.clip));
}
