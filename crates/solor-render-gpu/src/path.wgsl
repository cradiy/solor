fn edge(base: u32, i: u32) -> vec4f {
    let pair = solor_stops[base + i / 2u];
    return select(pair.color, pair.segment, i % 2u != 0u);
}

fn stroke(p: vec2f, base: u32, count: u32, radius: f32) -> f32 {
    var cover = 0.;
    for (var i = 0u; i < count; i++) {
        let e = edge(base, i);
        let delta = e.zw - e.xy;
        let t = clamp(dot(p - e.xy, delta) / max(dot(delta, delta), 1e-30), 0., 1.);
        let d = length((p - e.xy) - t * delta);
        cover = max(cover, radius + 0.5 - d);
    }
    return clamp(cover, 0., min(1., 2. * radius));
}

fn polygon(p: vec2f, base: u32, count: u32, nonzero: bool) -> f32 {
    let offsets = vec4f(-0.375, -0.125, 0.125, 0.375);
    let ys = p.y + offsets;
    var covered = 0.;
    for (var col = 0u; col < 4u; col++) {
        let x = p.x + offsets[col];
        var winding = vec4i(0);
        for (var i = 0u; i < count; i++) {
            let e = edge(base, i);
            if x < min(e.x, e.z) || x >= max(e.x, e.z) { continue; }
            let y = mix(e.y, e.w, (x - e.x) / (e.z - e.x));
            let direction = select(-1, 1, e.x < e.z);
            winding += select(vec4i(0), vec4i(direction), ys >= vec4f(y));
        }
        let inside = select((winding & vec4i(1)) != vec4i(0), winding != vec4i(0), vec4<bool>(nonzero));
        covered += dot(select(vec4f(0), vec4f(1), inside), vec4f(1));
    }
    return covered / 16.;
}

@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    if solor_outside_clip(v.pos.xy, q) { discard; }
    let base = u32(q.ramp.x);
    let count = u32(q.ramp.y);
    var cover: f32;
    if q.viewport.w == 2. {
        cover = stroke(v.pos.xy, base, count, q.ramp.z);
    } else {
        cover = polygon(v.pos.xy, base, count, q.viewport.w == 1.);
    }
    return vec4f(q.params.rgb, q.params.a * cover);
}
