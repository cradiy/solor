fn checker_integral(p: vec2f, cell: f32) -> vec2f {
    let period = 2. * cell;
    let phase = abs(p) % vec2f(period);
    return min(phase, period - phase);
}

// Area in a square of the given side where x + y falls below the cutoff.
fn area_below(cutoff: f32, side: f32) -> f32 {
    if cutoff <= side {
        let leg = max(cutoff, 0.);
        return 0.5 * leg * leg;
    }
    let leg = max(2. * side - cutoff, 0.);
    return side * side - 0.5 * leg * leg;
}

fn stripe_coverage(pixel: vec2f, band: vec4f) -> f32 {
    let side = band.z;
    var area = band.w;
    if side * side == 0. { return area; }
    let period = band.x + band.y;
    let phase = (pixel.x + pixel.y) % period;
    var start = -phase;
    if phase < 0. {
        area += area_below(start - band.y, side) - area_below(start - period, side);
    }
    // The residual square spans at most three bands.
    for (var i = 0u; i < 3u && start < 2. * side; i++) {
        area += area_below(start + band.x, side) - area_below(start, side);
        start += period;
    }
    return area;
}

fn solor_paint(local: vec2f, size: vec2f, q: solor_Quad) -> vec4f {
    if q.viewport.w >= 2. {
        let pixel = local * q.viewport.z - 0.5;
        var coverage: f32;
        if q.viewport.w == 2. {
            let cell = q.ramp.x;
            // Below this size the alternating area differs from half by less than one alpha level.
            coverage = 0.5;
            if cell > 0.001 {
                let delta = checker_integral(pixel + 1., cell) - checker_integral(pixel, cell);
                coverage -= 0.5 * delta.x * delta.y;
            }
        } else {
            coverage = stripe_coverage(pixel, q.ramp);
        }
        return vec4f(q.params.rgb, q.params.a * clamp(coverage, 0., 1.));
    }
    if q.ramp.y == 0. { return q.params; }
    return gradient_color(local, size, q.params, q.ramp, q.viewport.w != 0.);
}
