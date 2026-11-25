fn ramp_color(t: f32, first: u32, count: u32, repeating: bool) -> vec4f {
    var lo = 0u;
    var hi = count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2u;
        if stops[first + mid].segment.x <= t { lo = mid + 1u; } else { hi = mid; }
    }
    if !repeating {
        if lo == 0u { return stops[first].color; }
        let left = stops[first + lo - 1u];
        if lo == count || left.segment.x == t { return left.color; }
        let right = stops[first + lo];
        let f = (t - left.segment.x) / (right.segment.x - left.segment.x);
        return mix(left.color, right.color, pow(f, left.segment.y));
    }
    // Periodic cubic B-spline over the four neighboring control colors.
    let li = (lo + count - 1u) % count;
    let ri = lo % count;
    let left = stops[first + li];
    let right = stops[first + ri];
    let width = right.segment.x - left.segment.x + select(0., 1., ri == 0u);
    var f = 0.;
    if width > 0. {
        f = (t - left.segment.x + select(0., 1., lo == 0u)) / width;
    }
    let ff = f * f;
    let fff = ff * f;
    let inv = 1. - f;
    let before = stops[first + (li + count - 1u) % count].color;
    let after = stops[first + (ri + 1u) % count].color;
    return (before * (inv * inv * inv) + left.color * (3.*fff - 6.*ff + 4.) +
            right.color * (-3.*fff + 3.*ff + 3.*f + 1.) + after * fff) / 6.;
}

fn oklab_to_srgb(lab: vec3f) -> vec3f {
    let roots = vec3f(dot(lab, vec3f(1., 0.39633778, 0.21580376)),
                      dot(lab, vec3f(1., -0.105561346, -0.06385417)),
                      dot(lab, vec3f(1., -0.08948418, -1.2914855)));
    let lms = roots * roots * roots;
    let linear = clamp(vec3f(dot(lms, vec3f(4.0767417, -3.3077116, 0.23096994)),
                             dot(lms, vec3f(-1.268438, 2.6097574, -0.34131938)),
                             dot(lms, vec3f(-0.0041960863, -0.7034186, 1.7076147))),
                       vec3f(0.), vec3f(1.));
    return select(12.92 * linear, 1.055 * pow(linear, vec3f(1. / 2.4)) - 0.055,
                  linear > vec3f(0.0031308));
}

fn paint(local: vec2f, size: vec2f, q: Quad) -> vec4f {
    if q.ramp.y == 0. { return q.params; }
    let first = u32(q.ramp.x);
    let count = u32(q.ramp.y);
    let centered = local - size * 0.5;
    let unit = (2. * centered) / size;
    var t: f32;
    switch u32(q.params.w) {
        case 1u: { t = length(unit); }
        case 2u: {
            var angle = 0.;
            if any(unit != vec2f(0.)) { angle = atan2(unit.y, unit.x); }
            t = fract((angle - q.params.x) / 6.28318530718);
        }
        case 3u: {
            let axis = q.params.xy;
            t = abs(dot(unit, axis)) + abs(dot(unit, vec2f(-axis.y, axis.x)));
        }
        default: { t = 0.5 + dot(centered, q.params.xy); }
    }
    let phase = q.ramp.z;
    let half_seam = q.ramp.w * 0.5;
    var color: vec4f;
    if phase >= 0. {
        color = ramp_color(fract(fract(t) + phase), first, count, true);
    } else if q.params.w == 2. && half_seam > 0. && (t < half_seam || t > 1. - half_seam) {
        let wrapped = select(t - 1., t, t < half_seam);
        color = mix(ramp_color(1. - half_seam, first, count, false),
                    ramp_color(half_seam, first, count, false),
                    smoothstep(-half_seam, half_seam, wrapped));
    } else {
        color = ramp_color(clamp(t, 0., 1.), first, count, false);
    }
    var rgb = color.rgb / max(color.a, 1e-8);
    if q.viewport.w != 0. { rgb = oklab_to_srgb(rgb); }
    return vec4f(rgb, color.a * q.params.z);
}
