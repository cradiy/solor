// u runs clockwise along an edge and v runs inward from its outside boundary.
fn edge_uv(side: u32, p: vec2f, size: vec2f) -> vec2f {
    switch side {
        case 0u: { return p; }
        case 1u: { return vec2f(p.y, size.x - p.x); }
        case 2u: { return size - p; }
        default: { return vec2f(size.y - p.y, p.x); }
    }
}

fn nearest_mark(u: f32, marks: vec4f) -> f32 {
    let index = clamp(floor((u - marks.x) / marks.y + 0.5), 0., marks.z);
    return marks.x + index * marks.y;
}

fn circle_point(arc: f32, r: f32, half_width: f32) -> vec2f {
    let radius = r - half_width;
    let angle = arc / radius;
    return radius * vec2f(sin(angle), cos(angle));
}

fn curve_mark(uv: vec2f, length: f32, width: f32, radii: vec2f, marks: vec4f) -> bool {
    let half_width = width * 0.5;
    let tangent = length - radii.y;
    var along = uv.x;
    if radii.x > half_width && all(uv < vec2f(radii.x)) {
        along = radii.x - (radii.x - half_width) * atan2(radii.x - uv.x, radii.x - uv.y);
    } else if radii.y > half_width && uv.x > tangent && uv.y < radii.y {
        along = tangent + (radii.y - half_width) * atan2(uv.x - tangent, radii.y - uv.y);
    }
    let mark = nearest_mark(along, marks);
    if marks.w == 1. { return abs(along - mark) <= width; }
    var center = vec2f(mark, half_width);
    if radii.x > half_width && mark < radii.x {
        center = vec2f(radii.x) - circle_point(radii.x - mark, radii.x, half_width);
    } else if radii.y > half_width && mark > tangent {
        center = vec2f(tangent, radii.y) + vec2f(1., -1.) * circle_point(mark - tangent, radii.y, half_width);
    }
    let delta = uv - center;
    return dot(delta, delta) <= half_width * half_width;
}

fn curve_ink(side: u32, xs: vec2f, y: f32, size: vec2f, width: f32, radii: vec2f, marks: vec4f) -> f32 {
    if xs.x >= xs.y { return 0.; }
    let length = size[side % 2u];
    var hits = 0u;
    for (var i = 0u; i < 4u; i++) {
        let x = mix(xs.x, xs.y, (f32(i) + 0.5) * 0.25);
        hits += u32(curve_mark(edge_uv(side, vec2f(x, y), size), length, width, radii, marks));
    }
    return (xs.y - xs.x) * f32(hits) * 0.25;
}

// Integral over a clipped interval of equally spaced marks with half-width chord.
fn straight_ink(us: vec2f, chord: f32, marks: vec4f) -> f32 {
    if chord <= 0. { return 0.; }
    let start = marks.x - chord;
    let span = marks.z * marks.y + 2. * chord;
    let u = clamp(us - start, vec2f(0.), vec2f(span));
    if 2. * chord >= marks.y || marks.z == 0. { return u.y - u.x; }
    let count = floor(u / marks.y);
    let ink = count * (2. * chord) + min(u - count * marks.y, vec2f(2. * chord));
    return ink.y - ink.x;
}

