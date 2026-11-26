fn solor_rounded_coverage(local: vec2f, size: vec2f, radii: vec4f, scale: f32) -> f32 {
    let far = size - local;
    let corners = array(local,vec2f(far.x,local.y),far,vec2f(local.x,far.y));
    let half_pixel = 0.5 / scale;
    var touches_corner = false;
    var dist = min(min(local.x,local.y), min(far.x,far.y));
    for (var i=0u; i<4u; i++) {
        let r = radii[i];
        let c = corners[i];
        if r > 0. && all(c - half_pixel < vec2f(r)) { touches_corner = true; }
        if all(c < vec2f(r)) { dist = min(dist, r - length(vec2f(r)-c)); }
    }
    let pixel_min = local * scale - 0.5;
    let pixel_max = pixel_min + 1.0;
    let overlap = clamp(min(pixel_max, size * scale) - max(pixel_min, vec2f(0.0)),
                        vec2f(0.0), vec2f(1.0));
    var coverage = overlap.x * overlap.y;
    if touches_corner {
        let distance_px = dist * scale;
        if abs(distance_px) >= 0.71 {
            coverage = select(0., 1., distance_px > 0.);
        } else {
            var inside = 0u;
            for (var y = 0u; y < 4u; y++) {
                for (var x = 0u; x < 4u; x++) {
                    let delta = (vec2f(f32(x), f32(y)) * 0.25 - 0.375) / scale;
                    let a = local + delta;
                    let b = far - delta;
                    var hit = all(min(a, b) >= vec2f(0.));
                    let distances = array(a, vec2f(b.x, a.y), b, vec2f(a.x, b.y));
                    for (var i = 0u; i < 4u; i++) {
                        let inset = max(vec2f(radii[i]) - distances[i], vec2f(0.));
                        hit = hit && dot(inset, inset) <= radii[i] * radii[i];
                    }
                    inside += u32(hit);
                }
            }
            coverage = f32(inside) / 16.;
        }
    }
    return coverage;
}

fn solor_clip_coverage(point: vec2f, index: u32) -> f32 {
    if index == 0u { return 1.; }
    let end = bitcast<u32>(solor_stops[index].color.x);
    var coverage = 1.;
    for (var i = index + 1u; i < end; i++) {
        let clip = solor_stops[i];
        coverage = min(coverage, solor_rounded_coverage(point - clip.color.xy, clip.color.zw, clip.segment, 1.));
    }
    return coverage;
}

fn solor_box_coverage(local: vec2f, q: solor_Quad) -> f32 {
    return solor_rounded_coverage(local, q.rect.zw, q.radii, q.viewport.z);
}
