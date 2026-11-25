fn box_filter(p: vec2i, axis: vec2i, extent: i32, radius: f32) -> vec4f {
    let center = dot(p, axis);
    let whole = i32(floor(radius));
    let first = max(-whole, -center);
    let last = min(whole, extent - 1 - center);
    var total = vec4f(0.);
    for (var i = first; i <= last; i++) {
        total += read_sample(p + i * axis);
    }
    var weight = f32(last - first + 1);
    let fraction = fract(radius);
    if fraction > 0. {
        if center - whole > 0 {
            total += fraction * read_sample(p - (whole + 1) * axis);
            weight += fraction;
        }
        if center + whole + 1 < extent {
            total += fraction * read_sample(p + (whole + 1) * axis);
            weight += fraction;
        }
    }
    return total / weight;
}