fn pattern_ink(side: u32, xs: vec2f, y: f32, size: vec2f, width: f32, radii: vec2f, marks: vec4f) -> f32 {
    let span = xs.y - xs.x;
    if span <= 0. { return 0.; }
    if marks.w == 0. { return span; }
    if marks.y < 0.5 && marks.z >= 1. {
        return span * min(1., width / marks.y * select(2., 0.7853981634, marks.w == 2.));
    }
    let half_width = 0.5 * width;
    let length = size[side % 2u];
    let straight = vec2f(select(0., radii.x + half_width, radii.x > half_width),
                         length - select(0., radii.y + half_width, radii.y > half_width));
    let uv = edge_uv(side, vec2f(xs.x, y), size);
    if side % 2u == 0u && straight.x < straight.y {
        let limits = select(size.x - straight.yx, straight, side == 0u);
        let mid = clamp(limits, vec2f(xs.x), vec2f(xs.y));
        let us = select(size.x - mid.yx, mid, side == 0u);
        let dy = uv.y - half_width;
        let chord = select(width, sqrt(max(0., half_width * half_width - dy * dy)), marks.w == 2.);
        return curve_ink(side, vec2f(xs.x, mid.x), y, size, width, radii, marks)
             + straight_ink(us, chord, marks)
             + curve_ink(side, vec2f(mid.y, xs.y), y, size, width, radii, marks);
    }
    if side % 2u != 0u && uv.x >= straight.x && uv.x <= straight.y {
        let d = abs(uv.x - nearest_mark(uv.x, marks));
        if marks.w == 1. { return select(0., span, d <= width); }
        let chord = sqrt(max(0., half_width * half_width - d*d));
        let center = select(half_width, size.x - half_width, side == 1u);
        return max(0., min(xs.y, center + chord) - max(xs.x, center - chord));
    }
    return curve_ink(side, xs, y, size, width, radii, marks);
}

fn box_span(y: f32, box: vec4f, rx: vec4f, ry: vec4f) -> vec2f {
    if y < box.y || y >= box.w || box.x >= box.z { return vec2f(0.); }
    let top = y - box.y;
    let bottom = box.w - y;
    return vec2f(box.x + max(inset(rx.x, ry.x, top), inset(rx.w, ry.w, bottom)),
                 box.z - max(inset(rx.y, ry.y, top), inset(rx.z, ry.z, bottom)));
}

fn border_coverage(pixel: vec2f, size: vec2f, w: vec4f, r: vec4f, inner: vec4f, rx: vec4f, ry: vec4f, index: u32) -> vec4f {
    let tr = solor_stops[index];
    let bl = solor_stops[index + 1u];
    let marks = array(tr.color, tr.segment, bl.color, bl.segment);
    let cuts = vec4f(0., size.y, inner.y, inner.w);
    let split = size.x * w.w / max(w.y + w.w, 1e-30);
    var areas = vec4f(0.);
    var low = pixel.y;
    for (var quarter = 1u; quarter <= 4u; quarter++) {
        let end = pixel.y + f32(quarter) * 0.25;
        while low < end {
            let edges = select(vec4f(end), min(cuts, vec4f(end)), cuts > vec4f(low));
            let high = min(min(edges.x, edges.y), min(edges.z, edges.w));
            let y = (low + high) * 0.5;
            let weight = high - low;
            low = high;
            let outer = box_span(y, vec4f(0., 0., size), r, r);
            let left = max(outer.x, pixel.x);
            let right = min(outer.y, pixel.x + 1.);
            if left >= right { continue; }
            let upper = select(1e30, y / w.x, w.x > 0.);
            let lower = select(1e30, (size.y - y) / w.z, w.z > 0.);
            let depth = min(upper, lower);
            let a = clamp(min(w.w * depth, split), left, right);
            let b = clamp(max(size.x - w.y * depth, split), left, right);
            let hole = box_span(y, inner, rx, ry);
            let sides = array(3u, select(2u, 0u, upper <= lower), 1u);
            let points = vec4f(left, a, b, right);
            for (var i = 0u; i < 3u; i++) {
                let side = sides[i];
                let xs = vec2f(points[i], points[i+1u]);
                if xs.x >= xs.y || w[side] <= 0. { continue; }
                let radii = vec2f(r[side], r[(side + 1u) % 4u]);
                var ink: f32;
                if hole.x >= hole.y || xs.y <= hole.x || xs.x >= hole.y {
                    ink = pattern_ink(side, xs, y, size, w[side], radii, marks[side]);
                } else {
                    ink = pattern_ink(side, vec2f(xs.x, hole.x), y, size, w[side], radii, marks[side])
                        + pattern_ink(side, vec2f(hole.y, xs.y), y, size, w[side], radii, marks[side]);
                }
                areas[side] += weight * ink;
            }
        }
    }
    return areas;
}
