fn solor_box_coverage(local: vec2f, q: solor_Quad) -> f32 {
    let far = q.rect.zw - local;
    let corners = array(local,vec2f(far.x,local.y),far,vec2f(local.x,far.y));
    let half_pixel = 0.5 / q.viewport.z;
    var touches_corner = false;
    var dist = min(min(local.x,local.y), min(far.x,far.y));
    for (var i=0u; i<4u; i++) {
        let r = q.radii[i];
        let c = corners[i];
        if r > 0. && all(c - half_pixel < vec2f(r)) { touches_corner = true; }
        if all(c < vec2f(r)) { dist = min(dist, r - length(vec2f(r)-c)); }
    }
    let pixel_min = local * q.viewport.z - 0.5;
    let pixel_max = pixel_min + 1.0;
    let overlap = clamp(min(pixel_max, q.rect.zw * q.viewport.z) - max(pixel_min, vec2f(0.0)),
                        vec2f(0.0), vec2f(1.0));
    var coverage = overlap.x * overlap.y;
    if touches_corner {
        let distance_px = dist * q.viewport.z;
        if abs(distance_px) >= 0.71 {
            coverage = select(0., 1., distance_px > 0.);
        } else {
            var inside = 0u;
            for (var y = 0u; y < 4u; y++) {
                for (var x = 0u; x < 4u; x++) {
                    let delta = (vec2f(f32(x), f32(y)) * 0.25 - 0.375) / q.viewport.z;
                    let a = local + delta;
                    let b = far - delta;
                    var hit = all(min(a, b) >= vec2f(0.));
                    let distances = array(a, vec2f(b.x, a.y), b, vec2f(a.x, b.y));
                    for (var i = 0u; i < 4u; i++) {
                        let inset = max(vec2f(q.radii[i]) - distances[i], vec2f(0.));
                        hit = hit && dot(inset, inset) <= q.radii[i] * q.radii[i];
                    }
                    inside += u32(hit);
                }
            }
            coverage = f32(inside) / 16.;
        }
    }
    return coverage;
}
