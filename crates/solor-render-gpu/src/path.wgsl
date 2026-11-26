fn solor_edge(base: u32, i: u32) -> vec4f {
    let pair = solor_stops[base + i / 2u];
    return select(pair.color, pair.segment, i % 2u != 0u);
}

fn solor_stroke(p: vec2f, base: u32, count: u32, radius: f32) -> f32 {
    var cover = 0.;
    for (var i = 0u; i < count; i++) {
        let e = solor_edge(base, i);
        let delta = e.zw - e.xy;
        let t = clamp(dot(p - e.xy, delta) / max(dot(delta, delta), 1e-30), 0., 1.);
        let d = length((p - e.xy) - t * delta);
        cover = max(cover, radius + 0.5 - d);
    }
    return clamp(cover, 0., min(1., 2. * radius));
}

// Each edge contributes a signed winding over a horizontal interval on each scanline.
// Integrate up to the next interval endpoint, snapped to 1/256 of a device pixel.
fn solor_boundary_coverage(p: vec2f, base: u32, count: u32, nonzero: bool) -> f32 {
    let ys = p.y + vec4f(-0.375, -0.125, 0.125, 0.375);
    let left = p.x - 0.5;
    var cursor = vec4f(0.);
    var covered = vec4f(0.);
    loop {
        var next = vec4f(256.);
        var winding = vec4i(0);
        for (var i = 0u; i < count; i++) {
            let e = solor_edge(base, i);
            let lo = min(e.xy, e.zw);
            let hi = max(e.xy, e.zw);
            if hi.x <= left || lo.x >= left + 1. || ys.w < lo.y || e.x == e.z { continue; }
            var start = vec4f(lo.x);
            var end = vec4f(hi.x);
            // The vertical ray crosses this edge only where the scanline is below it.
            if ys.x < hi.y {
                if e.y == e.w {
                    end = select(start, end, ys >= vec4f(e.y));
                } else {
                    let t = clamp((ys - e.y) / (e.w - e.y), vec4f(0.), vec4f(1.));
                    let cross = mix(vec4f(e.x), vec4f(e.z), t);
                    if (e.z > e.x) == (e.w > e.y) { end = cross; } else { start = cross; }
                }
            }
            start = floor(clamp(start - left, vec4f(0.), vec4f(1.)) * 256. + 0.5);
            end = floor(clamp(end - left, vec4f(0.), vec4f(1.)) * 256. + 0.5);
            let contributes = (start <= cursor) & (cursor < end);
            winding += select(vec4i(0), vec4i(select(-1, 1, e.x < e.z)), contributes);
            next = min(next, select(vec4f(256.), start, start > cursor));
            next = min(next, select(vec4f(256.), end, contributes));
        }
        let inside = select((winding & vec4i(1)) != vec4i(0), winding != vec4i(0), vec4<bool>(nonzero));
        covered += select(vec4f(0.), next - cursor, inside);
        cursor = next;
        if all(cursor == vec4f(256.)) { break; }
    }
    return floor(dot(covered, vec4f(1.)) * (255. / 1024.) + 0.5) / 255.;
}

fn solor_polygon(p: vec2f, base: u32, count: u32, nonzero: bool) -> f32 {
    var winding = 0;
    for (var i = 0u; i < count; i++) {
        let e = solor_edge(base, i);
        let lo = min(e.xy, e.zw);
        let hi = max(e.xy, e.zw);
        if hi.x <= p.x - 0.5 || lo.x >= p.x + 0.5 || lo.y > p.y + 0.375 { continue; }
        let delta = e.zw - e.xy;
        let side = (p.y - e.y) * delta.x - (p.x - e.x) * delta.y;
        // Only a segment touching the four scanlines inside this pixel can vary its winding.
        if hi.y >= p.y - 0.375 && abs(side) <= 0.5 * abs(delta.y) + 0.375 * abs(delta.x) {
            return solor_boundary_coverage(p, base, count, nonzero);
        }
        if lo.x <= p.x && p.x < hi.x && (side >= 0.) == (delta.x > 0.) {
            winding += select(-1, 1, delta.x > 0.);
        }
    }
    return f32(select((winding & 1) != 0, winding != 0, nonzero));
}

@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    if solor_outside_clip(v.pos.xy, q) { discard; }
    let base = u32(q.ramp.x);
    let count = u32(q.ramp.y);
    var cover: f32;
    if q.viewport.w == 2. {
        cover = solor_stroke(v.pos.xy, base, count, q.ramp.z);
    } else {
        cover = solor_polygon(v.pos.xy, base, count, q.viewport.w == 1.);
    }
    let color = solor_path_color(v.pos.xy, q);
    return vec4f(color.rgb, color.a * cover * solor_clip_coverage(v.pos.xy, v.clip));
}
