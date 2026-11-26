struct BoxShape { bounds: vec4f, radii: vec4f }

fn chord(box: BoxShape, y: f32) -> vec2f {
    let b = box.bounds;
    if y < b.y || y >= b.w || b.x >= b.z { return vec2f(0.); }
    let dy = vec4f(y - b.y, y - b.y, b.w - y, b.w - y);
    let d = max(box.radii - dy, vec4f(0.));
    let inset = box.radii - sqrt(max((box.radii - d) * (box.radii + d), vec4f(0.)));
    return vec2f(b.x + max(inset.x, inset.w), b.z - max(inset.y, inset.z));
}

fn overlap(a: vec2f, b: vec2f) -> f32 {
    return max(0., min(a.y, b.y) - max(a.x, b.x));
}

// Cumulative distribution of the sum of three unit-width box filters.
fn cdf(x: f32) -> f32 {
    let t = abs(x);
    let d = max(1.5 - t, 0.);
    let tail = select(d*d*d / 6., 0.5 - 0.75*t + t*t*t / 3., t < 0.5);
    return select(1. - tail, tail, x < 0.);
}

fn integrated(x: f32, blur: f32) -> f32 {
    let t = abs(x) / blur;
    let d = max(vec2f(1.5, 0.5) - t, vec2f(0.));
    let p = d*d*d*d;
    return max(x, 0.) + blur * (p.x - 3.*p.y) / 24.;
}

fn edge(d: f32, width: f32, blur: f32) -> f32 {
    if d >= 1.5 * blur { return width; }
    if d + width <= -1.5 * blur { return 0.; }
    if blur >= 4. * width { return width * cdf((d + 0.5 * width) / blur); }
    return integrated(d + width, blur) - integrated(d, blur);
}

fn horizontal(x: vec2f, span: vec2f, blur: f32) -> f32 {
    let width = x.y - x.x;
    if width <= 0. || span.x >= span.y { return 0.; }
    return edge(x.x - span.x, width, blur) - edge(x.x - span.y, width, blur);
}

// Integrate only the part of this pixel on which the shadow can be visible.
fn band_area(x: vec2f, ys: vec2f, base: vec2f, hard: vec2f, caster: BoxShape, blur: f32, inset: bool) -> f32 {
    let h = ys.y - ys.x;
    let inside = clamp(base, vec2f(x.x), vec2f(x.y));
    let inner = max(0., inside.y - inside.x);
    if (inset && inner == 0.) || (!inset && inner >= 1.) { return 0.; }
    if blur == 0. {
        return h * (select(overlap(x, hard), inner, inset) - overlap(inside, hard));
    }

    let top = max(caster.bounds.y, ys.x - 1.5 * blur);
    let bottom = min(caster.bounds.w, ys.y + 1.5 * blur);
    var ink = select(0., inner, inset);
    if bottom <= top || caster.bounds.z <= caster.bounds.x { return h * ink; }
    let rounded = top < caster.bounds.y + max(caster.radii.x, caster.radii.y) ||
                  bottom > caster.bounds.w - max(caster.radii.z, caster.radii.w);
    let steps = select(1u, 12u, rounded);
    let step = (bottom - top) / f32(steps);
    var previous = edge(top - ys.y, h, blur) / h;
    for (var i = 0u; i < steps; i++) {
        let span = chord(caster, top + (f32(i) + 0.5) * step);
        let next = edge(top + f32(i + 1u) * step - ys.y, h, blur) / h;
        var value: f32;
        if inset {
            value = -horizontal(inside, span, blur);
        } else if inner == 0. {
            value = horizontal(x, span, blur);
        } else {
            value = horizontal(vec2f(x.x, inside.x), span, blur) +
                    horizontal(vec2f(inside.y, x.y), span, blur);
        }
        ink += (next - previous) * value;
        previous = next;
    }
    return h * ink;
}

@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    if any(v.pos.xy < q.rect.xy) || any(v.pos.xy >= q.rect.xy + q.rect.zw) { discard; }
    let data = solor_stops[u32(q.ramp.x)];
    let caster = BoxShape(data.color, data.segment);
    let base = BoxShape(q.clip, q.radii);
    let blur = q.ramp.y;
    let inset = q.viewport.w != 0.;
    let p = floor(v.pos.xy);
    let x = vec2f(p.x, p.x + 1.);
    let interior = select(base.bounds, caster.bounds, inset);
    let radii = select(base.radii, caster.radii, inset);
    let margin = select(0., 1.5 * blur, inset);
    let guard = max(radii.xxyz, radii.wyzw) + margin;
    if all(p >= interior.xy + guard.xy) && all(p + 1. <= interior.zw - guard.zw) { discard; }

    // Split at box edges as well as quarter pixels, preserving fractional hard edges.
    let cuts = vec4f(base.bounds.y, base.bounds.w, caster.bounds.y, caster.bounds.w);
    var start = p.y;
    var y = p.y;
    var prev_base = vec2f(0.);
    var prev_hard = vec2f(0.);
    var area = 0.;
    for (var quarter = 1u; quarter <= 4u; quarter++) {
        let end = p.y + f32(quarter) * 0.25;
        while y < end {
            let edges = select(vec4f(end), min(cuts, vec4f(end)), cuts > vec4f(y));
            let next = min(min(edges.x, edges.y), min(edges.z, edges.w));
            let mid = (next + y) * 0.5;
            let silhouette = chord(base, mid);
            let hard = chord(caster, mid);
            if y > start && (any(silhouette != prev_base) || any(hard != prev_hard)) {
                area += band_area(x, vec2f(start, y), prev_base, prev_hard, caster, blur, inset);
                start = y;
            }
            prev_base = silhouette;
            prev_hard = hard;
            y = next;
        }
    }
    area += band_area(x, vec2f(start, p.y + 1.), prev_base, prev_hard, caster, blur, inset);
    return vec4f(q.params.rgb, q.params.a * clamp(area, 0., 1.) * solor_clip_coverage(v.pos.xy, v.clip));
}
